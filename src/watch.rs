//! React to tracked components and resources changing.
//!
//! Derive [`Track`] on a component or resource, then [`watch`] it from a
//! scene. The handler's first parameter says what to react to, written like
//! the system parameter you would use to read it:
//!
//! ```ignore
//! use bevy_radioactive::watch::*;
//!
//! #[derive(Component, Clone, Track)]
//! struct Health(u32);
//!
//! #[derive(Resource, Clone, Track)]
//! struct Score(u32);
//!
//! app.add_plugins(WatchPlugin);
//!
//! bsn! {
//!     Text("")
//!     // Any entity's `Health`, as `Query<&Health>` would see it.
//!     watch(|hp: Watch<Query<&Health>>, mut commands: Commands| {
//!         info!("{:?} has {} HP", hp.entity(), hp.0);
//!     })
//!     // Query filters narrow it down.
//!     watch(|hp: Watch<Query<&Health, With<Enemy>>>| { /* .. */ })
//!     // The resource.
//!     watch(|score: Watch<Res<Score>>, mut texts: Query<&mut Text>| {
//!         if let Ok(mut text) = texts.get_mut(score.target()) {
//!             **text = score.0.to_string();
//!         }
//!     })
//!     // Removal, or the entity despawning.
//!     watch(|gone: Removed<Query<&Health>>| { /* .. */ })
//!     // Write a component of this entity from the latest value.
//!     bind(|score: &Score, text: &mut Text| **text = score.0.to_string())
//! }
//! ```
//!
//! - [`Watch<Query<&T, F>>`](Watch) runs for every entity that `Query<&T, F>`
//!   would return when `T` is inserted or changed on it, so default query
//!   filters apply: disabled entities are skipped unless `F` allows them.
//!   [`Watch<Res<T>>`](Watch) runs when resource `T` is inserted or changed.
//! - [`Removed<..>`](Removed) takes the same selectors and runs when `T` is
//!   removed or its entity despawns.
//! - A watch declared in a scene despawns with its entity, and
//!   [`target()`](Watch::target) is that entity. [`WatchAppExt::watch`] adds
//!   one to the app instead.
//!
//! Insertion and mutable access count as changes; use `set_if_neq` to skip
//! no-op writes. Changes are published in [`WatchSystems`] in `PostUpdate`,
//! before UI layout, once per frame with the latest value. A handler that
//! writes another tracked value is published in the same frame, for up to 8
//! rounds.

use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    ops::Deref,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use bevy::{
    ecs::{
        component::Mutable,
        entity_disabling::Disabled,
        event::EntityEvent,
        lifecycle::HookContext,
        query::{Allow, QueryFilter},
        schedule::ScheduleLabel,
        system::{FromInput, IntoObserverSystem, SystemInput},
        template::{Template, TemplateContext},
        world::{CommandQueue, DeferredWorld},
    },
    platform::collections::HashSet,
    prelude::*,
    scene::EmptySceneEffect,
};

pub use crate::macros::Track;

/// Rounds of publishing per frame, so chained reactions settle in one frame.
const MAX_ROUNDS: usize = 8;

/// A component or resource that [`watch`] and [`bind`] react to.
///
/// Derive it next to `Component` or `Resource`; the type must be `Clone`,
/// since each change is delivered as a snapshot:
///
/// ```ignore
/// #[derive(Component, Clone, Track)]
/// struct Player { coords: Vec2 }
/// ```
pub trait Track: Component + Clone {}

/// Publishes tracked changes each frame. Required by [`watch`] and [`bind`].
#[derive(Default)]
pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Watched>()
            .init_resource::<Published>()
            .init_resource::<PendingBinds>()
            .init_schedule(Publish)
            .add_systems(PostUpdate, publish_all.in_set(WatchSystems));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            WatchSystems.before(bevy::ui::UiSystems::Prepare),
        );
    }
}

/// System set in `PostUpdate` that publishes tracked changes.
///
/// Order systems that write tracked values in `PostUpdate`
/// `.before(WatchSystems)` to have them shown in the same frame.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WatchSystems;

