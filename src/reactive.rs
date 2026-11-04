//! One reactive API for scenes and plain ECS code: watching, conditionals,
//! keyed lists, event handlers, context and cleanup, all built on components.
//!
//! There is no separate state type. State is a component or a resource,
//! context is an ancestor's component, a reactive branch is an entity, and
//! cleanup is despawning. Every reactive closure takes ordinary Bevy system
//! parameters; the parameter types say what it reads, what re-runs it and
//! what it writes.
//!
//! Scenes are usually written with [`rsn!`]: `bsn!` plus Rust control flow
//! and `$` reads. Whatever contains a `$` re-runs when what it read changes;
//! everything else runs once, when the scene spawns.
//!
//! | `$` read | Reads |
//! | --- | --- |
//! | `$Ref<Type>(#Name)` | component `Type` on the nearest entity named `#Name`: this entity and its subtree, then each ancestor and its subtree |
//! | `$Ref<Type>(entity)` | `Type` on that entity; skipped while it has none |
//! | `$Mut<Type>(#Name)`, `$Mut<Type>(entity)` | the same, writable; only in `on(..)` |
//! | `$Ref<Type>("Name")` | like `#Name`, for an entity named in another scene: not checked at compile time |
//!
//! A `#Name` must be declared in the same `rsn!`, or it is a compile error.
//! In debug builds, a closure whose `#Name` or `"Name"` matches nothing (or
//! whose entity lacks the component) logs one warning; it is skipped either
//! way.
//! | `$Res<T>`, `$ResMut<T>`, `$Local<T>`, `$Query<..>`, `$Single<..>`, .. | that system parameter, as in a Bevy system |
//!
//! ```ignore
//! use bevy::{prelude::*, ui_widgets::Activate};
//! use bevy_radioactive::reactive::*;
//!
//! #[derive(Component, Clone, Default)]
//! struct Count(i32);
//!
//! fn counter() -> impl Scene {
//!     rsn! {
//!         #Counter
//!         Node
//!         Count(0)                                         // state, local to this counter
//!         Children [
//!             Button
//!             on(|_: On<Activate>| $Mut<Count>(#Counter).0 += 1) // writes it
//!             --
//!             Text({ $Ref<Count>(#Counter).0.to_string() })       // re-renders when it changes
//!             --
//!             if $Ref<Count>(#Counter).0 > 10 { Text("That's a lot") } else { Text("Keep going") }
//!             --
//!             for todo in $Query<&Todo> { Text({ todo.title.clone() }) }
//!         ]
//!     }
//! }
//!
//! // app.add_plugins(ReactivePlugin)
//! ```
//!
//! # `rsn!`
//!
//! | In `rsn!` | Becomes |
//! | --- | --- |
//! | `Comp(.. $ ..)` | a [`watch`] that rebuilds `Comp` |
//! | `Comp { a: .. $ .., b: .. }` | `Comp { b: .. }` plus a [`watch`] that patches `a` |
//! | `Type::new(.. $ ..)`, `func(.. $ ..)`, `Comp(..).with(.. $ ..)` | a [`watch`] that rebuilds the component the expression returns |
//! | `Comp { a: Inner { x: .. $ .. } }` | patches only `a.x`, as `bsn!` does for nested values |
//! | `@scene(.. $ ..)`, `@Comp { @prop: .. $ .. }` | the entity in a [`switch`] region, rebuilt when an input changes |
//! | `@Comp { field: .. $ .. }`, `@Comp(.. $ ..)` | the scene once, plus a [`watch`] that patches `Comp` in place |
//! | `on(\|e: On<E>\| .. $ ..)` | a [`handle`]r; write with `$Mut<Type>(..)` or `$ResMut<T>` |
//! | `bind(.. $ ..)` | a [`bind`]ing: the widget shows the value and writes the user's edits back |
//! | `if`/`else if`/`else` | a [`switch`] region showing one branch |
//! | `match value { pat => .., }` | a [`switch`] region; arms are `pat => scene,` or `pat => { scene -- scene }` |
//! | `for pat in $Query<D, F> { .. }` | an [`each`] region with one row per entity |
//! | `#[key(expr)] for pat in iter { .. }` | an [`each`] region, keyed |
//! | `for pat in iter { .. }` | an [`each_indexed`] region |
//! | anything else | passed to `bsn!` unchanged |
//!
//! A `$Query` loop binds an owned copy of each item (`&T` becomes `T`, which
//! needs `Clone + PartialEq`) and rebuilds a row when it changes. Loop over
//! `$Query<Entity, With<T>>` and read `$Ref<T>(entity)` to update rows in place.
//! Other loops need owned items that implement `PartialEq`. Loop and match
//! bindings may be used any number of times. [`rsn_list!`] is the list form.
//!
//! A scene function runs once, so `$` in `@scene(..)` arguments or `@prop`s
//! rebuilds the entity when an input changes (inputs are cloned out of the
//! read and need `Clone + PartialEq`). A `SceneComponent` field is patched in
//! place instead, and its scene reads the field with `$Ref<Comp>(#Root)`: use
//! fields for values that change often and scenes that keep state.
//!
//! The functions it expands to are public and take closures with ordinary
//! Bevy system parameters; use them in `bsn!` or plain Rust:
//!
//! ```ignore
//! bsn! {
//!     Text("")
//!     watch(|enemies: Query<(), With<Enemy>>| Text(format!("{} enemies", enemies.iter().count())))
//!     --
//!     @when(|count: Ref<Count>| count.0 > 10, || bsn! { Text("That's a lot") })
//! }
//! ```
//!
//! # Two-way binding
//!
//! `bind(place)` in [`rsn!`] keeps a widget and a value in sync: the widget
//! shows the value and re-renders when it changes, and the user's edits are
//! written back. The place is read with `$Res`/`$Ref` and written with
//! `$ResMut`/`$Mut`, only when the edit differs from what is there.
//!
//! ```ignore
//! Slider SliderRange::new(0.0, 1.0) bind($Res<Settings>.volume)  // f32
//! --
//! Checkbox bind($Res<Settings>.muted)                            // bool
//! --
//! TextInput bind($Ref<Player>(#Player).name)                     // String
//! ```
//!
//! The widget is the entity's [`Bindable`] component for the value's type.
//! The `widgets` feature registers `bevy_ui_widgets`' `Slider`, `Checkbox`
//! and `TextInput`, which Feathers' controls build on; register your own
//! with [`BindableAppExt::register_bindable`]. Button clicks and other
//! events are handled with `on(|_: On<Activate>| ..)`.
//!
//! # Parameters
//!
//! | In a closure | Re-runs it? | Meaning |
//! | --- | --- | --- |
//! | `c: Ref<T>` | yes | the nearest component `T`: this entity, its [`Source`], then its ancestors |
//! | `c: Option<Ref<T>>` | yes | the same, `None` while there is none |
//! | `Res<T>`, `Option<Res<T>>` | yes | resource `T` |
//! | `Query<D, F>`, `Single<D, F>`, `Populated<D, F>` | yes | re-runs when a component it reads or filters on changes, is added or is removed on any entity |
//! | `c: &mut C` | no | the nearest `C`, found like `Ref<T>`, written in place |
//! | `e: Entity` | no | the entity that declared the closure |
//! | `ResMut`, `Commands`, `Local`, `MessageReader`, `MessageWriter` | no | ordinary system parameters |
//! | `Untracked<P>` | no | any system parameter, read without re-running |
//!
//! A run is skipped while a `Ref<T>` or `&mut C` it needs is missing. Query
//! parameters that write a component (`Query<&mut T>`) don't track that
//! component.
//!
//! # Building blocks
//!
//! | Item | Use it to |
//! | --- | --- |
//! | [`watch`] | re-run a closure on change; a returned component or bundle is inserted |
//! | [`handle`] | run a closure when an entity event reaches this entity |
//! | [`bind`] | keep a widget and a value in sync, both ways |
//! | `@`[`when`]`(cond, then)` | show entities while a condition holds |
//! | `@`[`if_else`]`(cond, then, otherwise)` | `if`/`else` |
//! | `@`[`switch`]`(key, render)` | show entities for the current value of a key (`match`) |
//! | `@`[`each`]`(items, key, row)` | show entities per item, keyed |
//! | `@`[`each_indexed`]`(items, row)` | show entities per item, by position |
//! | [`Source`] | read and write another entity's components from this subtree |
//! | [`Nearest`] | find the nearest component from ordinary systems and observers |
//!
//! [`watch`] and [`handle`] are templates: put them on an entity in `bsn!`,
//! or attach them in plain code with [`ReactExt::react`]. The control flow
//! functions are scenes, included with `@` like any scene function:
//! `Children [ @when(..) -- @each(..) ]`. The entity that includes one
//! becomes a [`Region`] (a `GhostNode` with the `ui` feature, so layout
//! ignores it) whose children are the current content. In plain code, spawn
//! them like any scene: `commands.spawn_scene(when(..))`, or
//! `queue_spawn_related_scenes::<Children>(bsn_list! { @when(..) })`.
//!
//! # Scope and cleanup
//!
//! Reactions are entities linked to the entity that declared them and
//! despawn with it. A region's content is its children, so replacing a
//! branch despawns everything in it, including the reactions and observers it
//! declared. Run cleanup code with Bevy's own `on(|_: On<Despawn>| ..)`.
//!
//! # Updates
//!
//! A reaction first runs right after its scene spawns, then in
//! [`ReactiveSystems`] in `PostUpdate` (before UI layout with the `ui`
//! feature) whenever its inputs changed. Outer regions update before the
//! reactions inside them. A reaction doesn't see its own writes; one that
//! changes another's input re-runs it in the same frame, for up to 16 rounds.

