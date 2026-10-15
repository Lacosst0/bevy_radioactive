//! Re-run a system whenever the components and resources it reads change.
//!
//! [`watch`] takes an ordinary Bevy system and attaches it to the current BSN
//! entity. The system runs once when the scene spawns, then again whenever
//! anything it reads changes: `Res<T>`, `&T` in a `Query`, and query filters
//! such as `With<T>`, `Without<T>` or `Changed<T>`. There are no sources to
//! name and nothing to register:
//!
//! ```ignore
//! use bevy_radioactive::watch::*;
//!
//! app.add_plugins(WatchPlugin);
//!
//! bsn! {
//!     Children [
//!         Player Health(100)
//!         --
//!         Text("")
//!         watch(|player: Query<&Health, With<Player>>, mut text: This<&mut Text>| {
//!             if let Some(mut text) = text.get_mut() {
//!                 **text = match player.single() {
//!                     Ok(hp) => hp.0.to_string(),
//!                     Err(_) => "DEAD".into(),
//!                 };
//!             }
//!         })
//!         --
//!         Text("")
//!         watch(|score: Res<Score>, mut text: This<&mut Text>| {
//!             if let Some(mut text) = text.get_mut() {
//!                 **text = score.0.to_string();
//!             }
//!         })
//!     ]
//! }
//! ```
//!
//! [`This`] queries the entity that declared the watch.
//!
//! Insertion, mutable access and removal (including despawn) count as
//! changes; use `set_if_neq` to skip no-op writes. Values a system only writes
//! (`ResMut<T>`, `&mut T`) are not dependencies, so a watch never re-runs
//! because of its own writes. Watches run in [`WatchSystems`] in `PostUpdate`,
//! before UI layout. A watch whose writes are read by another watch re-runs it
//! in the same frame, for up to 8 rounds.

use std::sync::Arc;

use bevy::{
    ecs::{
        change_detection::Tick,
        component::{ComponentId, IS_RESOURCE},
        error::ErrorContext,
        lifecycle::RemoveEvent,
        query::{FilteredAccess, QueryData, QueryFilter, ROQueryItem},
        system::{BoxedSystem, RunSystemError, SystemAccess, SystemParam},
        template::{Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::{HashMap, HashSet},
    prelude::*,
    scene::EmptySceneEffect,
};

/// Rounds of watches per frame, so chained watches settle in one frame.
const MAX_ROUNDS: usize = 8;

/// Runs watches each frame. Required by [`watch`].
#[derive(Default)]
pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WatchState>()
            .insert_resource(WatchTarget(Entity::PLACEHOLDER))
            .add_systems(PostUpdate, run_watches.in_set(WatchSystems));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            WatchSystems.before(bevy::ui::UiSystems::Prepare),
        );
    }
}

/// System set in `PostUpdate` that runs watches.
///
/// Order systems that write watched values in `PostUpdate`
/// `.before(WatchSystems)` to have them shown in the same frame.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WatchSystems;

/// Runs `system` when the scene spawns, and again whenever a component or
/// resource it reads changes.
///
/// Dependencies are taken from the system's parameters: everything it reads
/// (`Res<T>`, `&T`, `Option<&T>`, `Has<T>`, ..) and every query filter
/// (`With<T>`, `Without<T>`, `Changed<T>`, ..), minus anything it also writes.
/// Use [`This`] to access the entity that declared the watch.
///
/// Systems that read the whole world (`&World`, `EntityRef`) have no
/// dependencies to infer and are reported as a scene error.
pub fn watch<S, M>(system: S) -> impl Template<Output = EmptySceneEffect>
where
    S: IntoSystem<(), (), M> + Clone + Send + Sync + 'static,
    M: 'static,
{
    Watch {
        system,
        marker: std::marker::PhantomData::<fn() -> M>,
    }
}

/// A query for the entity that declared the running [`watch`].
///
/// ```ignore
/// Text("")
/// watch(|score: Res<Score>, mut text: This<&mut Text>| {
///     if let Some(mut text) = text.get_mut() {
///         **text = score.0.to_string();
///     }
/// })
/// ```
///
/// Only meaningful inside a [`watch`] system.
#[derive(SystemParam)]
pub struct This<'w, 's, D: QueryData + 'static, F: QueryFilter + 'static = ()> {
    query: Query<'w, 's, D, F>,
    target: Res<'w, WatchTarget>,
}

impl<'s, D: QueryData + 'static, F: QueryFilter + 'static> This<'_, 's, D, F> {
    /// The entity that declared the watch.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.target.0
    }

    /// The query item for this entity, if it matches.
    #[must_use]
    pub fn get(&self) -> Option<ROQueryItem<'_, 's, D>> {
        self.query.get(self.target.0).ok()
    }

    /// The mutable query item for this entity, if it matches.
    pub fn get_mut(&mut self) -> Option<D::Item<'_, 's>> {
        self.query.get_mut(self.target.0).ok()
    }
}

/// The entity whose watch is running. Read through [`This`].
#[doc(hidden)]
#[derive(Resource)]
pub struct WatchTarget(Entity);

// ---------------------------------------------------------------------------
// Watch template
// ---------------------------------------------------------------------------

struct Watch<S, M> {
    system: S,
    marker: std::marker::PhantomData<fn() -> M>,
}

