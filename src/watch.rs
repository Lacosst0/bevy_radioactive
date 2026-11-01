//! React to components and resources changing, with one closure.
//!
//! [`watch`] takes a closure whose parameter types say what it reads, what
//! makes it run again, and what it writes:
//!
//! ```ignore
//! use bevy_radioactive::watch::*;
//!
//! #[derive(Resource, Clone, Default)]
//! struct Count(i32);
//!
//! app.add_plugins(WatchPlugin);
//!
//! bsn! {
//!     Text("0")
//!     // Re-runs whenever `Count` changes, and returns a new `Text`.
//!     watch(|c: Ref<Count>| Text(c.0.to_string()))
//!     --
//!     Node
//!     // Edits a component of this entity in place.
//!     watch(|c: Ref<Count>, node: &mut Node| {
//!         node.display = if c.0 != 0 { Display::Flex } else { Display::None };
//!     })
//!     --
//!     #Player Health(100)
//!     --
//!     Text("")
//!     // Reads `Health` from another entity; `FontSize` is read, not tracked.
//!     watch_from(#Player, |hp: Ref<Health>, font: Res<FontSize>| {
//!         Text(format!("HP {} at {}px", hp.0, font.0))
//!     })
//! }
//! ```
//!
//! | In the closure | Re-runs it? | Meaning |
//! |---|---|---|
//! | `c: Ref<T>` | yes | `T` on the source entity, or resource `T` |
//! | `c: Option<Ref<T>>` | yes | the same, and `None` once `T` is removed or missing |
//! | `Res`, `ResMut`, `Query`, `Single`, `Commands`, `Local`, .. | no | ordinary Bevy system parameters |
//! | `c: &mut C` | no | component `C` of the entity that declared the watch |
//! | `e: Entity` | no | the source entity |
//! | returns a component or bundle | no | inserted on the entity that declared the watch |
//!
//! A watch runs once when it spawns, then again whenever a [`Ref`] input
//! changes: it is inserted, mutated, or (for `Option<Ref<T>>`) removed.
//! [`Ref::is_changed`] says which input changed since the last run. A run is
//! skipped while a plain `Ref<T>` or `&mut C` input is missing.
//!
//! The source of `Ref<T>` inputs is, in order: the entity given to
//! [`watch_from`] if it has a `T`, otherwise the entity that declared the
//! watch if it has a `T`, otherwise the resource `T`. Any `Component` or
//! `Resource` can be watched; nothing needs to be derived.
//!
//! [`watch_from`] with [`any()`] or [`any_with::<C>()`](any_with) runs once
//! per matching entity whose tracked components change, with that entity as
//! the source.
//!
//! For add, insert and removal of a component on the entity itself, use
//! Bevy's observers: `on(|_: On<Add<Health>>| ..)`, `On<Insert<Health>>` and
//! `On<Remove<Health>>`.
//!
//! Insertion and mutable access count as changes; use `set_if_neq` to skip
//! no-op writes. Watches run in [`WatchSystems`] in `PostUpdate`, before UI
//! layout, at most once per frame per source with the latest values. A watch
//! that changes another watched value is re-run in the same frame, for up to
//! 8 rounds.

use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    sync::{Arc, Mutex, PoisonError},
};