pub use crate::macros::{rsn, rsn_list};

use crate::shared::{self, despawn_all, lock};

use std::{
    any::TypeId,
    hash::Hash,
    marker::PhantomData,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use bevy::{
    ecs::{
        change_detection::Tick,
        component::{ComponentId, Mutable},
        entity_disabling::Disabled,
        event::EntityEvent,
        lifecycle::{Discard, Insert},
        message::Message,
        query::{Allow, IterQueryData, QueryData, QueryFilter},
        resource::IsResource,
        system::{SystemParam, SystemParamItem, SystemState},
        template::{FromTemplate, Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::HashMap,
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, SceneList},
};

/// Rounds of updates per frame, so chained reactions settle in one frame.
const MAX_ROUNDS: usize = 16;

/// Runs reactions. Required by everything in [this module](self).
#[derive(Default)]
pub struct ReactivePlugin;

impl Plugin for ReactivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnyChanges>()
            .init_resource::<Pending>()
            .init_resource::<Bindables>()
            .init_resource::<HierarchyChanged>()
            .add_observer(|_: On<Insert<(ChildOf, Source)>>, world: DeferredWorld| {
                note_hierarchy(world);
            })
            .add_observer(|_: On<Discard<(ChildOf, Source)>>, world: DeferredWorld| {
                note_hierarchy(world);
            })
            .add_systems(PostUpdate, run_reactions.in_set(ReactiveSystems));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            ReactiveSystems.before(bevy::ui::UiSystems::Prepare),
        );
        #[cfg(feature = "widgets")]
        widgets::register(app);
    }
}

/// System set in `PostUpdate` that runs reactions.
///
/// Order systems that write reactive inputs in `PostUpdate`
/// `.before(ReactiveSystems)` to have them shown in the same frame.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReactiveSystems;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Makes `Ref<T>` and `&mut C` parameters in this entity's subtree look at
/// another entity: on each step up the hierarchy, an entity's own components
/// come first, then those of its `Source`.
///
/// ```ignore
/// bsn! {
///     #Player Health(100)
///     --
///     Node
///     Source(#Player)
///     Children [
///         Text("") watch(|hp: Ref<Health>| Text(format!("HP {}", hp.0)))
///     ]
/// }
/// ```
#[derive(Component, FromTemplate, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source(pub Entity);

pub use crate::shared::Region;

/// Links a reaction entity to the entity that declared it.
#[derive(Component, Debug)]
#[relationship(relationship_target = Reactions)]
pub struct ReactionOf(pub Entity);

/// The reaction entities an entity declared. They despawn with it.
#[derive(Component, Debug)]
#[relationship_target(relationship = ReactionOf, linked_spawn)]
pub struct Reactions(Vec<Entity>);

/// One reaction: what it reads, when it last ran, and what it does.
#[derive(Component)]
struct Reaction {
    host: Entity,
    inputs: Vec<Input>,
    resolved: Vec<Option<Entity>>,
    /// What the last run read through `$Ref<T>(..)` in `rsn!`.
    reads: Vec<Read>,
    last_run: Option<Tick>,
    /// When `resolved` was last confirmed, which may be after `last_run`.
    /// Atomic so the check can update it through a shared reference.
    resolved_at: AtomicU32,
    body: Option<Box<dyn Body>>,
}

// ---------------------------------------------------------------------------
// Inputs and resolution
// ---------------------------------------------------------------------------

/// Where an input is looked up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lookup {
    /// This entity, its `Source`, then its ancestors.
    Nearest,
    /// The resource.
    Resource,
    /// Any entity.
    Any,
}

/// A value a reaction reads.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct Input {
    type_id: TypeId,
    id: ComponentId,
    lookup: Lookup,
    track: Track,
}

/// What change to an input re-runs its reaction.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Track {
    /// None: the input is only read.
    No,
    /// An entity gaining or losing the component, as for a `With` or
    /// `Without` filter.
    Presence,
    /// That, or a change to its value.
    Value,
}

impl Input {
    fn same_target(&self, other: &Self) -> bool {
        self.id == other.id && self.lookup == other.lookup
    }
}

/// One component read on a specific entity during a run, recorded while
/// the run happens instead of declared up front.
#[derive(Clone, Copy, Debug)]
enum Read {
    /// Component `id` on `entity`, which had it.
    At(Entity, ComponentId),
    /// Component `id` on an entity that didn't have it, or that couldn't
    /// be found: re-runs when any entity gains or loses it.
    Missing(ComponentId),
}

std::thread_local! {
    /// The reads of the reaction running on this thread, if any.
    static READS: std::cell::RefCell<Option<Vec<Read>>> = const { std::cell::RefCell::new(None) };
}

/// Records a read for the running reaction; does nothing outside one.
fn record(read: Read) {
    READS.with_borrow_mut(|reads| {
        if let Some(reads) = reads {
            reads.push(read);
        }
    });
}

/// Runs `f`, collecting the reads it records. Nested runs collect their
/// own.
fn collect_reads(f: impl FnOnce()) -> Vec<Read> {
    let outer = READS.replace(Some(Vec::new()));
    f();
    READS.replace(outer).unwrap_or_default()
}

/// Hierarchy walks stop after this many steps, in case of a `ChildOf` cycle.
const MAX_DEPTH: usize = 4096;

/// `start` and its ancestors, nearest first.
fn ancestors(
    start: Entity,
    parent: impl Fn(Entity) -> Option<Entity>,
) -> impl Iterator<Item = Entity> {
    std::iter::successors(Some(start), move |&entity| parent(entity)).take(MAX_DEPTH)
}

/// The first entity that `matches`, trying `start`, its [`Source`], then
/// the same for each ancestor. Shared by reactive closures and [`Nearest`].
fn find_nearest(
    start: Entity,
    links: impl Fn(Entity) -> (Option<Entity>, Option<Entity>),
    matches: impl Fn(Entity) -> bool,
) -> Option<Entity> {
    ancestors(start, |entity| links(entity).0).find_map(|entity| {
        if matches(entity) {
            return Some(entity);
        }
        links(entity).1.filter(|&source| matches(source))
    })
}

/// When an entity last got or lost a [`ChildOf`] or [`Source`]: until then,
/// `Ref<T>` lookups find what they found before, unless an entity gained or
/// lost `T`.
#[derive(Resource)]
struct HierarchyChanged(Tick);

impl FromWorld for HierarchyChanged {
    fn from_world(world: &mut World) -> Self {
        Self(world.change_tick())
    }
}

fn note_hierarchy(mut world: DeferredWorld) {
    let now = world.change_tick();
    world.resource_mut::<HierarchyChanged>().0 = now;
}

/// The entity holding component `id` for a reaction declared on `host`.
fn resolve(world: &World, host: Entity, id: ComponentId, lookup: Lookup) -> Option<Entity> {
    match lookup {
        Lookup::Any => None,
        Lookup::Resource => world.resource_entities().get(id),
        Lookup::Nearest => find_nearest(
            host,
            |entity| {
                (
                    world.get::<ChildOf>(entity).map(ChildOf::parent),
                    world.get::<Source>(entity).map(|source| source.0),
                )
            },
            |entity| {
                world
                    .get_entity(entity)
                    .is_ok_and(|entity| entity.contains_id(id))
            },
        ),
    }
}

fn resolve_all(world: &World, host: Entity, inputs: &[Input]) -> Vec<Option<Entity>> {
    inputs
        .iter()
        .map(|input| resolve(world, host, input.id, input.lookup))
        .collect()
}

/// What one run of a closure sees.
#[doc(hidden)]
pub struct Cx<'a> {
    host: Entity,
    inputs: &'a [Input],
    resolved: &'a [Option<Entity>],
    /// The event a [`handle`] closure runs for.
    event: Option<&'a (dyn std::any::Any + Send + Sync)>,
}

impl Cx<'_> {
    fn entity_of<T: 'static>(&self, lookup: Lookup) -> Option<Entity> {
        let type_id = TypeId::of::<T>();
        self.inputs
            .iter()
            .zip(self.resolved)
            .find_map(|(input, &entity)| {
                (input.type_id == type_id && input.lookup == lookup).then_some(entity)
            })
            .flatten()
    }
}

/// Last change seen per component tracked on any entity (by `Query`
/// parameters and missed `$Ref<T>(..)` reads).
#[derive(Resource, Default)]
struct AnyChanges(HashMap<ComponentId, AnyState>);

struct AnyState {
    /// Entities with the component.
    count: u64,
    /// The newest change to the component's value or presence.
    changed: Tick,
    /// The newest change to which entities have it.
    presence: Tick,
}

impl AnyState {
    fn new(world: &World, id: ComponentId) -> Self {
        let now = world.read_change_tick();
        Self {
            count: archetypes_with(world, id).map(|a| u64::from(a.len())).sum(),
            changed: now,
            presence: now,
        }
    }

    /// Catches up with changes since the last survey.
    fn survey(&mut self, world: &World, id: ComponentId, now: Tick) {
        let mut count = 0;
        let (mut changed, mut added) = (self.changed, self.presence);
        for archetype in archetypes_with(world, id) {
            count += u64::from(archetype.len());
            for entity in archetype.entities() {
                if let Some(ticks) = world
                    .get_entity(entity.id())
                    .ok()
                    .and_then(|entity| entity.get_change_ticks_by_id(id))
                {
                    if ticks.changed.is_newer_than(changed, now) {
                        changed = ticks.changed;
                    }
                    if ticks.added.is_newer_than(added, now) {
                        added = ticks.added;
                    }
                }
            }
        }
        // As many entities, unless one lost the component.
        if count != self.count {
            changed = now;
            added = now;
        }
        self.count = count;
        self.changed = changed;
        self.presence = added;
    }

