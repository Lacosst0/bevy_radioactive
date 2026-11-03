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
//! and `$Type`, "the nearest `Type`" (this entity, its [`Source`], its
//! ancestors, then resource `Type`). Whatever reads a `$Type` re-runs when it
//! changes.
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
//!         Node
//!         Count(0)                                 // state, local to this counter
//!         Children [
//!             Button
//!             on(|_: On<Activate>| $Count.0 += 1) // writes the nearest Count
//!             --
//!             Text({ $Count.0.to_string() })       // re-renders when it changes
//!             --
//!             if $Count.0 > 10 { Text("That's a lot") } else { Text("Keep going") }
//!             --
//!             #[key(todo.id)]
//!             for todo in $Todos.0.clone() { Text({ todo.title.clone() }) }
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
//! | `Comp(.. $T ..)` | a [`watch`] that rebuilds `Comp` when `T` changes |
//! | `Comp { a: .. $T .., b: .. }` | `Comp { b: .. }` plus a [`watch`] that patches `a` |
//! | `on(\|e: On<E>\| .. $T ..)` | a [`handle`]r where `$T` is the nearest `T`, writable |
//! | `if`/`else if`/`else` | a [`switch`] region showing one branch |
//! | `match value { pat => .., }` | a [`switch`] region; arms are `pat => scene,` or `pat => { scene -- scene }` |
//! | `#[key(expr)] for pat in iter { .. }` | an [`each`] region, keyed |
//! | `for pat in iter { .. }` | an [`each_indexed`] region |
//! | anything else | passed to `bsn!` unchanged |
//!
//! Without `$`, `if`/`match`/`for` run once. `for` needs owned items that
//! implement `PartialEq`. Loop and match bindings may be used any number of
//! times in the body. [`rsn_list!`] is the list form.
//!
//! The functions it expands to are public and take closures with ordinary
//! Bevy system parameters; use them in `bsn!` or plain Rust, and for inputs
//! `$Type` doesn't cover, such as a `Query`:
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
//! # Parameters
//!
//! | In a closure | Re-runs it? | Meaning |
//! | --- | --- | --- |
//! | `c: Ref<T>` | yes | the nearest `T`: this entity, its [`Source`], its ancestors, then resource `T` |
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
//! | `@`[`when`]`(cond, then)` | show entities while a condition holds |
//! | `@r#if(cond, then, otherwise)` | `if`/`else`; also named [`if_else`] |
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

use std::{
    any::TypeId,
    hash::Hash,
    marker::PhantomData,
    sync::{Arc, Mutex, PoisonError},
};

use bevy::{
    ecs::{
        change_detection::Tick,
        component::{ComponentId, Mutable},
        entity_disabling::Disabled,
        event::EntityEvent,
        message::Message,
        query::{Allow, IterQueryData, QueryData, QueryFilter},
        resource::IsResource,
        system::{SystemParam, SystemParamItem, SystemState},
        template::{FromTemplate, Template, TemplateContext},
    },
    platform::collections::HashMap,
    prelude::*,
    scene::{
        EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, SceneList,
        WorldSceneExt,
    },
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
            .add_systems(PostUpdate, run_reactions.in_set(ReactiveSystems));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            ReactiveSystems.before(bevy::ui::UiSystems::Prepare),
        );
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

/// Host entity of a [`when`], [`switch`] or [`each`] region. With the `ui`
/// feature it is a `GhostNode`, so its children are laid out as children of
/// its parent.
#[derive(Component, Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "ui", require(bevy::ui::experimental::GhostNode))]
#[cfg_attr(feature = "ui", component(on_add = strip_ghost_transform))]
pub struct Region;

#[cfg(feature = "ui")]
fn strip_ghost_transform(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    // `GhostNode` requires `Transform`, which UI layout doesn't use and Bevy
    // warns about under a UI parent.
    world
        .commands()
        .entity(context.entity)
        .remove::<(Transform, GlobalTransform)>();
}

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
    last_run: Option<Tick>,
    body: Option<Box<dyn Body>>,
}

// ---------------------------------------------------------------------------
// Inputs and resolution
// ---------------------------------------------------------------------------

/// Where an input is looked up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lookup {
    /// This entity, its `Source`, its ancestors, then the resource.
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
    tracked: bool,
}

impl Input {
    fn same_target(&self, other: &Self) -> bool {
        self.id == other.id && self.lookup == other.lookup
    }
}