use bevy::{
    ecs::{
        bundle::Bundle,
        change_detection::Tick,
        component::{ComponentId, Mutable},
        entity_disabling::Disabled,
        lifecycle::HookContext,
        message::Message,
        query::{Allow, IterQueryData, QueryData, QueryFilter},
        resource::IsResource,
        schedule::ScheduleLabel,
        system::{SystemParam, SystemParamItem, SystemState},
        template::{EntityTemplate, Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::{HashMap, HashSet},
    prelude::*,
    scene::EmptySceneEffect,
};

/// Rounds of watches per frame, so chained reactions settle in one frame.
const MAX_ROUNDS: usize = 8;

/// Runs watches each frame. Required by [`watch`] and [`watch_from`].
#[derive(Default)]
pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tracked>()
            .init_resource::<WatchIndex>()
            .init_resource::<Changes>()
            .init_resource::<Jobs>()
            .init_schedule(Publish)
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

/// Watches from the app instead of a scene.
pub trait WatchAppExt {
    /// Runs `watch` like a scene's [`watch`], for as long as the app runs.
    /// It belongs to an otherwise empty entity, so `Ref<T>` inputs read
    /// resources. Adds [`WatchPlugin`] if needed.
    ///
    /// ```ignore
    /// app.watch(|score: Ref<Score>| info!("score is {}", score.0));
    /// ```
    fn watch<M>(&mut self, watch: impl IntoWatch<M>) -> &mut Self;

    /// Runs `watch` like a scene's [`watch_from`], for as long as the app
    /// runs.
    ///
    /// ```ignore
    /// app.watch_from(any(), |e: Entity, hp: Ref<Health>| info!("{e} has {} HP", hp.0));
    /// ```
    fn watch_from<M>(&mut self, source: impl WatchSource, watch: impl IntoWatch<M>) -> &mut Self;
}

impl WatchAppExt for App {
    fn watch<M>(&mut self, watch: impl IntoWatch<M>) -> &mut Self {
        add_app_watch(self, Source::Owner, watch);
        self
    }

    fn watch_from<M>(&mut self, source: impl WatchSource, watch: impl IntoWatch<M>) -> &mut Self {
        let source = source.into_source(self.world_mut());
        match source {
            Ok(source) => add_app_watch(self, source, watch),
            Err(error) => error!("watch_from: {error}"),
        }
        self
    }
}

fn add_app_watch<M>(app: &mut App, source: Source, watch: impl IntoWatch<M>) {
    if !app.is_plugin_added::<WatchPlugin>() {
        app.add_plugins(WatchPlugin);
    }
    let world = app.world_mut();
    let owner = world.spawn_empty().id();
    spawn_watcher(world, owner, source, watch);
}

// ---------------------------------------------------------------------------
// Scene templates
// ---------------------------------------------------------------------------

/// Runs `watch` on spawn and whenever one of its [`Ref`] inputs changes.
///
/// `Ref<T>` inputs come from the entity that declares the watch, or else
/// from resource `T`. See the [module docs](self) for every parameter kind.
///
/// ```ignore
/// Text("0")
/// watch(|c: Ref<Count>| Text(c.0.to_string()))
/// ```
pub fn watch<M, W>(watch: W) -> impl Template<Output = EmptySceneEffect>
where
    W: IntoWatch<M> + Clone,
{
    WatchTemplate {
        source: OwnerSource,
        watch,
        marker: PhantomData::<fn() -> M>,
    }
}

/// Like [`watch`], but `Ref<T>` inputs come from `source`: a `#Name`, an
/// [`Entity`], [`any()`] or [`any_with::<C>()`](any_with). Resources still
/// work as inputs.
///
/// ```ignore
/// Text("")
/// watch_from(#Player, |hp: Ref<Health>| Text(format!("HP {}", hp.0)))
///
/// watch_from(any_with::<Enemy>(), |e: Entity, hp: Ref<Health>, mut score: ResMut<Score>| {
///     if hp.0 == 0 { score.0 += 1; }
/// })
/// ```
pub fn watch_from<M, S, W>(source: S, watch: W) -> impl Template<Output = EmptySceneEffect>
where
    S: WatchSource,
    W: IntoWatch<M> + Clone,
{
    WatchTemplate {
        source,
        watch,
        marker: PhantomData::<fn() -> M>,
    }
}

struct WatchTemplate<S, W, M> {
    source: S,
    watch: W,
    marker: PhantomData<fn() -> M>,
}

impl<S, W, M> Template for WatchTemplate<S, W, M>
where
    S: WatchSource,
    W: IntoWatch<M> + Clone,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let source = self.source.build_source(context)?;
        let owner = context.entity.id();
        let watch = self.watch.clone();
        context.entity.world_scope(|world| {
            if !world.contains_resource::<WatchIndex>() {
                return Err(format!(
                    "watching with `{}` needs `WatchPlugin`; add it to the app",
                    type_name::<W>(),
                )
                .into());
            }
            spawn_watcher(world, owner, source, watch);
            Ok(EmptySceneEffect)
        })
    }

    fn clone_template(&self) -> Self {
        Self {
            source: self.source.clone_source(),
            watch: self.watch.clone(),
            marker: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// Where [`watch_from`] reads `Ref<T>` inputs from: a `#Name`, an
/// [`Entity`], [`any()`] or [`any_with::<C>()`](any_with).
pub trait WatchSource: Send + Sync + 'static {
    #[doc(hidden)]
    fn build_source(&self, context: &mut TemplateContext) -> Result<Source>;
    #[doc(hidden)]
    fn into_source(self, world: &mut World) -> Result<Source>;
    #[doc(hidden)]
    #[must_use]
    fn clone_source(&self) -> Self;
}

#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum Source {
    Owner,
    Entity(Entity),
    Any(fn(&mut World) -> Box<dyn AnyMatcher>),
}

struct OwnerSource;

impl WatchSource for OwnerSource {
    fn build_source(&self, _: &mut TemplateContext) -> Result<Source> {
        Ok(Source::Owner)
    }

    fn into_source(self, _: &mut World) -> Result<Source> {
        Ok(Source::Owner)
    }

    fn clone_source(&self) -> Self {
        Self
    }
}

impl WatchSource for Entity {
    fn build_source(&self, _: &mut TemplateContext) -> Result<Source> {
        Ok(Source::Entity(*self))
    }

    fn into_source(self, _: &mut World) -> Result<Source> {
        Ok(Source::Entity(self))
    }

    fn clone_source(&self) -> Self {
        *self
    }
}

impl WatchSource for EntityTemplate {
    fn build_source(&self, context: &mut TemplateContext) -> Result<Source> {
        Ok(Source::Entity(self.build_template(context)?))
    }

    fn into_source(self, _: &mut World) -> Result<Source> {
        match self {
            Self::Entity(entity) => Ok(Source::Entity(entity)),
            _ => Err("a `#Name` source only works inside a scene".into()),
        }
    }

    fn clone_source(&self) -> Self {
        self.clone_template()
    }
}

/// Every entity matching query filter `F`, as a [`watch_from`] source.
/// Made by [`any()`], [`any_with`] and [`any_matching`].
pub struct Any<F>(PhantomData<fn() -> F>);

/// Every entity, as a [`watch_from`] source: the watch runs once for each
/// entity whose tracked components change. Resource entities are left out.
#[must_use]
pub fn any() -> Any<()> {
    Any(PhantomData)
}

/// Every entity with component `C`, as a [`watch_from`] source.
#[must_use]
pub fn any_with<C: Component>() -> Any<With<C>> {
    Any(PhantomData)
}

/// Every entity matching query filter `F`, as a [`watch_from`] source.
/// Default query filters apply, so disabled entities are skipped unless `F`
/// allows them.
#[must_use]
pub fn any_matching<F: QueryFilter + 'static>() -> Any<F> {
    Any(PhantomData)
}