    /// Whether a change `track` cares about happened after `since`.
    fn changed_since(&self, track: Track, since: Tick, now: Tick) -> bool {
        match track {
            Track::No => false,
            Track::Presence => self.presence.is_newer_than(since, now),
            Track::Value => self.changed.is_newer_than(since, now),
        }
    }
}

fn scan_any(world: &mut World) {
    world.resource_scope(|world, mut changes: Mut<AnyChanges>| {
        let now = world.change_tick();
        for (&id, state) in &mut changes.bypass_change_detection().0 {
            state.survey(world, id, now);
        }
    });
}

fn archetypes_with(
    world: &World,
    id: ComponentId,
) -> impl Iterator<Item = &bevy::ecs::archetype::Archetype> {
    world
        .archetypes()
        .iter()
        .filter(move |archetype| archetype.contains(id))
}

/// Starts surveying each of `ids` that isn't surveyed yet.
fn track_any(world: &mut World, ids: impl IntoIterator<Item = ComponentId>) {
    for id in ids {
        if world.resource::<AnyChanges>().0.contains_key(&id) {
            continue;
        }
        let state = AnyState::new(world, id);
        world
            .resource_mut::<AnyChanges>()
            .bypass_change_detection()
            .0
            .insert(id, state);
    }
}

impl Reaction {
    /// Whether a tracked input changed, or now resolves to another entity,
    /// since the last run.
    fn is_dirty(&self, world: &World, changes: &AnyChanges, hierarchy: Tick, now: Tick) -> bool {
        let Some(last_run) = self.last_run else {
            return true;
        };
        let any_changed = |id: ComponentId, track: Track| {
            changes
                .0
                .get(&id)
                .is_some_and(|state| state.changed_since(track, last_run, now))
        };
        let ticks = |entity: Entity, id: ComponentId| {
            world
                .get_entity(entity)
                .ok()
                .and_then(|entity| entity.get_change_ticks_by_id(id))
        };
        let read_changed = |read: &Read| match *read {
            Read::At(entity, id) => {
                ticks(entity, id).is_none_or(|ticks| ticks.is_changed(last_run, now))
            }
            Read::Missing(id) => any_changed(id, Track::Presence),
        };
        let resolved_at = Tick::new(self.resolved_at.load(Ordering::Relaxed));
        let dirty = self.reads.iter().any(read_changed)
            || self.inputs.iter().enumerate().any(|(index, input)| {
                if input.lookup == Lookup::Any {
                    return any_changed(input.id, input.track);
                }
                let cached = self.resolved.get(index).copied().flatten();
                // Walking the hierarchy is the costly part, so it is skipped
                // while nothing could have moved the input.
                let moved = input.lookup != Lookup::Nearest
                    || hierarchy.is_newer_than(resolved_at, now)
                    || changes
                        .0
                        .get(&input.id)
                        .is_none_or(|state| state.changed_since(Track::Presence, resolved_at, now))
                    || cached.is_some_and(|entity| ticks(entity, input.id).is_none());
                let entity = if moved {
                    resolve(world, self.host, input.id, input.lookup)
                } else {
                    cached
                };
                entity != cached
                    || (input.track != Track::No
                        && entity
                            .and_then(|entity| ticks(entity, input.id))
                            .is_some_and(|ticks| ticks.is_changed(last_run, now)))
            });
        if !dirty {
            // Every input resolves as before: no need to walk again until
            // something moves after `now`.
            self.resolved_at.store(now.get(), Ordering::Relaxed);
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// A parameter of a reactive closure: `Ref<T>`, `Option<Ref<T>>`, `&mut C`,
/// [`Entity`], [`Untracked`] and Bevy's system parameters.
pub trait ReactiveParam {
    #[doc(hidden)]
    type Param: SystemParam + 'static;
    /// This type, for any lifetimes.
    #[doc(hidden)]
    type Item<'w, 's>;
    #[doc(hidden)]
    fn inputs(_world: &mut World, _inputs: &mut Vec<Input>) {}
    #[doc(hidden)]
    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>>;
}

impl Input {
    fn new<T: Component>(world: &mut World, lookup: Lookup, track: Track) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            id: world.register_component::<T>(),
            lookup,
            track,
        }
    }
}

/// Every component a query reads or filters on, except those it writes.
/// Components it only filters on with `With` or `Without` re-run it only
/// when entities gain or lose them.
fn query_inputs<D: QueryData + 'static, F: QueryFilter + 'static>(
    world: &mut World,
    inputs: &mut Vec<Input>,
) {
    let state = QueryState::<D, F>::new(world);
    let access = state.component_access();
    let writes = access.access().writes();
    let is_resource = world.register_component::<IsResource>();
    let reads = access
        .access()
        .reads()
        .as_finite_set()
        .into_iter()
        .flat_map(bevy::ecs::query::ComponentIdSet::iter)
        .map(|id| (id, Track::Value));
    let filters = access
        .with_filters()
        .chain(access.without_filters())
        .map(|id| (id, Track::Presence));
    inputs.extend(
        reads
            .chain(filters)
            .filter(|&(id, _)| !writes.contains(id) && id != is_resource)
            .map(|(id, track)| Input {
                type_id: TypeId::of::<()>(),
                id,
                lookup: Lookup::Any,
                track,
            }),
    );
}

/// The entity a `Ref<T>` or `&mut C` parameter reads.
fn nearest_of<T: 'static>(cx: &Cx) -> Option<Entity> {
    cx.entity_of::<T>(Lookup::Nearest)
}

impl<T: Component> ReactiveParam for Ref<'_, T> {
    type Param = Query<'static, 'static, Ref<'static, T>, Allow<Disabled>>;
    type Item<'w, 's> = Ref<'w, T>;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(Input::new::<T>(world, Lookup::Nearest, Track::Value));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        param.get_inner(nearest_of::<T>(cx)?).ok()
    }
}

impl<T: Component> ReactiveParam for Option<Ref<'_, T>> {
    type Param = Query<'static, 'static, Ref<'static, T>, Allow<Disabled>>;
    type Item<'w, 's> = Option<Ref<'w, T>>;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(Input::new::<T>(world, Lookup::Nearest, Track::Value));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        Some(nearest_of::<T>(cx).and_then(|entity| param.get_inner(entity).ok()))
    }
}

impl<C: Component<Mutability = Mutable>> ReactiveParam for &mut C {
    type Param = Query<'static, 'static, &'static mut C, Allow<Disabled>>;
    type Item<'w, 's> = &'w mut C;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(Input::new::<C>(world, Lookup::Nearest, Track::No));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        param
            .get_inner(nearest_of::<C>(cx)?)
            .ok()
            .map(Mut::into_inner)
    }
}

impl ReactiveParam for Entity {
    type Param = ();
    type Item<'w, 's> = Entity;

    fn fetch<'w, 's>((): SystemParamItem<'w, 's, ()>, cx: &Cx) -> Option<Self::Item<'w, 's>> {
        Some(cx.host)
    }
}

/// Implements [`ReactiveParam`] for Bevy system parameters passed through
/// as they are, tracking what `inputs` adds (nothing when left out).
macro_rules! system_params {
    ($([$($generics:tt)*] $ty:ty => $param:ty, $item:ty $(, $inputs:expr)?;)*) => {$(
        impl<$($generics)*> ReactiveParam for $ty {
            type Param = $param;
            type Item<'w, 's> = $item;

            fn inputs(_world: &mut World, _inputs: &mut Vec<Input>) {
                $(($inputs)(_world, _inputs);)?
            }

            fn fetch<'w, 's>(
                param: SystemParamItem<'w, 's, Self::Param>,
                _: &Cx,
            ) -> Option<Self::Item<'w, 's>> {
                Some(param)
            }
        }
    )*};
}