/// The entity holding component `id` for a reaction declared on `host`.
fn resolve(world: &World, host: Entity, id: ComponentId, lookup: Lookup) -> Option<Entity> {
    match lookup {
        Lookup::Any => None,
        Lookup::Resource => world.resource_entities().get(id),
        Lookup::Nearest => nearest(world, host, id).or_else(|| world.resource_entities().get(id)),
    }
}

fn has(world: &World, entity: Entity, id: ComponentId) -> bool {
    world
        .get_entity(entity)
        .is_ok_and(|entity| entity.contains_id(id))
}

/// `start`, its `Source`, then the same for each ancestor.
fn nearest(world: &World, start: Entity, id: ComponentId) -> Option<Entity> {
    let mut current = Some(start);
    let mut steps = 0;
    while let Some(entity) = current {
        if has(world, entity, id) {
            return Some(entity);
        }
        if let Some(&Source(source)) = world.get::<Source>(entity)
            && has(world, source, id)
        {
            return Some(source);
        }
        current = world.get::<ChildOf>(entity).map(ChildOf::parent);
        steps += 1;
        if steps > 4096 {
            break;
        }
    }
    None
}

/// What one run of a closure sees.
#[doc(hidden)]
pub struct Cx<'a> {
    host: Entity,
    inputs: &'a [Input],
    resolved: &'a [Option<Entity>],
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
/// parameters), and how many entities had it.
#[derive(Resource, Default)]
struct AnyChanges(HashMap<ComponentId, AnyState>);

struct AnyState {
    count: u64,
    stamp: Tick,
}

fn scan_any(world: &mut World) {
    let ids: Vec<ComponentId> = world.resource::<AnyChanges>().0.keys().copied().collect();
    if ids.is_empty() {
        return;
    }
    let now = world.change_tick();
    let mut updates = Vec::new();
    for id in ids {
        let state = &world.resource::<AnyChanges>().0[&id];
        let (old_count, mut stamp) = (state.count, state.stamp);
        let mut count = 0;
        for archetype in world.archetypes().iter() {
            if !archetype.contains(id) {
                continue;
            }
            count += u64::from(archetype.len());
            for entity in archetype.entities() {
                if let Some(ticks) = world
                    .get_entity(entity.id())
                    .ok()
                    .and_then(|entity| entity.get_change_ticks_by_id(id))
                    && ticks.changed.is_newer_than(stamp, now)
                {
                    stamp = ticks.changed;
                }
            }
        }
        if count != old_count {
            // An entity gained or lost the component.
            stamp = now;
        }
        updates.push((id, AnyState { count, stamp }));
    }
    let mut changes = world.resource_mut::<AnyChanges>();
    for (id, state) in updates {
        changes.0.insert(id, state);
    }
}

fn track_any(world: &mut World, inputs: &[Input]) {
    let now = world.change_tick();
    let mut missing = Vec::new();
    {
        let changes = world.resource::<AnyChanges>();
        for input in inputs {
            if input.lookup == Lookup::Any && !changes.0.contains_key(&input.id) {
                missing.push(input.id);
            }
        }
    }
    for id in missing {
        let count = world
            .archetypes()
            .iter()
            .filter(|archetype| archetype.contains(id))
            .map(|archetype| u64::from(archetype.len()))
            .sum();
        world
            .resource_mut::<AnyChanges>()
            .0
            .insert(id, AnyState { count, stamp: now });
    }
}