/// Watches from the app instead of a scene.
pub trait WatchAppExt {
    /// Runs `handler` like a scene's [`watch`], for as long as the app runs.
    /// Its [`target()`](Watch::target) is an entity holding the watch. Adds
    /// [`WatchPlugin`] if needed.
    ///
    /// ```ignore
    /// app.watch(|player: Watch<Query<&Player>>| info!("{:?} moved", player.entity()));
    /// ```
    fn watch<E, I, M>(&mut self, handler: I) -> &mut Self
    where
        E: WatchEvent,
        I: IntoObserverSystem<E, M>;
}

impl WatchAppExt for App {
    fn watch<E, I, M>(&mut self, handler: I) -> &mut Self
    where
        E: WatchEvent,
        I: IntoObserverSystem<E, M>,
    {
        if !self.is_plugin_added::<WatchPlugin>() {
            self.add_plugins(WatchPlugin);
        }
        let world = self.world_mut();
        let holder = world.spawn_empty().id();
        spawn_watcher::<E>(world, holder, None).observe(handler);
        self
    }
}

// ---------------------------------------------------------------------------
// Scene templates
// ---------------------------------------------------------------------------

/// Runs `handler` when a tracked value changes or is removed.
///
/// The handler is an observer whose first parameter selects what it reacts
/// to: [`Watch<Query<&T, F>>`](Watch), [`Watch<Res<T>>`](Watch),
/// [`Removed<Query<&T, F>>`](Removed) or [`Removed<Res<T>>`](Removed), where
/// `T` derives [`Track`]. Other parameters are ordinary system parameters.
/// A watch isn't sent current values when it spawns; it sees what is
/// inserted or changed afterwards, including by the same scene. The first
/// time a type is watched, every existing value counts as new.
pub fn watch<E, I, M>(handler: I) -> impl Template<Output = EmptySceneEffect>
where
    E: WatchEvent,
    I: IntoObserverSystem<E, M> + Clone,
    M: 'static,
{
    WatchTemplate {
        handler,
        marker: PhantomData::<fn() -> (E, M)>,
    }
}

/// Writes component `C` of the current entity from the latest `T`, on spawn
/// and whenever `T` changes.
///
/// It follows `T` on every entity `Query<&T>` returns, so it suits
/// resources and components that exist once, like a player:
///
/// ```ignore
/// Text("")
/// bind(|score: &Score, text: &mut Text| **text = score.0.to_string())
/// ```
pub fn bind<T, C, F>(write: F) -> impl Template<Output = EmptySceneEffect>
where
    T: Track,
    C: Component<Mutability = Mutable>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
{
    BindTemplate {
        write,
        marker: PhantomData::<fn() -> (T, C)>,
    }
}

struct WatchTemplate<E, I, M> {
    handler: I,
    marker: PhantomData<fn() -> (E, M)>,
}

impl<E, I, M> Template for WatchTemplate<E, I, M>
where
    E: WatchEvent,
    I: IntoObserverSystem<E, M> + Clone,
    M: 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let owner = context.entity.id();
        context.entity.world_scope(|world| {
            ensure_plugin::<E::Value>(world)?;
            spawn_watcher::<E>(world, owner, Some(owner)).observe(self.handler.clone());
            Ok(EmptySceneEffect)
        })
    }

    fn clone_template(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            marker: PhantomData,
        }
    }
}

struct BindTemplate<T, C, F> {
    write: F,
    marker: PhantomData<fn() -> (T, C)>,
}

impl<T, C, F> Template for BindTemplate<T, C, F>
where
    T: Track,
    C: Component<Mutability = Mutable>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let write = self.write.clone();
        watch(
            move |change: Watch<Query<&T>>, mut targets: Query<&mut C, Allow<Disabled>>| {
                if let Ok(mut target) = targets.get_mut(change.target()) {
                    write(&change, &mut target);
                }
            },
        )
        .build_template(context)?;
        // Write the current value once the scene, including `C`, exists.
        let (owner, write) = (context.entity.id(), self.write.clone());
        context.entity.world_scope(|world| {
            world
                .resource_mut::<PendingBinds>()
                .0
                .push(Box::new(move |world: &mut World| {
                    let Some(value) = world.query::<&T>().iter(world).last().cloned() else {
                        return;
                    };
                    if let Some(mut target) = world.get_mut::<C>(owner) {
                        write(&value, &mut target);
                    }
                }));
        });
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            write: self.write.clone(),
            marker: PhantomData,
        }
    }
}