system_params! {
    // Tracked.
    ['a, T: Resource] Res<'a, T> => Res<'static, T>, Res<'w, T>,
        |world, inputs: &mut Vec<Input>| inputs.push(Input::new::<T>(world, Lookup::Resource, Track::Value));
    ['a, T: Resource] Option<Res<'a, T>> => Option<Res<'static, T>>, Option<Res<'w, T>>,
        |world, inputs: &mut Vec<Input>| inputs.push(Input::new::<T>(world, Lookup::Resource, Track::Value));
    ['a, 'b, D: QueryData + 'static, F: QueryFilter + 'static] Query<'a, 'b, D, F>
        => Query<'static, 'static, D, F>, Query<'w, 's, D, F>, query_inputs::<D, F>;
    ['a, 'b, D: IterQueryData + 'static, F: QueryFilter + 'static] Single<'a, 'b, D, F>
        => Single<'static, 'static, D, F>, Single<'w, 's, D, F>, query_inputs::<D, F>;
    ['a, 'b, D: IterQueryData + 'static, F: QueryFilter + 'static] Option<Single<'a, 'b, D, F>>
        => Option<Single<'static, 'static, D, F>>, Option<Single<'w, 's, D, F>>, query_inputs::<D, F>;
    ['a, 'b, D: QueryData + 'static, F: QueryFilter + 'static] Populated<'a, 'b, D, F>
        => Populated<'static, 'static, D, F>, Populated<'w, 's, D, F>, query_inputs::<D, F>;
    // Untracked.
    ['a, T: Resource + Component<Mutability = Mutable>] ResMut<'a, T> => ResMut<'static, T>, ResMut<'w, T>;
    ['a, T: Resource + Component<Mutability = Mutable>] Option<ResMut<'a, T>> => Option<ResMut<'static, T>>, Option<ResMut<'w, T>>;
    ['a, 'b] Commands<'a, 'b> => Commands<'static, 'static>, Commands<'w, 's>;
    ['a, T: FromWorld + Send + 'static] Local<'a, T> => Local<'static, T>, Local<'s, T>;
    ['a, 'b, E: Message] MessageReader<'a, 'b, E> => MessageReader<'static, 'static, E>, MessageReader<'w, 's, E>;
    ['a, E: Message] MessageWriter<'a, E> => MessageWriter<'static, E>, MessageWriter<'w, E>;
}

/// Any system parameter, read without re-running the closure when it
/// changes: `|items: Untracked<Query<&Item>>| ..`. Also how a custom
/// [`SystemParam`] is passed.
pub struct Untracked<'w, 's, P: SystemParam + 'static>(pub SystemParamItem<'w, 's, P>);

impl<'w, 's, P: SystemParam + 'static> std::ops::Deref for Untracked<'w, 's, P> {
    type Target = SystemParamItem<'w, 's, P>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<P: SystemParam + 'static> std::ops::DerefMut for Untracked<'_, '_, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<P: SystemParam + 'static> ReactiveParam for Untracked<'_, '_, P> {
    type Param = P;
    type Item<'w, 's> = Untracked<'w, 's, P>;

    fn fetch<'w, 's>(param: SystemParamItem<'w, 's, P>, _: &Cx) -> Option<Self::Item<'w, 's>> {
        Some(Untracked(param))
    }
}

// ---------------------------------------------------------------------------
// Closures
// ---------------------------------------------------------------------------

/// A closure that computes a value from [`ReactiveParam`]s.
#[doc(hidden)]
pub trait Reader<V>: Send + Sync {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>);
    /// `None` when an input it needs is missing.
    fn read(&mut self, world: &mut World, cx: &Cx) -> Option<V>;
}

/// A closure whose parameters are [`ReactiveParam`]s: what [`watch`],
/// [`when`], [`switch`] and [`each`] take.
pub trait IntoReader<M>: Send + Sync + 'static {
    /// What the closure returns.
    type Output: 'static;
    #[doc(hidden)]
    fn into_reader(self) -> Box<dyn Reader<Self::Output>>;
}

/// A closure that [`handle`] runs: its first parameter is `&E`, the rest are
/// [`ReactiveParam`]s.
pub trait IntoHandler<E, M>: Send + Sync + 'static {
    #[doc(hidden)]
    fn into_handler(self) -> Box<dyn Reader<()>>;
}

/// A closure with the system state of its parameters. `M` tells reader
/// closures (`fn(P..) -> V`) from handlers (`Handles<fn(&E, P..)>`).
struct FnReader<W, M, P: SystemParam + 'static> {
    f: W,
    state: Option<SystemState<P>>,
    marker: PhantomData<fn() -> M>,
}

impl<W, M, P: SystemParam + 'static> FnReader<W, M, P> {
    fn boxed(f: W) -> Box<Self> {
        Box::new(Self {
            f,
            state: None,
            marker: PhantomData,
        })
    }
}

#[doc(hidden)]
pub struct Handles<F>(PhantomData<F>);

/// Implements [`IntoReader`] and [`IntoHandler`] for closures of `$param`s,
/// then recurses with one parameter fewer.
macro_rules! impl_closures {
    () => { impl_closures!(@impl); };
    ($head:ident $(, $tail:ident)*) => {
        impl_closures!(@impl $head $(, $tail)*);
        impl_closures!($($tail),*);
    };
    (@impl $($param:ident),*) => {
        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, V, $($param: ReactiveParam + 'static),*> IntoReader<fn($($param,)*) -> V> for W
        where
            W: Fn($($param),*) -> V + Fn($($param::Item<'_, '_>),*) -> V + Send + Sync + 'static,
            V: 'static,
        {
            type Output = V;

            fn into_reader(self) -> Box<dyn Reader<V>> {
                FnReader::<W, fn($($param,)*) -> V, ($($param::Param,)*)>::boxed(self)
            }
        }

        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, V, $($param: ReactiveParam + 'static),*> Reader<V>
            for FnReader<W, fn($($param,)*) -> V, ($($param::Param,)*)>
        where
            W: Fn($($param),*) -> V + Fn($($param::Item<'_, '_>),*) -> V + Send + Sync + 'static,
            V: 'static,
        {
            fn inputs(&self, _world: &mut World, _inputs: &mut Vec<Input>) {
                $($param::inputs(_world, _inputs);)*
            }

            fn read(&mut self, world: &mut World, _cx: &Cx) -> Option<V> {
                // Like Bevy's function systems: helps rustc see `W` is a function.
                #[allow(clippy::too_many_arguments, reason = "one per closure parameter")]
                fn call<V, $($param),*>(f: impl Fn($($param),*) -> V, $($param: $param),*) -> V {
                    f($($param),*)
                }
                let state = self.state.get_or_insert_with(|| SystemState::new(world));
                let output = {
                    let Ok(($($param,)*)) = state.get_mut(world) else {
                        return None;
                    };
                    $(let $param = $param::fetch($param, _cx)?;)*
                    call(&self.f, $($param),*)
                };
                state.apply(world);
                Some(output)
            }
        }

        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, E, $($param: ReactiveParam + 'static),*> IntoHandler<E, Handles<fn(&E, $($param,)*)>> for W
        where
            E: Send + Sync + 'static,
            W: Fn(&E, $($param),*) + Fn(&E, $($param::Item<'_, '_>),*) + Send + Sync + 'static,
        {
            fn into_handler(self) -> Box<dyn Reader<()>> {
                FnReader::<W, Handles<fn(&E, $($param,)*)>, ($($param::Param,)*)>::boxed(self)
            }
        }

        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, E, $($param: ReactiveParam + 'static),*> Reader<()>
            for FnReader<W, Handles<fn(&E, $($param,)*)>, ($($param::Param,)*)>
        where
            E: Send + Sync + 'static,
            W: Fn(&E, $($param),*) + Fn(&E, $($param::Item<'_, '_>),*) + Send + Sync + 'static,
        {
            fn inputs(&self, _world: &mut World, _inputs: &mut Vec<Input>) {
                $($param::inputs(_world, _inputs);)*
            }

            fn read(&mut self, world: &mut World, cx: &Cx) -> Option<()> {
                #[allow(clippy::too_many_arguments, reason = "one per closure parameter")]
                fn call<E, $($param),*>(f: impl Fn(&E, $($param),*), event: &E, $($param: $param),*) {
                    f(event, $($param),*);
                }
                let event = cx.event?.downcast_ref::<E>()?;
                let state = self.state.get_or_insert_with(|| SystemState::new(world));
                {
                    let Ok(($($param,)*)) = state.get_mut(world) else {
                        return None;
                    };
                    $(let $param = $param::fetch($param, cx)?;)*
                    call(&self.f, event, $($param),*);
                }
                state.apply(world);
                Some(())
            }
        }
    };
}

impl_closures!(P0, P1, P2, P3, P4, P5, P6, P7);

/// Makes fresh readers of one closure.
type ReaderFactory<V> = Arc<dyn Fn() -> Box<dyn Reader<V>> + Send + Sync>;

fn factory<M, F: IntoReader<M> + Clone>(f: F) -> ReaderFactory<F::Output> {
    Arc::new(move || f.clone().into_reader())
}

/// Content of a region branch or row.
type Render<V> = Arc<dyn Fn(&V) -> Box<dyn SceneList> + Send + Sync>;

fn boxed<L: SceneList>(list: L) -> Box<dyn SceneList> {
    Box::new(list)
}

// ---------------------------------------------------------------------------
// Reactions
// ---------------------------------------------------------------------------

/// What a reaction does when it runs.
trait Body: Send + Sync {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>);
    fn run(&mut self, world: &mut World, cx: &Cx);
}

/// Makes a reaction's body; cloned with its template.
type BodyFactory = Arc<dyn Fn() -> Box<dyn Body> + Send + Sync>;

/// Spawns a reaction for `host` and runs it once the current commands apply.
fn attach(world: &mut World, host: Entity, body: Box<dyn Body>) -> Result<Entity> {
    if !world.contains_resource::<AnyChanges>() {
        return Err("reactive closures need `ReactivePlugin`; add it to the app".into());
    }
    let mut inputs = Vec::new();
    body.inputs(world, &mut inputs);
    let mut unique: Vec<Input> = Vec::with_capacity(inputs.len());
    for input in inputs {
        if let Some(existing) = unique.iter_mut().find(|other| other.same_target(&input)) {
            existing.track = existing.track.max(input.track);
        } else {
            unique.push(input);
        }
    }
    track_any(
        world,
        unique
            .iter()
            .filter(|input| input.lookup != Lookup::Resource)
            .map(|input| input.id),
    );
    let reaction = world
        .spawn((
            Reaction {
                host,
                inputs: unique,
                resolved: Vec::new(),
                reads: Vec::new(),
                last_run: None,
                resolved_at: AtomicU32::new(0),
                body: Some(body),
            },
            ReactionOf(host),
        ))
        .id();
    // After the rest of the scene spawns, so ancestors and `Source`s exist.
    let mut pending = world.resource_mut::<Pending>();
    pending.reactions.push(reaction);
    // One queued run covers every reaction attached before it applies.
    if pending.reactions.len() == 1 {
        world.commands().queue(run_pending);
    }
    Ok(reaction)
}

/// Reactions waiting for their first run, held back while region content is
/// being attached to its parent.
#[derive(Resource, Default)]
struct Pending {
    reactions: Vec<Entity>,
    held: usize,
}

fn run_pending(world: &mut World) {
    if world.resource::<Pending>().held > 0 {
        return;
    }
    loop {
        let reactions = std::mem::take(&mut world.resource_mut::<Pending>().reactions);
        if reactions.is_empty() {
            return;
        }
        for reaction in reactions {
            run_reaction(world, reaction);
        }
    }
}

/// Runs a reaction now.
fn run_reaction(world: &mut World, reaction: Entity) {
    let Some(mut state) = world.get_mut::<Reaction>(reaction) else {
        return;
    };
    let state = state.bypass_change_detection();
    let host = state.host;
    let Some(mut body) = state.body.take() else {
        return;
    };
    let inputs = std::mem::take(&mut state.inputs);
    let resolved = resolve_all(world, host, &inputs);
    let reads = collect_reads(|| {
        if world.get_entity(host).is_ok() {
            body.run(
                world,
                &Cx {
                    host,
                    inputs: &inputs,
                    resolved: &resolved,
                    event: None,
                },
            );
        }
    });
    track_any(
        world,
        reads.iter().filter_map(|read| match *read {
            Read::Missing(id) => Some(id),
            Read::At(..) => None,
        }),
    );
    // Its own writes are older than `last_run`; later ones are newer.
    let last_run = world.increment_change_tick();
    if let Some(mut state) = world.get_mut::<Reaction>(reaction) {
        let state = state.bypass_change_detection();
        state.body = Some(body);
        state.inputs = inputs;
        state.resolved = resolved;
        state.reads = reads;
        state.last_run = Some(last_run);
        state.resolved_at = AtomicU32::new(last_run.get());
    }
    world.flush();
}

fn run_reactions(world: &mut World, reactions: &mut QueryState<(Entity, &Reaction)>) {
    shared::settle(
        world,
        MAX_ROUNDS,
        |world| run_round(world, reactions),
        "reactive: values",
    );
}

/// Runs every reaction whose inputs changed. Returns how many ran.
fn run_round(world: &mut World, reactions: &mut QueryState<(Entity, &Reaction)>) -> usize {
    world.flush();
    run_pending(world);
    scan_any(world);
    let now = world.change_tick();
    let mut due: Vec<(usize, Entity)> = {
        let world = &*world;
        let changes = world.resource::<AnyChanges>();
        let hierarchy = world.resource::<HierarchyChanged>().0;
        reactions
            .iter(world)
            .filter(|(_, reaction)| {
                reaction.body.is_some()
                    && !world
                        .get_entity(reaction.host)
                        .is_ok_and(|host| host.contains::<Disabled>())
                    && reaction.is_dirty(world, changes, hierarchy, now)
            })
            .map(|(entity, reaction)| {
                let depth = ancestors(reaction.host, |entity| {
                    world.get::<ChildOf>(entity).map(ChildOf::parent)
                })
                .count();
                (depth, entity)
            })
            .collect()
    };
    // Changes from here on are newer than the `now` checks were made at.
    world.increment_change_tick();
    // Outer regions first: they may despawn the reactions inside them.
    due.sort_unstable();
    for &(_, reaction) in &due {
        run_reaction(world, reaction);
    }
    due.len()
}

/// Something [`ReactExt::react`] can attach to an entity: [`watch`] or
/// [`handle`]. Both are also templates, for use in `bsn!`.
pub trait Reactive: Send + Sync + 'static {
    #[doc(hidden)]
    fn attach_to(&self, world: &mut World, host: Entity) -> Result<()>;
}

/// The [`Template::build_template`] of a [`Reactive`].
fn build_reactive(
    reactive: &impl Reactive,
    context: &mut TemplateContext,
) -> Result<EmptySceneEffect> {
    let host = context.entity.id();
    context
        .entity
        .world_scope(|world| reactive.attach_to(world, host))?;
    Ok(EmptySceneEffect)
}

/// Attaches [`watch`] and [`handle`] to entities outside of scenes.
///
/// ```ignore
/// commands
///     .spawn((Text::default(), Count(0)))
///     .react(watch(|count: Ref<Count>| Text(count.0.to_string())));
/// ```
pub trait ReactExt {
    /// Attaches `reactive` to this entity, as if declared on it in a scene.
    fn react(&mut self, reactive: impl Reactive) -> &mut Self;
}

impl ReactExt for EntityCommands<'_> {
    fn react(&mut self, reactive: impl Reactive) -> &mut Self {
        self.queue(move |mut entity: EntityWorldMut| {
            let host = entity.id();
            entity.world_scope(|world| reactive.attach_to(world, host))
        });
        self
    }
}

impl ReactExt for EntityWorldMut<'_> {
    fn react(&mut self, reactive: impl Reactive) -> &mut Self {
        let host = self.id();
        if let Err(error) = self.world_scope(|world| {
            let result = reactive.attach_to(world, host);
            world.flush();
            result
        }) {
            error!("react: {error}");
        }
        self
    }
}