impl Reaction {
    /// Whether a tracked input changed since the last run.
    fn is_dirty(&self, world: &World, changes: &AnyChanges, now: Tick) -> bool {
        let Some(last_run) = self.last_run else {
            return true;
        };
        for (index, input) in self.inputs.iter().enumerate() {
            match input.lookup {
                Lookup::Any => {
                    if input.tracked
                        && changes
                            .0
                            .get(&input.id)
                            .is_some_and(|state| state.stamp.is_newer_than(last_run, now))
                    {
                        return true;
                    }
                }
                Lookup::Nearest | Lookup::Resource => {
                    let entity = resolve(world, self.host, input.id, input.lookup);
                    if entity != self.resolved.get(index).copied().flatten() {
                        return true;
                    }
                    if input.tracked
                        && let Some(entity) = entity
                        && let Some(ticks) = world
                            .get_entity(entity)
                            .ok()
                            .and_then(|entity| entity.get_change_ticks_by_id(input.id))
                        && ticks.is_changed(last_run, now)
                    {
                        return true;
                    }
                }
            }
        }
        false
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

fn nearest_input<T: Component>(world: &mut World, tracked: bool) -> Input {
    Input {
        type_id: TypeId::of::<T>(),
        id: world.register_component::<T>(),
        lookup: Lookup::Nearest,
        tracked,
    }
}

fn resource_input<T: Resource>(world: &mut World) -> Input {
    Input {
        type_id: TypeId::of::<T>(),
        id: world.register_component::<T>(),
        lookup: Lookup::Resource,
        tracked: true,
    }
}

/// Every component a query reads or filters on, except those it writes.
fn query_inputs<D: QueryData + 'static, F: QueryFilter + 'static>(
    world: &mut World,
    inputs: &mut Vec<Input>,
) {
    let state = QueryState::<D, F>::new(world);
    let access = state.component_access();
    let mut ids: Vec<ComponentId> = Vec::new();
    if let Some(reads) = access.access().reads().as_finite_set() {
        ids.extend(reads.iter());
    }
    ids.extend(access.with_filters());
    ids.extend(access.without_filters());
    let writes = access.access().writes();
    let is_resource = world.register_component::<IsResource>();
    for id in ids {
        if writes.contains(id) || id == is_resource {
            continue;
        }
        inputs.push(Input {
            type_id: TypeId::of::<()>(),
            id,
            lookup: Lookup::Any,
            tracked: true,
        });
    }
}

impl<T: Component> ReactiveParam for Ref<'_, T> {
    type Param = Query<'static, 'static, Ref<'static, T>, Allow<Disabled>>;
    type Item<'w, 's> = Ref<'w, T>;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(nearest_input::<T>(world, true));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        param.get_inner(cx.entity_of::<T>(Lookup::Nearest)?).ok()
    }
}

impl<T: Component> ReactiveParam for Option<Ref<'_, T>> {
    type Param = Query<'static, 'static, Ref<'static, T>, Allow<Disabled>>;
    type Item<'w, 's> = Option<Ref<'w, T>>;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(nearest_input::<T>(world, true));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        Some(
            cx.entity_of::<T>(Lookup::Nearest)
                .and_then(|entity| param.get_inner(entity).ok()),
        )
    }
}

impl<C: Component<Mutability = Mutable>> ReactiveParam for &mut C {
    type Param = Query<'static, 'static, &'static mut C, Allow<Disabled>>;
    type Item<'w, 's> = &'w mut C;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(nearest_input::<C>(world, false));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        cx: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        param
            .get_inner(cx.entity_of::<C>(Lookup::Nearest)?)
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

impl<T: Resource> ReactiveParam for Res<'_, T> {
    type Param = Res<'static, T>;
    type Item<'w, 's> = Res<'w, T>;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(resource_input::<T>(world));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        _: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        Some(param)
    }
}

impl<T: Resource> ReactiveParam for Option<Res<'_, T>> {
    type Param = Option<Res<'static, T>>;
    type Item<'w, 's> = Option<Res<'w, T>>;

    fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
        inputs.push(resource_input::<T>(world));
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        _: &Cx,
    ) -> Option<Self::Item<'w, 's>> {
        Some(param)
    }
}