fn ensure_plugin<T>(world: &World) -> Result {
    if world.contains_resource::<Watched>() {
        Ok(())
    } else {
        Err(format!(
            "watching `{}` needs `WatchPlugin`; add it to the app",
            type_name::<T>(),
        )
        .into())
    }
}

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

/// What a watch reacts to: `Query<&T, F>` or `Res<T>`, for a [`Track`] type
/// `T`. Used as the parameter of [`Watch`] and [`Removed`].
pub trait Selector {
    /// The tracked type.
    type Value: Track;
    #[doc(hidden)]
    type Key: SelectorKey<Value = Self::Value>;
    #[doc(hidden)]
    type At<'i>: Selector<Value = Self::Value, Key = Self::Key>;
}

/// `'static` stand-in for a [`Selector`], which borrows.
#[doc(hidden)]
pub trait SelectorKey: Send + Sync + 'static {
    type Value: Track;

    /// Tests whether a source entity is selected.
    fn matcher(world: &mut World) -> WatchMatcher;
}

#[doc(hidden)]
pub type WatchMatcher = Box<dyn FnMut(&World, Entity) -> bool + Send + Sync>;

#[doc(hidden)]
pub struct QueryKey<T, F>(PhantomData<fn() -> (T, F)>);

#[doc(hidden)]
pub struct ResKey<T>(PhantomData<fn() -> T>);

impl<T: Track, F: QueryFilter + 'static> Selector for Query<'_, '_, &T, F> {
    type Value = T;
    type Key = QueryKey<T, F>;
    type At<'i> = Query<'i, 'i, &'i T, F>;
}

impl<T: Track, F: QueryFilter + 'static> SelectorKey for QueryKey<T, F> {
    type Value = T;

    fn matcher(world: &mut World) -> WatchMatcher {
        // Exactly the entities `Query<&T, F>` returns, default filters included.
        let mut state = QueryState::<(), (With<T>, F)>::new(world);
        Box::new(move |world, entity| {
            state.update_archetypes(world);
            state.contains(
                entity,
                world,
                world.last_change_tick(),
                world.read_change_tick(),
            )
        })
    }
}

impl<T: Track + Resource> Selector for Res<'_, T> {
    type Value = T;
    type Key = ResKey<T>;
    type At<'i> = Res<'i, T>;
}

impl<T: Track + Resource> SelectorKey for ResKey<T> {
    type Value = T;

    fn matcher(_: &mut World) -> WatchMatcher {
        Box::new(|world, entity| world.resource_entity::<T>() == Some(entity))
    }
}

// ---------------------------------------------------------------------------
// Events and handler inputs
// ---------------------------------------------------------------------------

/// A change delivered to one watch. Use [`Watch`] in handlers.
#[doc(hidden)]
#[derive(EntityEvent)]
pub struct WatchChanged<K: SelectorKey> {
    #[event_target]
    delivery: Entity,
    target: Entity,
    source: Entity,
    value: Arc<K::Value>,
}

/// A removal delivered to one watch. Use [`Removed`] in handlers.
#[doc(hidden)]
#[derive(EntityEvent)]
pub struct WatchRemoved<K: SelectorKey> {
    #[event_target]
    delivery: Entity,
    target: Entity,
    source: Entity,
    marker: PhantomData<fn() -> K>,
}

/// The event behind a [`watch`] handler's first parameter.
#[doc(hidden)]
pub trait WatchEvent: EntityEvent + Sized {
    type Value: Track;
    type Key: SelectorKey<Value = Self::Value>;
    const REMOVAL: bool;

    fn send(commands: &mut Commands, delivery: WatchDelivery, value: Option<&Arc<Self::Value>>);
}

/// Where one delivery goes.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct WatchDelivery {
    watcher: Entity,
    target: Entity,
    source: Entity,
}

impl<K: SelectorKey> WatchEvent for WatchChanged<K> {
    type Value = K::Value;
    type Key = K;
    const REMOVAL: bool = false;

    fn send(commands: &mut Commands, delivery: WatchDelivery, value: Option<&Arc<K::Value>>) {
        if let Some(value) = value {
            commands.trigger(Self {
                delivery: delivery.watcher,
                target: delivery.target,
                source: delivery.source,
                value: Arc::clone(value),
            });
        }
    }
}