// ---------------------------------------------------------------------------
// watch
// ---------------------------------------------------------------------------

/// Runs `f` when its entity spawns and again whenever a tracked parameter
/// changes. A returned component or bundle is inserted on the entity.
///
/// ```ignore
/// Text("0")
/// watch(|count: Ref<Count>| Text(count.0.to_string()))
/// --
/// Node
/// watch(|count: Ref<Count>, node: &mut Node| {
///     node.display = if count.0 != 0 { Display::Flex } else { Display::None };
/// })
/// --
/// Text("")
/// watch(|enemies: Query<(), With<Enemy>>| Text(format!("{} left", enemies.iter().count())))
/// ```
///
/// A returned component is a derived value: descendants can read it as
/// `Ref<T>` like any other component.
pub fn watch<M, F>(f: F) -> Watch<F, M>
where
    F: IntoReader<M> + Clone,
    F::Output: Bundle,
{
    Watch(f, PhantomData)
}

/// Made by [`watch`].
pub struct Watch<F, M>(F, PhantomData<fn() -> M>);

struct WatchBody<R>(Box<dyn Reader<R>>);

impl<R: Bundle> Body for WatchBody<R> {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
        self.0.inputs(world, inputs);
    }

    fn run(&mut self, world: &mut World, cx: &Cx) {
        let Some(output) = self.0.read(world, cx) else {
            return;
        };
        if TypeId::of::<R>() != TypeId::of::<()>()
            && let Ok(mut host) = world.get_entity_mut(cx.host)
        {
            host.insert(output);
        }
    }
}

impl<M: 'static, F> Reactive for Watch<F, M>
where
    F: IntoReader<M> + Clone,
    F::Output: Bundle,
{
    fn attach_to(&self, world: &mut World, host: Entity) -> Result<()> {
        let body = Box::new(WatchBody(self.0.clone().into_reader()));
        attach(world, host, body).map(|_| ())
    }
}

impl<M: 'static, F> Template for Watch<F, M>
where
    F: IntoReader<M> + Clone,
    F::Output: Bundle,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        build_reactive(self, context)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

// ---------------------------------------------------------------------------
// handle
// ---------------------------------------------------------------------------

/// Runs `f` whenever entity event `E` reaches this entity, including events
/// that bubble up from its descendants. The other parameters work as in
/// [`watch`], so `&mut C` writes the nearest `C`: this entity's, its
/// [`Source`]'s or an ancestor's.
///
/// ```ignore
/// Button
/// handle(|_: &Activate, count: &mut Count| count.0 += 1)
/// ```
pub fn handle<E, M, F>(f: F) -> Handler<E, F, M>
where
    E: EntityEvent + Clone,
    F: IntoHandler<E, M> + Clone,
{
    Handler(f, PhantomData)
}

/// Made by [`handle`].
pub struct Handler<E, F, M>(F, PhantomData<fn() -> (E, M)>);

impl<E, F, M: 'static> Reactive for Handler<E, F, M>
where
    E: EntityEvent + Clone,
    F: IntoHandler<E, M> + Clone,
{
    fn attach_to(&self, world: &mut World, host: Entity) -> Result<()> {
        let handler = handler_state(self.0.clone().into_handler());
        let observer = move |event: On<E>, mut commands: Commands| {
            let event = event.event().clone();
            let handler = Arc::clone(&handler);
            // Exclusive world access, so handlers can take any parameter.
            commands.queue(move |world: &mut World| run_handler(world, host, &handler, &event));
        };
        world
            .get_entity_mut(host)
            .map_err(|_| "react: the entity doesn't exist")?
            .observe(observer);
        Ok(())
    }
}

/// A handler closure and its inputs, found on its first run.
type HandlerState = Arc<Mutex<(Box<dyn Reader<()>>, Option<Vec<Input>>)>>;

fn handler_state(reader: Box<dyn Reader<()>>) -> HandlerState {
    Arc::new(Mutex::new((reader, None)))
}

/// Runs a [`handle`] or [`bind`] closure for `event`, declared on `host`.
fn run_handler(
    world: &mut World,
    host: Entity,
    handler: &HandlerState,
    event: &(dyn std::any::Any + Send + Sync),
) {
    let mut handler = lock(handler);
    let (reader, inputs) = &mut *handler;
    let inputs: &mut Vec<Input> = inputs.get_or_insert_with(|| {
        let mut inputs = Vec::new();
        reader.inputs(world, &mut inputs);
        inputs
    });
    let resolved = resolve_all(world, host, inputs);
    reader.read(
        world,
        &Cx {
            host,
            inputs,
            resolved: &resolved,
            event: Some(event),
        },
    );
}