impl<F: QueryFilter + 'static> WatchSource for Any<F> {
    fn build_source(&self, _: &mut TemplateContext) -> Result<Source> {
        Ok(Source::Any(any_matcher::<F>))
    }

    fn into_source(self, _: &mut World) -> Result<Source> {
        Ok(Source::Any(any_matcher::<F>))
    }

    fn clone_source(&self) -> Self {
        Any(PhantomData)
    }
}

/// Tests and lists the entities an [`Any`] source selects.
#[doc(hidden)]
pub trait AnyMatcher: Send + Sync {
    fn contains(&mut self, world: &World, entity: Entity) -> bool;
    fn all(&mut self, world: &World) -> Vec<Entity>;
}

struct FilterMatcher<F: QueryFilter + 'static>(QueryState<Entity, (Without<IsResource>, F)>);

impl<F: QueryFilter + 'static> AnyMatcher for FilterMatcher<F> {
    fn contains(&mut self, world: &World, entity: Entity) -> bool {
        self.0.update_archetypes(world);
        self.0.contains(
            entity,
            world,
            world.last_change_tick(),
            world.read_change_tick(),
        )
    }

    fn all(&mut self, world: &World) -> Vec<Entity> {
        self.0.iter(world).collect()
    }
}

fn any_matcher<F: QueryFilter + 'static>(world: &mut World) -> Box<dyn AnyMatcher> {
    Box::new(FilterMatcher::<F>(QueryState::new(world)))
}