impl<K: SelectorKey> WatchEvent for WatchRemoved<K> {
    type Value = K::Value;
    type Key = K;
    const REMOVAL: bool = true;

    fn send(commands: &mut Commands, delivery: WatchDelivery, _: Option<&Arc<K::Value>>) {
        commands.trigger(Self {
            delivery: delivery.watcher,
            target: delivery.target,
            source: delivery.source,
            marker: PhantomData,
        });
    }
}

/// Handler input for [`watch`]: `T` was inserted or changed on an entity
/// that `S` selects. Dereferences to the new value.
///
/// `S` is `Query<&T>`, `Query<&T, F>` with any query filter, or `Res<T>`:
/// `|player: Watch<Query<&Player>>, mut commands: Commands| ..`.
pub struct Watch<'w, 't, S: Selector> {
    on: On<'w, 't, WatchChanged<S::Key>>,
    marker: PhantomData<fn() -> S>,
}

impl<S: Selector> Watch<'_, '_, S> {
    /// Entity whose value changed; for a resource, its singleton entity.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.on.event().source
    }

    /// Entity that declared the watch.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.on.event().target
    }
}

impl<S: Selector> Deref for Watch<'_, '_, S> {
    type Target = S::Value;

    fn deref(&self) -> &S::Value {
        &self.on.event().value
    }
}

/// Handler input for [`watch`]: `T` was removed from an entity that `S`
/// selects, or the entity despawned. `S` is as for [`Watch`].
pub struct Removed<'w, 't, S: Selector> {
    on: On<'w, 't, WatchRemoved<S::Key>>,
    marker: PhantomData<fn() -> S>,
}

impl<S: Selector> Removed<'_, '_, S> {
    /// Entity that lost the value.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.on.event().source
    }

    /// Entity that declared the watch.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.on.event().target
    }
}

macro_rules! handler_input {
    ($input:ident, $event:ident) => {
        impl<S: Selector> SystemInput for $input<'_, '_, S> {
            type Param<'i> = $input<'i, 'i, S::At<'i>>;
            type Inner<'i> = On<'i, 'i, $event<S::Key>>;

            fn wrap(on: Self::Inner<'_>) -> Self::Param<'_> {
                $input {
                    on,
                    marker: PhantomData,
                }
            }
        }

        impl<S: Selector> FromInput<On<'_, '_, $event<S::Key>>> for $input<'_, '_, S> {
            fn from_inner<'i>(
                on: <On<'_, '_, $event<S::Key>> as SystemInput>::Inner<'i>,
            ) -> Self::Inner<'i> {
                on
            }
        }
    };
}

handler_input!(Watch, WatchChanged);
handler_input!(Removed, WatchRemoved);

// ---------------------------------------------------------------------------
// Watchers
// ---------------------------------------------------------------------------

/// One watch: an entity observing its own deliveries.
#[derive(Component)]
#[component(on_add = add_watcher::<T>, on_remove = remove_watcher::<T>)]
struct Watcher<T: Track> {
    target: Entity,
    removal: bool,
    matcher: Mutex<WatchMatcher>,
    send: fn(&mut Commands, WatchDelivery, Option<&Arc<T>>),
}

/// Every [`Watcher<T>`] entity, so publishing doesn't need a query.
#[derive(Resource)]
struct WatcherList<T: Track>(Vec<Entity>, PhantomData<fn() -> T>);

fn add_watcher<T: Track>(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut list) = world.get_resource_mut::<WatcherList<T>>() {
        list.0.push(context.entity);
    }
}

fn remove_watcher<T: Track>(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut list) = world.get_resource_mut::<WatcherList<T>>() {
        list.0.retain(|&entity| entity != context.entity);
    }
}

/// Owns watcher entities, so they despawn with the entity that declared them.
#[derive(Component)]
#[relationship(relationship_target = Watches)]
struct WatchOf(Entity);

#[derive(Component)]
#[relationship_target(relationship = WatchOf, linked_spawn)]
struct Watches(Vec<Entity>);