impl<E, F, M: 'static> Template for Handler<E, F, M>
where
    E: EntityEvent + Clone,
    F: IntoHandler<E, M> + Clone,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        build_reactive(self, context)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

// ---------------------------------------------------------------------------
// bind
// ---------------------------------------------------------------------------

/// Keeps a widget and a value in sync, both ways: the widget shows what `get`
/// returns, re-run like [`watch`], and when the user edits the widget, `set`
/// runs with the new value, like a [`handle`]r.
///
/// The widget is the entity's [`Bindable`] component for the value's type:
/// with the `widgets` feature, a `Slider` edits an `f32`, a `Checkbox` a
/// `bool` and a `TextInput` a `String`. Register others with
/// [`BindableAppExt::register_bindable`].
///
/// ```ignore
/// Slider SliderRange::new(0.0, 1.0)
/// bind(
///     |volume: Res<Volume>| volume.0,
///     |value: &f32, mut volume: ResMut<Volume>| volume.0 = *value,
/// )
/// ```
///
/// In [`rsn!`], write the value once: `bind($Res<Volume>.0)` reads it with
/// `$Res`/`$Ref` and writes it with `$ResMut`/`$Mut`, only when it differs.
pub fn bind<M, N, G, S>(get: G, set: S) -> Bind<G, S, M, N>
where
    G: IntoReader<M> + Clone,
    G::Output: Clone + PartialEq + Send + Sync,
    S: IntoHandler<G::Output, N> + Clone,
{
    Bind(get, set, PhantomData)
}

/// Made by [`bind`].
pub struct Bind<G, S, M, N>(G, S, PhantomData<fn() -> (M, N)>);

/// A widget component that edits a value: what [`bind`] syncs.
///
/// ```ignore
/// impl Bindable for Knob {
///     type Value = f32;
///     type Event = ValueChange<f32>;
///
///     fn show(entity: &mut EntityWorldMut, value: &f32) {
///         entity.insert(KnobAngle(*value));
///     }
///
///     fn value(event: &ValueChange<f32>, _: EntityRef) -> Option<f32> {
///         Some(event.value)
///     }
/// }
///
/// // app.register_bindable::<Knob>()
/// ```
pub trait Bindable: Component {
    /// The value the widget edits.
    type Value: Clone + PartialEq + Send + Sync + 'static;
    /// The event the widget sends when the user edits it.
    type Event: EntityEvent;

    /// Shows `value` in the widget. Called when the bound value changes, so
    /// skip work when the widget already shows it.
    fn show(entity: &mut EntityWorldMut, value: &Self::Value);

    /// The edited value an `event` on `entity` carries, if any.
    fn value(event: &Self::Event, entity: EntityRef) -> Option<Self::Value>;
}

/// Registers [`Bindable`] widgets for [`bind`].
pub trait BindableAppExt {
    /// Lets [`bind`] sync `W` with values of type `W::Value`. Widgets
    /// registered later are tried first.
    fn register_bindable<W: Bindable>(&mut self) -> &mut Self;
}

impl BindableAppExt for App {
    fn register_bindable<W: Bindable>(&mut self) -> &mut Self {
        let marker = self.world_mut().register_component::<W>();
        self.world_mut()
            .get_resource_or_init::<Bindables>()
            .0
            .push(Widget {
                marker,
                value: TypeId::of::<W::Value>(),
                name: std::any::type_name::<W>(),
                show: show_widget::<W>,
                observe: observe_widget::<W>,
            });
        self
    }
}

/// The registered [`Bindable`]s.
#[derive(Resource, Default)]
struct Bindables(Vec<Widget>);

/// A [`Bindable`], type-erased.
#[derive(Clone, Copy)]
struct Widget {
    marker: ComponentId,
    value: TypeId,
    name: &'static str,
    show: fn(&mut World, Entity, &(dyn std::any::Any + Send + Sync)),
    observe: fn(&mut World, Entity, HandlerState),
}

fn show_widget<W: Bindable>(
    world: &mut World,
    host: Entity,
    value: &(dyn std::any::Any + Send + Sync),
) {
    if let Some(value) = value.downcast_ref::<W::Value>()
        && let Ok(mut entity) = world.get_entity_mut(host)
    {
        W::show(&mut entity, value);
    }
}

fn observe_widget<W: Bindable>(world: &mut World, host: Entity, set: HandlerState) {
    let observer = move |event: On<W::Event>, mut world: DeferredWorld| {
        if event.event_target() != host {
            return;
        }
        let Ok(entity) = world.get_entity(host) else {
            return;
        };
        let Some(value) = W::value(event.event(), entity) else {
            return;
        };
        let set = Arc::clone(&set);
        world
            .commands()
            .queue(move |world: &mut World| run_handler(world, host, &set, &value));
    };
    if let Ok(mut entity) = world.get_entity_mut(host) {
        entity.observe(observer);
    }
}

struct BindBody<V> {
    get: Box<dyn Reader<V>>,
    set: HandlerState,
    /// The widget, found on the first run.
    widget: Option<Widget>,
    looked_up: bool,
}

impl<V: Clone + PartialEq + Send + Sync + 'static> BindBody<V> {
    fn widget(&mut self, world: &mut World, host: Entity) -> Option<Widget> {
        if self.looked_up {
            return self.widget;
        }
        let found = world.get_resource::<Bindables>().and_then(|bindables| {
            let entity = world.get_entity(host).ok()?;
            bindables
                .0
                .iter()
                .rev()
                .find(|widget| {
                    widget.value == TypeId::of::<V>() && entity.contains_id(widget.marker)
                })
                .copied()
        });
        if let Some(widget) = found {
            (widget.observe)(world, host, Arc::clone(&self.set));
        } else {
            let registered: Vec<&str> = world
                .get_resource::<Bindables>()
                .map(|bindables| {
                    bindables
                        .0
                        .iter()
                        .filter(|widget| widget.value == TypeId::of::<V>())
                        .map(|widget| widget.name)
                        .collect()
                })
                .unwrap_or_default();
            warn!(
                "bind: entity {host} has no widget that edits `{}` values; add one of [{}], \
                 or register yours with `app.register_bindable::<W>()`",
                std::any::type_name::<V>(),
                registered.join(", "),
            );
        }
        self.widget = found;
        self.looked_up = true;
        found
    }
}

impl<V: Clone + PartialEq + Send + Sync + 'static> Body for BindBody<V> {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
        self.get.inputs(world, inputs);
    }

    fn run(&mut self, world: &mut World, cx: &Cx) {
        let Some(value) = self.get.read(world, cx) else {
            return;
        };
        if let Some(widget) = self.widget(world, cx.host) {
            (widget.show)(world, cx.host, &value);
        }
    }
}

impl<G, S, M: 'static, N: 'static> Reactive for Bind<G, S, M, N>
where
    G: IntoReader<M> + Clone,
    G::Output: Clone + PartialEq + Send + Sync,
    S: IntoHandler<G::Output, N> + Clone,
{
    fn attach_to(&self, world: &mut World, host: Entity) -> Result<()> {
        let body = Box::new(BindBody {
            get: self.0.clone().into_reader(),
            set: handler_state(self.1.clone().into_handler()),
            widget: None,
            looked_up: false,
        });
        attach(world, host, body).map(|_| ())
    }
}

impl<G, S, M: 'static, N: 'static> Template for Bind<G, S, M, N>
where
    G: IntoReader<M> + Clone,
    G::Output: Clone + PartialEq + Send + Sync,
    S: IntoHandler<G::Output, N> + Clone,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        build_reactive(self, context)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone(), self.1.clone(), PhantomData)
    }
}

/// [`Bindable`] for the `bevy_ui_widgets` widgets.
#[cfg(feature = "widgets")]
mod widgets {
    use super::Bindable;
    use bevy::{
        prelude::*,
        text::{EditableText, TextEdit, TextEditChange},
        ui::Checked,
        ui_widgets::{Checkbox, Slider, SliderValue, TextInput, ValueChange},
    };

    impl Bindable for Slider {
        type Value = f32;
        type Event = ValueChange<f32>;

        #[allow(clippy::float_cmp, reason = "skips only an exact repeat")]
        fn show(entity: &mut EntityWorldMut, &value: &f32) {
            if entity
                .get::<SliderValue>()
                .is_none_or(|shown| shown.0 != value)
            {
                entity.insert(SliderValue(value));
            }
        }

        fn value(event: &ValueChange<f32>, _: EntityRef) -> Option<f32> {
            Some(event.value)
        }
    }

    impl Bindable for Checkbox {
        type Value = bool;
        type Event = ValueChange<bool>;

        fn show(entity: &mut EntityWorldMut, &checked: &bool) {
            match (checked, entity.contains::<Checked>()) {
                (true, false) => {
                    entity.insert(Checked);
                }
                (false, true) => {
                    entity.remove::<Checked>();
                }
                _ => {}
            }
        }

        fn value(event: &ValueChange<bool>, _: EntityRef) -> Option<bool> {
            Some(event.value)
        }
    }

    impl Bindable for TextInput {
        type Value = String;
        type Event = TextEditChange;

        fn show(entity: &mut EntityWorldMut, value: &String) {
            if let Some(mut text) = entity.get_mut::<EditableText>()
                && text.value() != value.as_str()
            {
                text.editor.set_text(value);
                text.queue_edit(TextEdit::TextEnd(false));
            }
        }

        fn value(_: &TextEditChange, entity: EntityRef) -> Option<String> {
            Some(entity.get::<EditableText>()?.value().to_string())
        }
    }