// ---------------------------------------------------------------------------
// Closure parameters
// ---------------------------------------------------------------------------

/// What one watch run sees: who declared it and where its inputs live.
#[doc(hidden)]
pub struct WatchContext {
    owner: Entity,
    source: Entity,
    inputs: Vec<(TypeId, Option<Entity>)>,
}

impl WatchContext {
    fn input<T: 'static>(&self) -> Option<Entity> {
        let id = TypeId::of::<T>();
        self.inputs
            .iter()
            .find_map(|&(input, entity)| (input == id).then_some(entity))
            .flatten()
    }
}

/// A type that is tracked by a watch: re-runs it when it changes.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct Input {
    type_id: TypeId,
    register: fn(&mut World) -> ComponentId,
}

impl Input {
    fn of<T: Component>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            register: register_input::<T>,
        }
    }
}

/// A [`watch`] closure parameter. Implemented for `Ref<T>`,
/// `Option<Ref<T>>`, `&mut C`, [`Entity`] and Bevy's system parameters.
///
/// For a custom [`SystemParam`] type, wrap it in [`Param`].
pub trait WatchParam {
    #[doc(hidden)]
    type Param: SystemParam + 'static;
    /// This type, for any lifetimes.
    #[doc(hidden)]
    type Item<'w, 's>;
    #[doc(hidden)]
    fn inputs(_: &mut Vec<Input>) {}
    #[doc(hidden)]
    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        context: &WatchContext,
    ) -> Option<Self::Item<'w, 's>>;
}

impl<T: Component> WatchParam for Ref<'_, T> {
    type Param = Query<'static, 'static, Ref<'static, T>, Allow<Disabled>>;
    type Item<'w, 's> = Ref<'w, T>;

    fn inputs(inputs: &mut Vec<Input>) {
        inputs.push(Input::of::<T>());
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        context: &WatchContext,
    ) -> Option<Self::Item<'w, 's>> {
        param.get_inner(context.input::<T>()?).ok()
    }
}

impl<T: Component> WatchParam for Option<Ref<'_, T>> {
    type Param = Query<'static, 'static, Ref<'static, T>, Allow<Disabled>>;
    type Item<'w, 's> = Option<Ref<'w, T>>;

    fn inputs(inputs: &mut Vec<Input>) {
        inputs.push(Input::of::<T>());
    }

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        context: &WatchContext,
    ) -> Option<Self::Item<'w, 's>> {
        Some(
            context
                .input::<T>()
                .and_then(|entity| param.get_inner(entity).ok()),
        )
    }
}

impl<C: Component<Mutability = Mutable>> WatchParam for &mut C {
    type Param = Query<'static, 'static, &'static mut C, Allow<Disabled>>;
    type Item<'w, 's> = &'w mut C;

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, Self::Param>,
        context: &WatchContext,
    ) -> Option<Self::Item<'w, 's>> {
        param.get_inner(context.owner).ok().map(Mut::into_inner)
    }
}

impl WatchParam for Entity {
    type Param = ();
    type Item<'w, 's> = Entity;

    fn fetch<'w, 's>(
        (): SystemParamItem<'w, 's, ()>,
        context: &WatchContext,
    ) -> Option<Self::Item<'w, 's>> {
        Some(context.source)
    }
}

/// Wraps a custom [`SystemParam`] so a [`watch`] closure can take it:
/// `|hp: Ref<Health>, p: Param<MyParam>| ..`. Bevy's own parameters don't
/// need it.
pub struct Param<'w, 's, P: SystemParam + 'static>(pub SystemParamItem<'w, 's, P>);

impl<P: SystemParam + 'static> WatchParam for Param<'_, '_, P> {
    type Param = P;
    type Item<'w, 's> = Param<'w, 's, P>;

    fn fetch<'w, 's>(
        param: SystemParamItem<'w, 's, P>,
        _: &WatchContext,
    ) -> Option<Self::Item<'w, 's>> {
        Some(Param(param))
    }
}