/// Spawns the watcher entity for an `E` handler, delivering to `target`.
fn spawn_watcher<E: WatchEvent>(
    world: &mut World,
    target: Entity,
    owner: Option<Entity>,
) -> EntityWorldMut<'_> {
    track::<E::Value>(world);
    let matcher = <E::Key as SelectorKey>::matcher(world);
    let watcher = Watcher::<E::Value> {
        target,
        removal: E::REMOVAL,
        matcher: Mutex::new(matcher),
        send: E::send,
    };
    if let Some(owner) = owner {
        world.spawn((watcher, WatchOf(owner)))
    } else {
        let mut entity = world.entity_mut(target);
        entity.insert(watcher);
        entity
    }
}

/// Sends a change (`value`) or removal (`None`) of `T` on `source` to every
/// watcher whose selector matches it.
fn route<T: Track>(world: &World, commands: &mut Commands, source: Entity, value: Option<&Arc<T>>) {
    let Some(list) = world.get_resource::<WatcherList<T>>() else {
        return;
    };
    for &watcher in &list.0 {
        let Some(entry) = world.get::<Watcher<T>>(watcher) else {
            continue;
        };
        if entry.removal != value.is_none() {
            continue;
        }
        let selected =
            (entry.matcher.lock().unwrap_or_else(PoisonError::into_inner))(world, source);
        if selected {
            let delivery = WatchDelivery {
                watcher,
                target: entry.target,
                source,
            };
            (entry.send)(commands, delivery, value);
        }
    }
}

// ---------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Publish;

/// Tracked types, and publishers to add to [`Publish`] before the next round.
#[derive(Default, Resource)]
struct Watched {
    types: HashSet<TypeId>,
    pending: Vec<fn(&mut Schedule)>,
}

#[derive(Default, Resource)]
struct Published(AtomicUsize);

/// [`bind`]s waiting to write their current value.
#[derive(Default, Resource)]
struct PendingBinds(Vec<Box<dyn FnOnce(&mut World) + Send + Sync>>);

/// Starts publishing `T`. Safe while scenes spawn mid-round: the publisher
/// joins the schedule before the next round.
fn track<T: Track>(world: &mut World) {
    let mut watched = world.resource_mut::<Watched>();
    if !watched.types.insert(TypeId::of::<T>()) {
        return;
    }
    watched.pending.push(|schedule| {
        schedule.add_systems(publish::<T>);
    });
    world.insert_resource(WatcherList::<T>(Vec::new(), PhantomData));
    world.add_observer(publish_removed::<T>);
}

fn publish_all(world: &mut World) {
    for _ in 0..MAX_ROUNDS {
        let binds = std::mem::take(&mut world.resource_mut::<PendingBinds>().0);
        for bind in binds {
            bind(world);
        }
        let pending = std::mem::take(&mut world.resource_mut::<Watched>().pending);
        if !pending.is_empty() {
            world.schedule_scope(Publish, |_, schedule| {
                for add in pending {
                    add(schedule);
                }
            });
        }
        world.resource::<Published>().0.store(0, Ordering::Relaxed);
        let _ = world.try_run_schedule(Publish);
        world.flush();
        if world.resource::<Published>().0.load(Ordering::Relaxed) == 0
            && world.resource::<PendingBinds>().0.is_empty()
        {
            return;
        }
    }
    warn!(
        "watch: values still changing after {MAX_ROUNDS} rounds; the rest is published next \
         frame. A handler may be writing a value it watches."
    );
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are passed by value"
)]
fn publish<T: Track>(
    values: Query<(Entity, &T), (bevy::ecs::query::Changed<T>, Allow<Disabled>)>,
    published: Res<Published>,
    mut commands: Commands,
) {
    for (source, value) in &values {
        published.0.fetch_add(1, Ordering::Relaxed);
        let value = Arc::new(value.clone());
        commands.queue(move |world: &mut World| {
            let mut queue = CommandQueue::default();
            route(
                world,
                &mut Commands::new(&mut queue, world),
                source,
                Some(&value),
            );
            queue.apply(world);
        });
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn publish_removed<T: Track>(event: On<Remove<T>>, mut world: DeferredWorld) {
    // `Remove` observers run before the value goes, so selectors still see
    // the entity as it was.
    let mut queue = CommandQueue::default();
    route::<T>(
        &world,
        &mut Commands::new(&mut queue, &world),
        event.entity,
        None,
    );
    world.commands().append(&mut queue);
}