    pub(super) fn register(app: &mut App) {
        use super::BindableAppExt;
        // `bind` shows a value by editing `EditableText`, so it must run before
        // the edits are applied. `UiPlugin` implies this order; without it the
        // two would be unordered and a stale edit could overwrite the value.
        app.configure_sets(
            PostUpdate,
            super::ReactiveSystems.before(bevy::text::EditableTextSystems),
        );
        app.register_bindable::<Slider>()
            .register_bindable::<Checkbox>()
            .register_bindable::<TextInput>();
    }
}

// ---------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------

/// A region: the scene of an entity whose children are reactive content.
/// Made by [`when`], [`if_else`], [`switch`], [`each`] and [`each_indexed`],
/// and included in `bsn!` with `@`.
#[must_use = "include it in a scene with `@`, such as `Children [ @when(..) ]`"]
pub struct RegionScene(BodyFactory);

impl Scene for RegionScene {
    fn resolve(
        self,
        _: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> std::result::Result<(), ResolveSceneError> {
        scene.push_template(RegionTemplate(self.0));
        Ok(())
    }
}

/// Makes its entity a [`Region`] and attaches the region's reaction.
struct RegionTemplate(BodyFactory);

impl Template for RegionTemplate {
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        context.entity.insert(Region);
        let host = context.entity.id();
        let body = (self.0)();
        context
            .entity
            .world_scope(|world| attach(world, host, body))?;
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

fn region<B: Body + 'static>(make: impl Fn() -> B + Send + Sync + 'static) -> RegionScene {
    RegionScene(Arc::new(move || Box::new(make()) as Box<dyn Body>))
}

fn spawn_children(world: &mut World, parent: Entity, list: Box<dyn SceneList>) -> Vec<Entity> {
    // Hold first runs until the content has its parent, so lookups through
    // ancestors work.
    world.resource_mut::<Pending>().held += 1;
    let entities = shared::spawn_children(world, parent, list, "bevy_radioactive::reactive");
    world.resource_mut::<Pending>().held -= 1;
    run_pending(world);
    entities
}

/// Shows the content for the current key, rebuilt when the key changes.
struct BranchBody<K> {
    key: Box<dyn Reader<K>>,
    render: Render<K>,
    current: Option<(K, Vec<Entity>)>,
}

impl<K: PartialEq + Send + Sync + 'static> Body for BranchBody<K> {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
        self.key.inputs(world, inputs);
    }

    fn run(&mut self, world: &mut World, cx: &Cx) {
        let Some(next) = self.key.read(world, cx) else {
            return;
        };
        if self.current.as_ref().is_some_and(|(key, _)| *key == next) {
            return;
        }
        if let Some((_, children)) = self.current.take() {
            despawn_all(world, &children);
        }
        let children = spawn_children(world, cx.host, (self.render)(&next));
        self.current = Some((next, children));
    }
}

/// The region behind [`when`], [`if_else`] and [`switch`].
fn branch<K: PartialEq + Send + Sync + 'static>(
    key: ReaderFactory<K>,
    render: Render<K>,
) -> RegionScene {
    region(move || BranchBody {
        key: key(),
        render: Arc::clone(&render),
        current: None,
    })
}

/// Shows `then` while `condition` returns `true`, and nothing otherwise.
/// The entity that includes it becomes a [`Region`] whose children are the
/// content; it is rebuilt only when the condition flips.
///
/// ```ignore
/// Children [
///     @when(|count: Ref<Count>| count.0 > 10, || bsn! { Text("That's a lot") })
/// ]
/// ```
pub fn when<M, C, L, R>(condition: C, then: R) -> RegionScene
where
    C: IntoReader<M, Output = bool> + Clone,
    L: SceneList,
    R: Fn() -> L + Send + Sync + 'static,
{
    branch(
        factory(condition),
        Arc::new(move |&shown: &bool| if shown { boxed(then()) } else { boxed(()) }),
    )
}

/// Shows `then` while `condition` returns `true`, and `otherwise` while it
/// returns `false`. Nest another one in `otherwise` for `else if`.
///
/// ```ignore
/// Children [
///     @if_else(
///         |count: Ref<Count>| count.0 >= 0,
///         || bsn! { Text("zero or more") },
///         || bsn! { Text("negative") },
///     )
/// ]
/// ```
pub fn if_else<M, C, L1, T, L2, E>(condition: C, then: T, otherwise: E) -> RegionScene
where
    C: IntoReader<M, Output = bool> + Clone,
    L1: SceneList,
    T: Fn() -> L1 + Send + Sync + 'static,
    L2: SceneList,
    E: Fn() -> L2 + Send + Sync + 'static,
{
    branch(
        factory(condition),
        Arc::new(move |&shown: &bool| {
            if shown {
                boxed(then())
            } else {
                boxed(otherwise())
            }
        }),
    )
}

/// Shows the content `render` returns for the current value of `key`,
/// rebuilt only when the value changes. Arms of different types are boxed
/// with `.into()`.
///
/// ```ignore
/// Children [
///     @switch(|tab: Res<Tab>| *tab, |tab| match tab {
///         Tab::Home => bsn! { @home() }.into(),
///         Tab::Settings => bsn_list! { @settings() -- @footer() }.into(),
///     })
/// ]
/// ```
pub fn switch<M, F, R>(key: F, render: R) -> RegionScene
where
    F: IntoReader<M> + Clone,
    F::Output: PartialEq + Send + Sync,
    R: Fn(&F::Output) -> Box<dyn SceneList> + Send + Sync + 'static,
{
    branch(factory(key), Arc::new(render))
}

// ---------------------------------------------------------------------------
// each
// ---------------------------------------------------------------------------

/// Shows the content `row` returns for each item `items` returns, one row
/// per `key`. A row whose key and item are unchanged keeps its entities, a
/// changed item rebuilds its row, and rows are reordered without being
/// rebuilt.
///
/// ```ignore
/// Children [
///     @each(
///         |todos: Res<Todos>| todos.0.clone(),
///         |todo| todo.id,
///         |todo| bsn! { Text({todo.title.clone()}) },
///     )
/// ]
/// ```
///
/// For rows that update in place, list entities instead of values and give
/// each row its entity as a [`Source`]: its reactions then track that
/// entity's components.
///
/// ```ignore
/// @each(
///     |todos: Query<Entity, With<Todo>>| todos.iter().collect::<Vec<_>>(),
///     |&todo| todo,
///     |&todo| bsn! {
///         Source({todo})
///         Text("") watch(|todo: Ref<Todo>| Text(todo.title.clone()))
///     },
/// )
/// ```
pub fn each<M, F, V, K, KF, L, R>(items: F, key: KF, row: R) -> RegionScene
where
    F: IntoReader<M> + Clone,
    F::Output: IntoIterator<Item = V>,
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
    KF: Fn(&V) -> K + Send + Sync + 'static,
    L: SceneList,
    R: Fn(&V) -> L + Send + Sync + 'static,
{
    list(
        items,
        Arc::new(move |_, item| key(item)),
        Arc::new(move |item| boxed(row(item))),
    )
}

/// Like [`each`], with rows identified by position: a changed item rebuilds
/// only the row at its position.
///
/// ```ignore
/// @each_indexed(|log: Res<Log>| log.lines.clone(), |line| bsn! { Text({line.clone()}) })
/// ```
pub fn each_indexed<M, F, V, L, R>(items: F, row: R) -> RegionScene
where
    F: IntoReader<M> + Clone,
    F::Output: IntoIterator<Item = V>,
    V: PartialEq + Send + Sync + 'static,
    L: SceneList,
    R: Fn(&V) -> L + Send + Sync + 'static,
{
    list(
        items,
        Arc::new(|index, _| index),
        Arc::new(move |item| boxed(row(item))),
    )
}

type Key<V, K> = Arc<dyn Fn(usize, &V) -> K + Send + Sync>;

/// The region behind [`each`] and [`each_indexed`].
fn list<M, F, V, K>(items: F, key: Key<V, K>, row: Render<V>) -> RegionScene
where
    F: IntoReader<M> + Clone,
    F::Output: IntoIterator<Item = V>,
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
{
    let items = factory(items);
    region(move || EachBody {
        items: items(),
        key: Arc::clone(&key),
        row: Arc::clone(&row),
        rows: HashMap::default(),
        order: Vec::new(),
    })
}

struct EachBody<I, V, K> {
    items: Box<dyn Reader<I>>,
    key: Key<V, K>,
    row: Render<V>,
    rows: HashMap<K, (V, Vec<Entity>)>,
    order: Vec<Entity>,
}

impl<I, V, K> Body for EachBody<I, V, K>
where
    I: IntoIterator<Item = V> + 'static,
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
{
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
        self.items.inputs(world, inputs);
    }

    fn run(&mut self, world: &mut World, cx: &Cx) {
        let Some(items) = self.items.read(world, cx) else {
            return;
        };
        let mut next_rows: HashMap<K, (V, Vec<Entity>)> = HashMap::default();
        let mut next_order = Vec::with_capacity(self.order.len());
        let mut duplicates = 0;
        for (index, item) in items.into_iter().enumerate() {
            let key = (self.key)(index, &item);
            let row = match self.rows.remove(&key) {
                Some((old, children)) if old == item => (old, children),
                old => {
                    if let Some((_, children)) = old {
                        despawn_all(world, &children);
                    }
                    let children = spawn_children(world, cx.host, (self.row)(&item));
                    (item, children)
                }
            };
            next_order.extend_from_slice(&row.1);
            if let Some((_, displaced)) = next_rows.insert(key, row) {
                // A duplicate key: the later item wins.
                duplicates += 1;
                next_order.retain(|entity| !displaced.contains(entity));
                despawn_all(world, &displaced);
            }
        }
        if duplicates > 0 {
            warn!(
                "reactive: {duplicates} duplicate key(s) in `each`; only the last item with each \
                 key is shown"
            );
        }
        for (_, (_, children)) in self.rows.drain() {
            despawn_all(world, &children);
        }
        self.rows = next_rows;
        if next_order != self.order
            && let Ok(mut host) = world.get_entity_mut(cx.host)
        {
            host.replace_children(&next_order);
        }
        self.order = next_order;
    }
}