/// Implements [`WatchParam`] for Bevy system parameters, passed through as is.
macro_rules! system_params {
    ($([$($generics:tt)*] $ty:ty => $param:ty, $item:ty;)*) => {$(
        impl<$($generics)*> WatchParam for $ty {
            type Param = $param;
            type Item<'w, 's> = $item;

            fn fetch<'w, 's>(
                param: SystemParamItem<'w, 's, Self::Param>,
                _: &WatchContext,
            ) -> Option<Self::Item<'w, 's>> {
                Some(param)
            }
        }
    )*};
}

system_params! {
    ['a, T: Resource] Res<'a, T> => Res<'static, T>, Res<'w, T>;
    ['a, T: Resource + Component<Mutability = Mutable>] ResMut<'a, T> => ResMut<'static, T>, ResMut<'w, T>;
    ['a, T: Resource] Option<Res<'a, T>> => Option<Res<'static, T>>, Option<Res<'w, T>>;
    ['a, T: Resource + Component<Mutability = Mutable>] Option<ResMut<'a, T>> => Option<ResMut<'static, T>>, Option<ResMut<'w, T>>;
    ['a, 'b, D: QueryData + 'static, F: QueryFilter + 'static] Query<'a, 'b, D, F>
        => Query<'static, 'static, D, F>, Query<'w, 's, D, F>;
    ['a, 'b, D: IterQueryData + 'static, F: QueryFilter + 'static] Single<'a, 'b, D, F>
        => Single<'static, 'static, D, F>, Single<'w, 's, D, F>;
    ['a, 'b, D: IterQueryData + 'static, F: QueryFilter + 'static] Option<Single<'a, 'b, D, F>>
        => Option<Single<'static, 'static, D, F>>, Option<Single<'w, 's, D, F>>;
    ['a, 'b, D: QueryData + 'static, F: QueryFilter + 'static] Populated<'a, 'b, D, F>
        => Populated<'static, 'static, D, F>, Populated<'w, 's, D, F>;
    ['a, 'b] Commands<'a, 'b> => Commands<'static, 'static>, Commands<'w, 's>;
    ['a, T: FromWorld + Send + 'static] Local<'a, T> => Local<'static, T>, Local<'s, T>;
    ['a, 'b, E: Message] MessageReader<'a, 'b, E> => MessageReader<'static, 'static, E>, MessageReader<'w, 's, E>;
    ['a, E: Message] MessageWriter<'a, E> => MessageWriter<'static, E>, MessageWriter<'w, E>;
}

// ---------------------------------------------------------------------------
// Closures
// ---------------------------------------------------------------------------

/// A closure that [`watch`] can run: any `Fn` whose parameters are
/// [`WatchParam`]s and which returns `()` or a component or bundle to insert.
pub trait IntoWatch<M>: Send + Sync + 'static {
    #[doc(hidden)]
    fn inputs() -> Vec<Input>;
    #[doc(hidden)]
    fn into_runner(self) -> Box<dyn WatchRunner>;
}

/// A watch closure with its system state.
#[doc(hidden)]
pub trait WatchRunner: Send + Sync {
    fn run(&mut self, world: &mut World, context: &WatchContext);
}

struct Runner<W, M, P: SystemParam + 'static> {
    watch: W,
    state: Option<SystemState<P>>,
    marker: PhantomData<fn() -> M>,
}

fn insert_output<R: Bundle>(world: &mut World, owner: Entity, output: R) {
    if TypeId::of::<R>() == TypeId::of::<()>() {
        return;
    }
    if let Ok(mut entity) = world.get_entity_mut(owner) {
        entity.insert(output);
    }
}

macro_rules! impl_into_watch {
    ($($param:ident),*) => {
        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, R, $($param: WatchParam + 'static),*> IntoWatch<fn($($param,)*) -> R> for W
        where
            W: Fn($($param),*) -> R
                + Fn($($param::Item<'_, '_>),*) -> R
                + Send + Sync + 'static,
            R: Bundle,
        {
            fn inputs() -> Vec<Input> {
                #[allow(unused_mut, reason = "empty for closures without parameters")]
                let mut inputs = Vec::new();
                $($param::inputs(&mut inputs);)*
                inputs
            }

            fn into_runner(self) -> Box<dyn WatchRunner> {
                Box::new(Runner::<W, fn($($param,)*) -> R, ($($param::Param,)*)> {
                    watch: self,
                    state: None,
                    marker: PhantomData,
                })
            }
        }

        #[allow(non_snake_case, reason = "the parameter names are type names")]
        impl<W, R, $($param: WatchParam + 'static),*> WatchRunner for Runner<W, fn($($param,)*) -> R, ($($param::Param,)*)>
        where
            W: Fn($($param),*) -> R
                + Fn($($param::Item<'_, '_>),*) -> R
                + Send + Sync + 'static,
            R: Bundle,
        {
            fn run(&mut self, world: &mut World, context: &WatchContext) {
                // Like Bevy's function systems: helps rustc see `W` is a function.
                #[allow(clippy::too_many_arguments, reason = "one per closure parameter")]
                fn call<R, $($param),*>(watch: impl Fn($($param),*) -> R, $($param: $param),*) -> R {
                    watch($($param),*)
                }
                let state = self.state.get_or_insert_with(|| SystemState::new(world));
                let output = {
                    let Ok(($($param,)*)) = state.get_mut(world) else {
                        return;
                    };
                    $(let Some($param) = $param::fetch($param, context) else {
                        return;
                    };)*
                    call(&self.watch, $($param),*)
                };
                state.apply(world);
                insert_output(world, context.owner, output);
            }
        }
    };
}

impl_into_watch!();
impl_into_watch!(P0);
impl_into_watch!(P0, P1);
impl_into_watch!(P0, P1, P2);
impl_into_watch!(P0, P1, P2, P3);
impl_into_watch!(P0, P1, P2, P3, P4);
impl_into_watch!(P0, P1, P2, P3, P4, P5);
impl_into_watch!(P0, P1, P2, P3, P4, P5, P6);
impl_into_watch!(P0, P1, P2, P3, P4, P5, P6, P7);

// ---------------------------------------------------------------------------
// Watchers
// ---------------------------------------------------------------------------

/// One watch: an entity owned by the entity that declared it.
#[derive(Component)]
#[component(on_add = add_watcher, on_remove = remove_watcher)]
struct Watcher {
    owner: Entity,
    source: WatcherSource,
    inputs: Vec<(TypeId, ComponentId)>,
    runner: Arc<Mutex<Box<dyn WatchRunner>>>,
    /// When it last ran, per source entity.
    last_runs: Mutex<HashMap<Entity, Tick>>,
}

enum WatcherSource {
    Owner,
    Entity(Entity),
    Any(Mutex<Box<dyn AnyMatcher>>),
}

/// Owns watcher entities, so they despawn with the entity that declared them.
#[derive(Component)]
#[relationship(relationship_target = Watches)]
struct WatchOf(Entity);

#[derive(Component)]
#[relationship_target(relationship = WatchOf, linked_spawn)]
struct Watches(Vec<Entity>);

/// Watcher entities by the types they track.
#[derive(Default, Resource)]
struct WatchIndex(HashMap<TypeId, Vec<Entity>>);

fn add_watcher(mut world: DeferredWorld, context: HookContext) {
    let Some(watcher) = world.get::<Watcher>(context.entity) else {
        return;
    };
    let types: Vec<TypeId> = watcher.inputs.iter().map(|&(id, _)| id).collect();
    if let Some(mut index) = world.get_resource_mut::<WatchIndex>() {
        for id in types {
            index.0.entry(id).or_default().push(context.entity);
        }
    }
}

fn remove_watcher(mut world: DeferredWorld, context: HookContext) {
    let Some(watcher) = world.get::<Watcher>(context.entity) else {
        return;
    };
    let types: Vec<TypeId> = watcher.inputs.iter().map(|&(id, _)| id).collect();
    if let Some(mut index) = world.get_resource_mut::<WatchIndex>() {
        for id in types {
            if let Some(list) = index.0.get_mut(&id) {
                list.retain(|&entity| entity != context.entity);
            }
        }
    }
}

fn spawn_watcher<M, W: IntoWatch<M>>(world: &mut World, owner: Entity, source: Source, watch: W) {
    let mut inputs = Vec::new();
    for input in W::inputs() {
        let id = (input.register)(world);
        if !inputs.iter().any(|&(type_id, _)| type_id == input.type_id) {
            inputs.push((input.type_id, id));
        }
    }
    let source = match source {
        Source::Owner => WatcherSource::Owner,
        Source::Entity(entity) => WatcherSource::Entity(entity),
        Source::Any(matcher) => WatcherSource::Any(Mutex::new(matcher(world))),
    };
    let watcher = world
        .spawn((
            Watcher {
                owner,
                source,
                inputs,
                runner: Arc::new(Mutex::new(watch.into_runner())),
                last_runs: Mutex::default(),
            },
            WatchOf(owner),
        ))
        .id();
    world.resource_mut::<Jobs>().0.push(Job::Spawned(watcher));
}

/// The entity whose `component` a watch reads, starting from `base`.
fn resolve(world: &World, base: Entity, component: ComponentId) -> Option<Entity> {
    if world
        .get_entity(base)
        .is_ok_and(|entity| entity.contains_id(component))
    {
        Some(base)
    } else {
        world.resource_entities().get(component)
    }
}

/// Queues runs of every watch that reads `T` (`type_id`) from `changed`.
/// Removals run unconditionally; changes only if the watch hasn't seen them.
fn route(world: &World, type_id: TypeId, changed: Entity, removed: bool, jobs: &mut Vec<Job>) {
    let Some(watchers) = world.resource::<WatchIndex>().0.get(&type_id) else {
        return;
    };
    for &entity in watchers {
        let Some(watcher) = world.get::<Watcher>(entity) else {
            continue;
        };
        let Some(&(_, component)) = watcher.inputs.iter().find(|(id, _)| *id == type_id) else {
            continue;
        };
        match &watcher.source {
            WatcherSource::Owner | WatcherSource::Entity(_) => {
                let base = match watcher.source {
                    WatcherSource::Entity(source) => source,
                    _ => watcher.owner,
                };
                if resolve(world, base, component) == Some(changed) {
                    jobs.push(Job::Run(entity, None, removed));
                }
            }
            WatcherSource::Any(matcher) => {
                let mut matcher = matcher.lock().unwrap_or_else(PoisonError::into_inner);
                if matcher.contains(world, changed) {
                    jobs.push(Job::Run(entity, Some(changed), removed));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Publish;

/// Watched types, and change detectors to add to [`Publish`] next round.
#[derive(Default, Resource)]
struct Tracked {
    types: HashSet<TypeId>,
    pending: Vec<fn(&mut Schedule)>,
}

/// Changed values found this round: `(type, entity)`.
#[derive(Default, Resource)]
struct Changes(Mutex<Vec<(TypeId, Entity)>>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Job {
    /// A watch just spawned: run it for its current values.
    Spawned(Entity),
    /// Run a watch, for one source entity of an [`Any`] source. `true` runs
    /// it even if it has already seen its inputs' latest changes.
    Run(Entity, Option<Entity>, bool),
}

/// Watch runs waiting for the next round.
#[derive(Default, Resource)]
struct Jobs(Vec<Job>);

/// Starts detecting changes to `T`. Safe while scenes spawn mid-round: the
/// detector joins the schedule before the next round.
fn register_input<T: Component>(world: &mut World) -> ComponentId {
    let id = world.register_component::<T>();
    let mut tracked = world.resource_mut::<Tracked>();
    if tracked.types.insert(TypeId::of::<T>()) {
        tracked.pending.push(|schedule| {
            schedule.add_systems(detect_changes::<T>);
        });
        world.add_observer(detect_removal::<T>);
    }
    id
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are passed by value"
)]
fn detect_changes<T: Component>(
    entities: Query<Entity, (Changed<T>, Allow<Disabled>)>,
    changes: Res<Changes>,
) {
    let mut changes = changes.0.lock().unwrap_or_else(PoisonError::into_inner);
    for entity in &entities {
        changes.push((TypeId::of::<T>(), entity));
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn detect_removal<T: Component>(event: On<Remove<T>>, mut world: DeferredWorld) {
    // `Remove` observers run before the value goes, so sources still resolve
    // to the entity as it was.
    let mut jobs = Vec::new();
    route(&world, TypeId::of::<T>(), event.entity, true, &mut jobs);
    if !jobs.is_empty() {
        world.resource_mut::<Jobs>().0.extend(jobs);
    }
}

fn run_watches(world: &mut World) {
    for _ in 0..MAX_ROUNDS {
        let pending = std::mem::take(&mut world.resource_mut::<Tracked>().pending);
        if !pending.is_empty() {
            world.schedule_scope(Publish, |_, schedule| {
                for add in pending {
                    add(schedule);
                }
            });
        }
        let _ = world.try_run_schedule(Publish);
        let changes = std::mem::take(
            &mut *world
                .resource::<Changes>()
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        let mut jobs = std::mem::take(&mut world.resource_mut::<Jobs>().0);
        for (type_id, entity) in changes {
            route(world, type_id, entity, false, &mut jobs);
        }
        if jobs.is_empty() {
            return;
        }
        let mut seen = HashSet::new();
        for job in expand(world, jobs) {
            if seen.insert(job) {
                run_job(world, job);
            }
        }
        world.flush();
    }
    warn!(
        "watch: values still changing after {MAX_ROUNDS} rounds; the rest runs next frame. A \
         watch may be writing a value it reads."
    );
}

/// Turns [`Job::Spawned`] into runs: one, or one per entity of an [`Any`]
/// source that has one of the watched components.
fn expand(world: &World, jobs: Vec<Job>) -> Vec<Job> {
    let mut runs = Vec::with_capacity(jobs.len());
    for job in jobs {
        let Job::Spawned(entity) = job else {
            runs.push(job);
            continue;
        };
        let Some(watcher) = world.get::<Watcher>(entity) else {
            continue;
        };
        let WatcherSource::Any(matcher) = &watcher.source else {
            runs.push(Job::Run(entity, None, true));
            continue;
        };
        let mut matcher = matcher.lock().unwrap_or_else(PoisonError::into_inner);
        for source in matcher.all(world) {
            let has_input = world.get_entity(source).is_ok_and(|source| {
                watcher
                    .inputs
                    .iter()
                    .any(|&(_, component)| source.contains_id(component))
            });
            if has_input {
                runs.push(Job::Run(entity, Some(source), true));
            }
        }
    }
    runs
}

fn run_job(world: &mut World, job: Job) {
    let Job::Run(entity, source, force) = job else {
        return;
    };
    let Some(watcher) = world.get::<Watcher>(entity) else {
        return;
    };
    let owner = watcher.owner;
    let base = match (&watcher.source, source) {
        (_, Some(source)) | (&WatcherSource::Entity(source), None) => source,
        _ => owner,
    };
    let context = WatchContext {
        owner,
        source: base,
        inputs: watcher
            .inputs
            .iter()
            .map(|&(type_id, component)| (type_id, resolve(world, base, component)))
            .collect(),
    };
    let this_run = world.read_change_tick();
    {
        let mut last_runs = watcher
            .last_runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !force
            && let Some(&last_run) = last_runs.get(&base)
            && !inputs_changed(world, &watcher.inputs, &context, last_run, this_run)
        {
            // Already run since these changes, e.g. later in the same round.
            return;
        }
        last_runs.insert(base, this_run);
    }
    let runner = Arc::clone(&watcher.runner);
    runner
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .run(world, &context);
    if source.is_some()
        && world.get_entity(base).is_err()
        && let Some(watcher) = world.get::<Watcher>(entity)
    {
        // An `Any` source despawned: forget it.
        watcher
            .last_runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&base);
    }
}

/// Whether any input changed after `last_run`.
fn inputs_changed(
    world: &World,
    inputs: &[(TypeId, ComponentId)],
    context: &WatchContext,
    last_run: Tick,
    this_run: Tick,
) -> bool {
    inputs
        .iter()
        .zip(&context.inputs)
        .any(|(&(_, component), &(_, entity))| {
            entity
                .and_then(|entity| world.get_entity(entity).ok())
                .and_then(|entity| entity.get_change_ticks_by_id(component))
                .is_none_or(|ticks| ticks.is_changed(last_run, this_run))
        })
}