/// Implements [`ReactiveParam`] for tracked Bevy queries.
macro_rules! tracked_queries {
    ($([$($generics:tt)*] $ty:ty => $param:ty, $item:ty;)*) => {$(
        impl<$($generics)*> ReactiveParam for $ty {
            type Param = $param;
            type Item<'w, 's> = $item;

            fn inputs(world: &mut World, inputs: &mut Vec<Input>) {
                query_inputs::<D, F>(world, inputs);
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

tracked_queries! {
    ['a, 'b, D: QueryData + 'static, F: QueryFilter + 'static] Query<'a, 'b, D, F>
        => Query<'static, 'static, D, F>, Query<'w, 's, D, F>;
    ['a, 'b, D: IterQueryData + 'static, F: QueryFilter + 'static] Single<'a, 'b, D, F>
        => Single<'static, 'static, D, F>, Single<'w, 's, D, F>;
    ['a, 'b, D: IterQueryData + 'static, F: QueryFilter + 'static] Option<Single<'a, 'b, D, F>>
        => Option<Single<'static, 'static, D, F>>, Option<Single<'w, 's, D, F>>;
    ['a, 'b, D: QueryData + 'static, F: QueryFilter + 'static] Populated<'a, 'b, D, F>
        => Populated<'static, 'static, D, F>, Populated<'w, 's, D, F>;
}

/// Implements [`ReactiveParam`] for untracked Bevy system parameters.
macro_rules! untracked_params {
    ($([$($generics:tt)*] $ty:ty => $param:ty, $item:ty;)*) => {$(
        impl<$($generics)*> ReactiveParam for $ty {
            type Param = $param;
            type Item<'w, 's> = $item;

            fn fetch<'w, 's>(
                param: SystemParamItem<'w, 's, Self::Param>,
                _: &Cx,
            ) -> Option<Self::Item<'w, 's>> {
                Some(param)
            }
        }
    )*};
}

untracked_params! {
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

struct FnReader<W, M, P: SystemParam + 'static> {
    f: W,
    state: Option<SystemState<P>>,
    marker: PhantomData<fn() -> M>,
}

macro_rules! impl_reader {
    ($($param:ident),*) => {
        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, V, $($param: ReactiveParam + 'static),*> IntoReader<fn($($param,)*) -> V> for W
        where
            W: Fn($($param),*) -> V + Fn($($param::Item<'_, '_>),*) -> V + Send + Sync + 'static,
            V: 'static,
        {
            type Output = V;

            fn into_reader(self) -> Box<dyn Reader<V>> {
                Box::new(FnReader::<W, fn($($param,)*) -> V, ($($param::Param,)*)> {
                    f: self,
                    state: None,
                    marker: PhantomData,
                })
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
    };
}

impl_reader!();
impl_reader!(P0);
impl_reader!(P0, P1);
impl_reader!(P0, P1, P2);
impl_reader!(P0, P1, P2, P3);
impl_reader!(P0, P1, P2, P3, P4);
impl_reader!(P0, P1, P2, P3, P4, P5);
impl_reader!(P0, P1, P2, P3, P4, P5, P6);
impl_reader!(P0, P1, P2, P3, P4, P5, P6, P7);

/// Makes fresh readers of one closure.
type ReaderFactory<V> = Arc<dyn Fn() -> Box<dyn Reader<V>> + Send + Sync>;

fn factory<M, F: IntoReader<M> + Clone>(f: F) -> ReaderFactory<F::Output> {
    Arc::new(move || f.clone().into_reader())
}

/// Content of a region branch.
type Render = Arc<dyn Fn() -> Box<dyn SceneList> + Send + Sync>;

fn render<L: SceneList, R: Fn() -> L + Send + Sync + 'static>(render: R) -> Render {
    Arc::new(move || Box::new(render()) as Box<dyn SceneList>)
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
    let mut unique: Vec<Input> = Vec::new();
    for input in inputs {
        if let Some(existing) = unique.iter_mut().find(|other| other.same_target(&input)) {
            existing.tracked |= input.tracked;
        } else {
            unique.push(input);
        }
    }
    track_any(world, &unique);
    let reaction = world
        .spawn((
            Reaction {
                host,
                inputs: unique,
                resolved: Vec::new(),
                last_run: None,
                body: Some(body),
            },
            ReactionOf(host),
        ))
        .id();
    // After the rest of the scene spawns, so ancestors and `Source`s exist.
    world.resource_mut::<Pending>().reactions.push(reaction);
    world.commands().queue(run_pending);
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

/// Runs a reaction now. Returns whether it ran.
fn run_reaction(world: &mut World, reaction: Entity) -> bool {
    let Some(mut state) = world.get_mut::<Reaction>(reaction) else {
        return false;
    };
    let state = state.bypass_change_detection();
    let Some(mut body) = state.body.take() else {
        return false;
    };
    let host = state.host;
    let inputs = state.inputs.clone();
    if world.get_entity(host).is_err() {
        return false;
    }
    let resolved: Vec<Option<Entity>> = inputs
        .iter()
        .map(|input| resolve(world, host, input.id, input.lookup))
        .collect();
    body.run(
        world,
        &Cx {
            host,
            inputs: &inputs,
            resolved: &resolved,
        },
    );
    // Its own writes are older than `last_run`; later ones are newer.
    let last_run = world.increment_change_tick();
    if let Some(mut state) = world.get_mut::<Reaction>(reaction) {
        let state = state.bypass_change_detection();
        state.body = Some(body);
        state.resolved = resolved;
        state.last_run = Some(last_run);
    }
    world.flush();
    true
}

fn depth(world: &World, mut entity: Entity) -> usize {
    let mut depth = 0;
    while let Some(parent) = world.get::<ChildOf>(entity) {
        entity = parent.parent();
        depth += 1;
        if depth > 4096 {
            break;
        }
    }
    depth
}

fn run_reactions(world: &mut World, reactions: &mut QueryState<(Entity, &Reaction)>) {
    for _ in 0..MAX_ROUNDS {
        world.flush();
        run_pending(world);
        scan_any(world);
        let now = world.change_tick();
        let mut due: Vec<(usize, Entity)> = {
            let world = &*world;
            let changes = world.resource::<AnyChanges>();
            reactions
                .iter(world)
                .filter(|(_, reaction)| {
                    reaction.body.is_some()
                        && !world
                            .get_entity(reaction.host)
                            .is_ok_and(|host| host.contains::<Disabled>())
                        && reaction.is_dirty(world, changes, now)
                })
                .map(|(entity, reaction)| (depth(world, reaction.host), entity))
                .collect()
        };
        if due.is_empty() {
            return;
        }
        // Outer regions first: they may despawn the reactions inside them.
        due.sort_by_key(|&(depth, entity)| (depth, entity));
        for (_, reaction) in due {
            run_reaction(world, reaction);
        }
    }
    warn!(
        "reactive: values still changing after {MAX_ROUNDS} rounds; the rest runs next frame. \
         A reaction may be writing a value it reads."
    );
}

/// A template that attaches one reaction to its entity.
struct ReactionTemplate {
    make: BodyFactory,
    region: bool,
}

impl Template for ReactionTemplate {
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        if self.region {
            context.entity.insert(Region);
        }
        let host = context.entity.id();
        let body = (self.make)();
        context
            .entity
            .world_scope(|world| attach(world, host, body))?;
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            make: Arc::clone(&self.make),
            region: self.region,
        }
    }
}

/// Something [`ReactExt::react`] can attach to an entity: [`watch`] or
/// [`handle`].
pub trait Reactive: Send + 'static {
    #[doc(hidden)]
    fn attach_to(self, world: &mut World, host: Entity) -> Result<()>;
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

struct WatchBody<R> {
    reader: Box<dyn Reader<R>>,
}

impl<R: Bundle> Body for WatchBody<R> {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
        self.reader.inputs(world, inputs);
    }

    fn run(&mut self, world: &mut World, cx: &Cx) {
        let Some(output) = self.reader.read(world, cx) else {
            return;
        };
        if TypeId::of::<R>() != TypeId::of::<()>()
            && let Ok(mut host) = world.get_entity_mut(cx.host)
        {
            host.insert(output);
        }
    }
}

impl<M: 'static, F> Watch<F, M>
where
    F: IntoReader<M> + Clone,
    F::Output: Bundle,
{
    fn body(&self) -> Box<dyn Body> {
        Box::new(WatchBody {
            reader: self.0.clone().into_reader(),
        })
    }
}

impl<M: 'static, F> Template for Watch<F, M>
where
    F: IntoReader<M> + Clone,
    F::Output: Bundle,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let host = context.entity.id();
        let body = self.body();
        context
            .entity
            .world_scope(|world| attach(world, host, body))?;
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<M: 'static, F> Reactive for Watch<F, M>
where
    F: IntoReader<M> + Clone,
    F::Output: Bundle,
{
    fn attach_to(self, world: &mut World, host: Entity) -> Result<()> {
        attach(world, host, self.body()).map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// handle
// ---------------------------------------------------------------------------

/// A closure that [`handle`] runs: its first parameter is `&E`, the rest are
/// [`ReactiveParam`]s.
pub trait IntoHandler<E, M>: Send + Sync + 'static {
    #[doc(hidden)]
    fn into_runner(self) -> Box<dyn HandlerRunner<E>>;
}

/// A [`handle`] closure with its system state.
#[doc(hidden)]
pub trait HandlerRunner<E>: Send + Sync {
    fn run(&mut self, world: &mut World, host: Entity, event: &E);
}

struct FnHandler<W, M, P: SystemParam + 'static> {
    f: W,
    state: Option<(SystemState<P>, Vec<Input>)>,
    marker: PhantomData<fn() -> M>,
}

macro_rules! impl_handler {
    ($($param:ident),*) => {
        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, E, $($param: ReactiveParam + 'static),*> IntoHandler<E, fn(&E, $($param,)*)> for W
        where
            E: 'static,
            W: Fn(&E, $($param),*) + Fn(&E, $($param::Item<'_, '_>),*) + Send + Sync + 'static,
        {
            fn into_runner(self) -> Box<dyn HandlerRunner<E>> {
                Box::new(FnHandler::<W, fn(&E, $($param,)*), ($($param::Param,)*)> {
                    f: self,
                    state: None,
                    marker: PhantomData,
                })
            }
        }

        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, E, $($param: ReactiveParam + 'static),*> HandlerRunner<E>
            for FnHandler<W, fn(&E, $($param,)*), ($($param::Param,)*)>
        where
            E: 'static,
            W: Fn(&E, $($param),*) + Fn(&E, $($param::Item<'_, '_>),*) + Send + Sync + 'static,
        {
            fn run(&mut self, world: &mut World, host: Entity, event: &E) {
                #[allow(clippy::too_many_arguments, reason = "one per closure parameter")]
                fn call<E, $($param),*>(f: impl Fn(&E, $($param),*), event: &E, $($param: $param),*) {
                    f(event, $($param),*);
                }
                let (state, inputs) = self.state.get_or_insert_with(|| {
                    #[allow(unused_mut, reason = "empty for handlers without parameters")]
                    let mut inputs = Vec::new();
                    $($param::inputs(world, &mut inputs);)*
                    (SystemState::new(world), inputs)
                });
                let resolved: Vec<Option<Entity>> = inputs
                    .iter()
                    .map(|input| resolve(world, host, input.id, input.lookup))
                    .collect();
                let _cx = Cx { host, inputs, resolved: &resolved };
                {
                    let Ok(($($param,)*)) = state.get_mut(world) else {
                        return;
                    };
                    $(let Some($param) = $param::fetch($param, &_cx) else {
                        return;
                    };)*
                    call(&self.f, event, $($param),*);
                }
                state.apply(world);
            }
        }
    };
}

impl_handler!();
impl_handler!(P0);
impl_handler!(P0, P1);
impl_handler!(P0, P1, P2);
impl_handler!(P0, P1, P2, P3);
impl_handler!(P0, P1, P2, P3, P4);
impl_handler!(P0, P1, P2, P3, P4, P5);
impl_handler!(P0, P1, P2, P3, P4, P5, P6);

/// Runs `f` whenever entity event `E` reaches this entity, including events
/// that bubble up from its descendants. The other parameters work as in
/// [`watch`], so `&mut C` writes the nearest `C`: this entity's, an
/// ancestor's, or the resource.
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

impl<E, F, M: 'static> Handler<E, F, M>
where
    E: EntityEvent + Clone,
    F: IntoHandler<E, M> + Clone,
{
    fn observer(&self, host: Entity) -> impl FnMut(On<E>, Commands) + Send + Sync + 'static {
        let runner = Arc::new(Mutex::new(self.0.clone().into_runner()));
        move |event: On<E>, mut commands: Commands| {
            let event = event.event().clone();
            let runner = Arc::clone(&runner);
            commands.queue(move |world: &mut World| {
                runner
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .run(world, host, &event);
            });
        }
    }
}

impl<E, F, M: 'static> Template for Handler<E, F, M>
where
    E: EntityEvent + Clone,
    F: IntoHandler<E, M> + Clone,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let host = context.entity.id();
        context.entity.observe(self.observer(host));
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<E, F, M: 'static> Reactive for Handler<E, F, M>
where
    E: EntityEvent + Clone,
    F: IntoHandler<E, M> + Clone,
{
    fn attach_to(self, world: &mut World, host: Entity) -> Result<()> {
        let observer = self.observer(host);
        world
            .get_entity_mut(host)
            .map_err(|_| "react: the entity doesn't exist")?
            .observe(observer);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------

/// A region: the scene of an entity whose children are reactive content.
/// Made by [`when`], `r#if`, [`if_else`], [`switch`], [`each`] and
/// [`each_indexed`], and included in `bsn!` with `@`.
#[must_use = "include it in a scene with `@`, such as `Children [ @when(..) ]`"]
pub struct RegionScene(BodyFactory);

impl Scene for RegionScene {
    fn resolve(
        self,
        _: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> std::result::Result<(), ResolveSceneError> {
        scene.push_template(ReactionTemplate {
            make: self.0,
            region: true,
        });
        Ok(())
    }
}

fn spawn_children(world: &mut World, parent: Entity, list: Box<dyn SceneList>) -> Vec<Entity> {
    // Hold first runs until the content has its parent, so lookups through
    // ancestors work.
    world.resource_mut::<Pending>().held += 1;
    let spawned = world.spawn_scene_list(list);
    world.resource_mut::<Pending>().held -= 1;
    match spawned {
        Ok(entities) => {
            for &entity in &entities {
                world.entity_mut(entity).insert(ChildOf(parent));
            }
            run_pending(world);
            entities
        }
        Err(error) => {
            let handler = world.fallback_error_handler();
            handler(
                BevyError::from(error),
                bevy::ecs::error::ErrorContext::Command {
                    name: "bevy_radioactive::reactive::spawn".into(),
                },
            );
            Vec::new()
        }
    }
}

fn despawn_all(world: &mut World, entities: &[Entity]) {
    for &entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
}

/// Shows one branch at a time, rebuilt when the chosen branch changes.
struct BranchBody<K> {
    key: Box<dyn Reader<K>>,
    render: Arc<dyn Fn(&K) -> Option<Box<dyn SceneList>> + Send + Sync>,
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
        let children = (self.render)(&next)
            .map(|list| spawn_children(world, cx.host, list))
            .unwrap_or_default();
        self.current = Some((next, children));
    }
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
    let condition = factory(condition);
    let then = render(then);
    RegionScene(Arc::new(move || {
        let then = Arc::clone(&then);
        Box::new(BranchBody {
            key: condition(),
            render: Arc::new(move |shown: &bool| shown.then(|| then())),
            current: None,
        })
    }))
}

/// Shows `then` while `condition` returns `true`, and `otherwise` while it
/// returns `false`. `if` is a Rust keyword, so `bsn!` spells it `@r#if`;
/// [`if_else`] is the same function under a plain name. Nest another one in
/// `otherwise` for `else if`.
///
/// ```ignore
/// Children [
///     @r#if(
///         |count: Ref<Count>| count.0 >= 0,
///         || bsn! { Text("zero or more") },
///         || bsn! { Text("negative") },
///     )
/// ]
/// ```
pub fn r#if<M, C, L1, T, L2, E>(condition: C, then: T, otherwise: E) -> RegionScene
where
    C: IntoReader<M, Output = bool> + Clone,
    L1: SceneList,
    T: Fn() -> L1 + Send + Sync + 'static,
    L2: SceneList,
    E: Fn() -> L2 + Send + Sync + 'static,
{
    let condition = factory(condition);
    let (then, otherwise) = (render(then), render(otherwise));
    RegionScene(Arc::new(move || {
        let (then, otherwise) = (Arc::clone(&then), Arc::clone(&otherwise));
        Box::new(BranchBody {
            key: condition(),
            render: Arc::new(move |shown: &bool| Some(if *shown { then() } else { otherwise() })),
            current: None,
        })
    }))
}

/// `r#if` under a name that needs no `r#`: `@if_else(cond, then, otherwise)`.
pub fn if_else<M, C, L1, T, L2, E>(condition: C, then: T, otherwise: E) -> RegionScene
where
    C: IntoReader<M, Output = bool> + Clone,
    L1: SceneList,
    T: Fn() -> L1 + Send + Sync + 'static,
    L2: SceneList,
    E: Fn() -> L2 + Send + Sync + 'static,
{
    r#if(condition, then, otherwise)
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
    let key = factory(key);
    let render = Arc::new(render);
    RegionScene(Arc::new(move || {
        let render = Arc::clone(&render);
        Box::new(BranchBody {
            key: key(),
            render: Arc::new(move |value: &F::Output| Some(render(value))),
            current: None,
        })
    }))
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
    list(items, Arc::new(move |_, item| key(item)), row)
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
    list(items, Arc::new(|index, _| index), row)
}

fn list<M, F, V, K, L, R>(
    items: F,
    key: Arc<dyn Fn(usize, &V) -> K + Send + Sync>,
    row: R,
) -> RegionScene
where
    F: IntoReader<M> + Clone,
    F::Output: IntoIterator<Item = V>,
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
    L: SceneList,
    R: Fn(&V) -> L + Send + Sync + 'static,
{
    let items = factory(items);
    let row: Arc<dyn Fn(&V) -> Box<dyn SceneList> + Send + Sync> =
        Arc::new(move |item| Box::new(row(item)) as Box<dyn SceneList>);
    RegionScene(Arc::new(move || {
        Box::new(EachBody {
            items: Box::new(Collect {
                reader: items(),
                marker: PhantomData,
            }),
            key: Arc::clone(&key),
            row: Arc::clone(&row),
            rows: HashMap::default(),
            order: Vec::new(),
        })
    }))
}

struct Collect<I, V> {
    reader: Box<dyn Reader<I>>,
    marker: PhantomData<fn() -> V>,
}

impl<I: IntoIterator<Item = V> + 'static, V: 'static> Reader<Vec<V>> for Collect<I, V> {
    fn inputs(&self, world: &mut World, inputs: &mut Vec<Input>) {
        self.reader.inputs(world, inputs);
    }

    fn read(&mut self, world: &mut World, cx: &Cx) -> Option<Vec<V>> {
        self.reader
            .read(world, cx)
            .map(|items| items.into_iter().collect())
    }
}

struct EachBody<V, K> {
    items: Box<dyn Reader<Vec<V>>>,
    key: Arc<dyn Fn(usize, &V) -> K + Send + Sync>,
    row: Arc<dyn Fn(&V) -> Box<dyn SceneList> + Send + Sync>,
    rows: HashMap<K, (V, Vec<Entity>)>,
    order: Vec<Entity>,
}

impl<V, K> Body for EachBody<V, K>
where
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
        let mut next_order = Vec::new();
        let mut duplicates = 0;
        for (index, item) in items.into_iter().enumerate() {
            let key = (self.key)(index, &item);
            let row = match self.rows.remove(&key) {
                Some((old, children)) if old == item => (old, children),
                Some((_, children)) => {
                    despawn_all(world, &children);
                    let children = spawn_children(world, cx.host, (self.row)(&item));
                    (item, children)
                }
                None => {
                    let children = spawn_children(world, cx.host, (self.row)(&item));
                    (item, children)
                }
            };
            next_order.extend(row.1.iter().copied());
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
/// its [`Source`], its ancestors, then a resource.
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
    resources: Query<'w, 's, D, With<IsResource>>,
    links: Query<'w, 's, (Option<&'static ChildOf>, Option<&'static Source>), Allow<Disabled>>,
}

/// Where [`Nearest`] found its match.
enum Found {
    Entity(Entity),
    Resource,
}

impl<'s, D: IterQueryData + 'static> Nearest<'_, 's, D> {
    fn find(&self, start: Entity) -> Option<Found> {
        let mut current = Some(start);
        let mut steps = 0;
        while let Some(entity) = current {
            if self.entities.contains(entity) {
                return Some(Found::Entity(entity));
            }
            let Ok((parent, source)) = self.links.get(entity) else {
                break;
            };
            if let Some(&Source(source)) = source
                && self.entities.contains(source)
            {
                return Some(Found::Entity(source));
            }
            current = parent.map(ChildOf::parent);
            steps += 1;
            if steps > 4096 {
                break;
            }
        }
        (!self.resources.is_empty()).then_some(Found::Resource)
    }

    /// The nearest entity matching `D`, if it isn't a resource.
    #[must_use]
    pub fn entity(&self, start: Entity) -> Option<Entity> {
        match self.find(start)? {
            Found::Entity(entity) => Some(entity),
            Found::Resource => None,
        }
    }

    /// The nearest match for `start`, read-only.
    #[must_use]
    pub fn get(&self, start: Entity) -> Option<bevy::ecs::query::ROQueryItem<'_, 's, D>> {
        match self.find(start)? {
            Found::Entity(entity) => self.entities.get(entity).ok(),
            Found::Resource => self.resources.single().ok(),
        }
    }

    /// The nearest match for `start`.
    pub fn get_mut(&mut self, start: Entity) -> Option<D::Item<'_, 's>> {
        match self.find(start)? {
            Found::Entity(entity) => self.entities.get_mut(entity).ok(),
            Found::Resource => self.resources.single_mut().ok(),
        }
    }
}

#[doc(hidden)]
pub mod __private {
    pub use super::{
        each as __rsn_each, each_indexed as __rsn_each_indexed, handle as __rsn_handle,
        switch as __rsn_switch, watch as __rsn_watch,
    };
    pub use bevy::scene::{bsn, bsn_list};
}

// `bsn!` checks that its call site's source text starts with `bsn!`. These
// wrappers give it that call site when invoked from `rsn!` output. `bsn` and
// `bsn_list` resolve through `__private`, which `rsn!` output glob-imports.
// The body is passed as one braced group so punctuation keeps its spacing.

#[doc(hidden)]
#[macro_export]
macro_rules! __rsn_bsn {
    ($body:tt) => { bsn! $body };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __rsn_bsn_list {
    ($body:tt) => { bsn_list! $body };
}