// ---------------------------------------------------------------------------
// Nearest
// ---------------------------------------------------------------------------

/// Finds the nearest entity matching `D` from ordinary systems and
/// observers, the way reactive closures find `Ref<T>`: the entity itself,
/// its [`Source`], then its ancestors.
///
/// ```ignore
/// on(|click: On<Activate>, mut counts: Nearest<&mut Count>| {
///     if let Some(mut count) = counts.get_mut(click.entity) {
///         count.0 += 1;
///     }
/// })
/// ```
#[derive(SystemParam)]
pub struct Nearest<'w, 's, D: QueryData + 'static> {
    entities: Query<'w, 's, D, (Allow<Disabled>, Without<IsResource>)>,
    links: Query<'w, 's, (Option<&'static ChildOf>, Option<&'static Source>), Allow<Disabled>>,
}

impl<'s, D: IterQueryData + 'static> Nearest<'_, 's, D> {
    /// The nearest entity matching `D`.
    #[must_use]
    pub fn entity(&self, start: Entity) -> Option<Entity> {
        find_nearest(
            start,
            |entity| {
                self.links
                    .get(entity)
                    .map_or((None, None), |(parent, source)| {
                        (parent.map(ChildOf::parent), source.map(|source| source.0))
                    })
            },
            |entity| self.entities.contains(entity),
        )
    }

    /// The nearest match for `start`, read-only.
    #[must_use]
    pub fn get(&self, start: Entity) -> Option<bevy::ecs::query::ROQueryItem<'_, 's, D>> {
        self.entities.get(self.entity(start)?).ok()
    }

    /// The nearest match for `start`.
    pub fn get_mut(&mut self, start: Entity) -> Option<D::Item<'_, 's>> {
        let entity = self.entity(start)?;
        self.entities.get_mut(entity).ok()
    }
}

#[doc(hidden)]
pub mod __private {
    pub use super::{
        bind as __rsn_bind, each as __rsn_each, each_indexed as __rsn_each_indexed,
        handle as __rsn_handle, switch as __rsn_switch, watch as __rsn_watch,
    };

    /// Whether `value` differs from `place`; ties a `bind` writer's value type
    /// to the place it writes.
    pub fn __rsn_differs<T: PartialEq>(place: &T, value: &T) -> bool {
        place != value
    }
    pub use bevy::scene::{bsn, bsn_list};

    use super::{Cx, Disabled, Input, IntoReader, ReactiveParam, Read, Reader, record};
    use bevy::{
        ecs::{
            component::{ComponentId, ComponentIdFor},
            query::QueryEntityError,
            system::SystemParamItem,
        },
        prelude::*,
    };
    use std::marker::PhantomData;

    /// A reader closure returning `Option<V>`, read as one returning `V`
    /// that skips its run on `None`. Used for `$Ref<Type>(entity)`.
    pub struct Skip<F, M>(F, PhantomData<fn() -> M>);

    impl<F: Clone, M> Clone for Skip<F, M> {
        fn clone(&self) -> Self {
            Self(self.0.clone(), PhantomData)
        }
    }

    pub fn __rsn_skip<M, F: IntoReader<M>>(f: F) -> Skip<F, M> {
        Skip(f, PhantomData)
    }

    pub struct SkipMarker<M>(PhantomData<fn() -> M>);

    impl<M: 'static, V: 'static, F> IntoReader<SkipMarker<M>> for Skip<F, M>
    where
        F: IntoReader<M, Output = Option<V>>,
    {
        type Output = V;

        fn into_reader(self) -> Box<dyn Reader<V>> {
            Box::new(SkipReader(self.0.into_reader()))
        }
    }

    struct SkipReader<V>(Box<dyn Reader<Option<V>>>);

    impl<V> Reader<V> for SkipReader<V> {
        fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
            self.0.inputs(world, inputs);
        }

        fn read(&mut self, world: &mut World, cx: &Cx) -> Option<V> {
            self.0.read(world, cx).flatten()
        }
    }

    /// `$Ref<Type>(..)`'s parameter: like `Query<Ref<Type>>`, but re-runs
    /// the closure only when what it got changes, not any `Type` anywhere.
    #[allow(non_camel_case_types)]
    pub struct __rsn_Get<'w, 's, T: Component> {
        query: Query<'w, 's, Ref<'static, T>>,
        id: ComponentId,
    }

    impl<T: Component> __rsn_Get<'_, '_, T> {
        /// `T` on `entity`, recording the read.
        pub fn get(&self, entity: Entity) -> Result<Ref<'_, T>, QueryEntityError> {
            let found = self.query.get(entity);
            record(if found.is_ok() {
                Read::At(entity, self.id)
            } else {
                Read::Missing(self.id)
            });
            found
        }
    }

    impl<T: Component> ReactiveParam for __rsn_Get<'_, '_, T> {
        type Param = (
            Query<'static, 'static, Ref<'static, T>>,
            ComponentIdFor<'static, T>,
        );
        type Item<'w, 's> = __rsn_Get<'w, 's, T>;

        fn fetch<'w, 's>(
            (query, id): SystemParamItem<'w, 's, Self::Param>,
            _: &Cx,
        ) -> Option<Self::Item<'w, 's>> {
            Some(__rsn_Get {
                query,
                id: id.get(),
            })
        }
    }

    /// Finds `#Name`d entities for `$Ref<Type>(#Name)`: the start entity and its
    /// subtree, then each ancestor and its subtree.
    #[allow(non_camel_case_types)]
    #[derive(bevy::ecs::system::SystemParam)]
    pub struct __rsn_Names<'w, 's> {
        names: Query<'w, 's, &'static Name, Allow<Disabled>>,
        parents: Query<'w, 's, &'static ChildOf, Allow<Disabled>>,
        children: Query<'w, 's, &'static Children, Allow<Disabled>>,
        warned: Local<'s, bool>,
    }

    impl __rsn_Names<'_, '_> {
        /// `found`, warning once per closure in debug builds when it is
        /// `None`: the name matched no entity, or the entity lacks `T`.
        pub fn check<T, V>(&mut self, found: Option<V>, name: &str) -> Option<V> {
            #[cfg(debug_assertions)]
            if found.is_none() && !*self.warned {
                *self.warned = true;
                warn!(
                    "rsn!: found no entity named `{name}` with a `{}` near this one; the \
                     closure is skipped until one appears",
                    std::any::type_name::<T>(),
                );
            }
            #[cfg(not(debug_assertions))]
            let _ = (name, &self.warned);
            found
        }

        /// The nearest entity named `name`, or `Entity::PLACEHOLDER`.
        pub fn find(&self, start: Entity, name: &str) -> Entity {
            let mut current = Some(start);
            let mut skip = None;
            while let Some(entity) = current {
                let mut queue = std::collections::VecDeque::from([entity]);
                while let Some(next) = queue.pop_front() {
                    if self.names.get(next).is_ok_and(|n| n.as_str() == name) {
                        return next;
                    }
                    if let Ok(children) = self.children.get(next) {
                        queue.extend(children.iter().filter(|&c| Some(c) != skip));
                    }
                }
                skip = Some(entity);
                current = self.parents.get(entity).ok().map(ChildOf::parent);
            }
            Entity::PLACEHOLDER
        }
    }

    /// An owned copy of a query item, for `for x in $Query<D> { .. }` rows.
    #[allow(non_camel_case_types)]
    pub trait __rsn_Own {
        type Owned: Clone + PartialEq + Send + Sync + 'static;
        fn __rsn_own(self) -> Self::Owned;
    }

    impl __rsn_Own for Entity {
        type Owned = Entity;
        fn __rsn_own(self) -> Entity {
            self
        }
    }

    impl<T: Clone + PartialEq + Send + Sync + 'static> __rsn_Own for &T {
        type Owned = T;
        fn __rsn_own(self) -> T {
            self.clone()
        }
    }

    impl<T: Component + Clone + PartialEq> __rsn_Own for Ref<'_, T> {
        type Owned = T;
        fn __rsn_own(self) -> T {
            (*self).clone()
        }
    }

    impl<A: __rsn_Own> __rsn_Own for Option<A> {
        type Owned = Option<A::Owned>;
        fn __rsn_own(self) -> Self::Owned {
            self.map(A::__rsn_own)
        }
    }

    macro_rules! impl_own {
        ($($t:ident),*) => {
            #[allow(non_snake_case, reason = "the bindings are type names")]
            impl<$($t: __rsn_Own),*> __rsn_Own for ($($t,)*) {
                type Owned = ($($t::Owned,)*);
                fn __rsn_own(self) -> Self::Owned {
                    let ($($t,)*) = self;
                    ($($t.__rsn_own(),)*)
                }
            }
        };
    }
    impl_own!(A);
    impl_own!(A, B);
    impl_own!(A, B, C);
    impl_own!(A, B, C, D);
    impl_own!(A, B, C, D, E);
    impl_own!(A, B, C, D, E, F);
}