impl<S, M> Template for Watch<S, M>
where
    S: IntoSystem<(), (), M> + Clone + Send + Sync + 'static,
    M: 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let listener = context.entity.id();
        context.entity.world_scope(|world| {
            if !world.contains_resource::<WatchState>() {
                return Err("`watch` needs `WatchPlugin`; add it to the app".to_string());
            }
            let mut system: BoxedSystem = Box::new(IntoSystem::into_system(self.system.clone()));
            let access = system.initialize(world);
            let deps = dependencies(world, &access).ok_or_else(|| {
                format!(
                    "`watch` can't tell what `{}` reads; use queries and resources instead of \
                     whole-world access",
                    system.name(),
                )
            })?;
            for &id in deps.iter() {
                observe_removal(world, id);
            }
            world.spawn((
                WatchOf(listener),
                Watcher {
                    system: Some(system),
                    deps,
                    fresh: true,
                },
            ));
            Ok(())
        })?;
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            system: self.system.clone(),
            marker: std::marker::PhantomData,
        }
    }
}

/// Components and resources the system reads or filters on, but doesn't
/// write. `None` if the access is unbounded.
fn dependencies(world: &mut World, access: &SystemAccess) -> Option<Arc<[ComponentId]>> {
    let set = match access {
        SystemAccess::None => return Some(Arc::new([])),
        SystemAccess::Shared(set) => set,
        SystemAccess::Exclusive => return None,
    };
    let combined = set.combined_access();
    let reads = combined.reads().as_finite_set()?;
    let writes = combined.writes().as_finite_set()?;
    let mut deps: HashSet<ComponentId> = reads.iter().collect();
    deps.extend(combined.archetypal().iter());
    for filters in set
        .filtered_accesses()
        .iter()
        .flat_map(FilteredAccess::filter_sets)
    {
        deps.extend(filters.with().iter());
        deps.extend(filters.without().iter());
    }
    deps.retain(|id| !writes.contains(*id));
    deps.remove(&IS_RESOURCE);
    deps.remove(&world.register_component::<WatchTarget>());
    Some(deps.into_iter().collect())
}

/// Marks `id` changed whenever it is removed from an entity.
fn observe_removal(world: &mut World, id: ComponentId) {
    if !world.resource_mut::<WatchState>().observed.insert(id) {
        return;
    }
    world.spawn(
        Observer::new(move |_: On<RemoveEvent>, mut world: DeferredWorld| {
            world.resource_mut::<WatchState>().removed.insert(id);
        })
        .with_component(id),
    );
}

/// Runs a watch system for the entity that declared it.
#[derive(Component)]
struct Watcher {
    /// Taken out while the system runs.
    system: Option<BoxedSystem>,
    deps: Arc<[ComponentId]>,
    /// Not run yet.
    fresh: bool,
}

/// Owns watchers, so they despawn with the entity that declared them.
#[derive(Component)]
#[relationship(relationship_target = Watchers)]
struct WatchOf(Entity);

#[derive(Component)]
#[relationship_target(relationship = WatchOf, linked_spawn)]
struct Watchers(Vec<Entity>);

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct WatchState {
    /// Changes up to this tick have been seen.
    checked: Tick,
    /// Watched components removed since the last round.
    removed: HashSet<ComponentId>,
    /// Components with a removal observer.
    observed: HashSet<ComponentId>,
}

impl FromWorld for WatchState {
    fn from_world(world: &mut World) -> Self {
        Self {
            checked: world.change_tick(),
            removed: HashSet::default(),
            observed: HashSet::default(),
        }
    }
}

fn run_watches(world: &mut World, watchers: &mut QueryState<(Entity, &Watcher, &WatchOf)>) {
    for _ in 0..MAX_ROUNDS {
        let now = world.change_tick();
        world.increment_change_tick();
        let mut state = world.resource_mut::<WatchState>();
        let last = std::mem::replace(&mut state.checked, now);
        let removed = std::mem::take(&mut state.removed);

        let mut changed: HashMap<ComponentId, bool> =
            removed.into_iter().map(|id| (id, true)).collect();
        let due: Vec<(Entity, Entity)> = watchers
            .iter(world)
            .filter(|(_, watcher, _)| {
                watcher.fresh
                    || watcher.deps.iter().any(|&id| {
                        *changed
                            .entry(id)
                            .or_insert_with(|| changed_since(world, id, last, now))
                    })
            })
            .map(|(runner, _, owner)| (runner, owner.0))
            .collect();
        if due.is_empty() {
            return;
        }
        for (runner, listener) in due {
            run_watcher(world, runner, listener);
        }
        world.flush();
    }
}

/// Whether any `id` value changed in `(last, now]`.
fn changed_since(world: &World, id: ComponentId, last: Tick, now: Tick) -> bool {
    let archetypes = world.archetypes();
    let Some(records) = archetypes.component_index().get(&id) else {
        return false;
    };
    records.keys().any(|&archetype| {
        archetypes[archetype].entities().iter().any(|entity| {
            world
                .entity(entity.id())
                .get_change_ticks_by_id(id)
                .is_some_and(|ticks| ticks.is_changed(last, now))
        })
    })
}

fn run_watcher(world: &mut World, runner: Entity, listener: Entity) {
    let Some(mut system) = world.get_mut::<Watcher>(runner).and_then(|mut watcher| {
        watcher.fresh = false;
        watcher.system.take()
    }) else {
        return;
    };
    world.resource_mut::<WatchTarget>().0 = listener;
    if let Err(RunSystemError::Failed(error)) = system.run((), world) {
        (world.fallback_error_handler())(
            error,
            ErrorContext::System {
                name: system.name(),
                last_run: system.get_last_run(),
            },
        );
    }
    if let Some(mut watcher) = world.get_mut::<Watcher>(runner) {
        watcher.system = Some(system);
    }
}
