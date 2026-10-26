use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bevy::{
    ecs::{
        component::Mutable,
        entity_disabling::Disabled,
        event::{EntityEvent, EntityTrigger, Event},
        query::Allow,
        schedule::ScheduleLabel,
        system::IntoObserverSystem,
        template::{Template, TemplateContext},
    },
    platform::collections::HashSet,
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

use crate::subscription::{
    Replay, Source, Subscribe, Subscribers, Subscription, SubscriptionEvent, forwarder,
    install_subscription, register_routed, route, spawn_subscriptions,
};

/// BSN template returned by [`watch`].
pub type Watch<T, S, I, M> = Subscribe<ValueChanged<T>, S, I, M>;

/// BSN template returned by [`watch_removed`].
pub type WatchRemoval<T, S, I, M> = Subscribe<ValueRemoved<T>, S, I, M>;

/// Scheduling set containing the reactive runtime.
///
/// The runtime runs in [`PostUpdate`]. With the `ui` feature it is ordered
/// before `UiSystems::Prepare`, so text and style written by reactions are
/// laid out in the same frame. Order systems that mutate watched values
/// in `PostUpdate` `.before(ReactiveSet::Publish)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum ReactiveSet {
    /// Publishes [`ValueChanged`] events and runs effects, components and
    /// regions until stable.
    Publish,
    /// Runs after [`Publish`](Self::Publish) has settled, and with the `ui`
    /// feature still before UI layout. Put systems that read reactive
    /// output in this set.
    Settled,
}

/// Schedule containing one publisher per tracked type.
///
/// It is run repeatedly by the reactive runtime inside [`ReactiveSet::Publish`].
#[derive(ScheduleLabel, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ReactivePublish;

/// Runtime configuration inserted by [`RadioactivePlugin`].
#[derive(Resource, Clone, Copy, Debug)]
pub struct ReactiveSettings {
    /// Maximum publish passes per frame.
    ///
    /// A reaction that writes another tracked value is published in the next
    /// pass of the same frame. Passes stop when nothing changes or this limit
    /// is reached; remaining changes are published on the next frame, a
    /// warning is logged once, and [`ReactiveDiagnostics::exhausted_frames`]
    /// is incremented. Nested components and regions do not use up passes.
    pub max_passes: usize,
}

impl Default for ReactiveSettings {
    fn default() -> Self {
        Self { max_passes: 8 }
    }
}

/// Installs the reactive runtime. Added automatically by [`ReactiveAppExt::track`].
#[derive(Default)]
pub struct RadioactivePlugin;

impl Plugin for RadioactivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReactiveSettings>()
            .init_resource::<ReactiveRegistry>()
            .init_resource::<PublishCounter>()
            .init_resource::<ReactiveDiagnostics>()
            .init_resource::<PendingReplays>()
            .init_schedule(ReactivePublish)
            .add_systems(PostUpdate, run_reactive.in_set(ReactiveSet::Publish));
        crate::runtime::init(app.world_mut());
        app.configure_sets(PostUpdate, ReactiveSet::Settled.after(ReactiveSet::Publish));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            (ReactiveSet::Publish, ReactiveSet::Settled).before(bevy::ui::UiSystems::Prepare),
        );
    }
}

/// Tracked value types, and publishers waiting to be added to
/// [`ReactivePublish`] at the start of the next pass.
#[derive(Default, Resource)]
struct ReactiveRegistry {
    tracked: HashSet<TypeId>,
    /// `ValueChanged<T>` and `ValueRemoved<T>` of every tracked `T`.
    events: HashSet<TypeId>,
    pending: Vec<fn(&mut Schedule)>,
}

/// Registers `T` once. Safe at any time, including while scenes spawn
/// during a reactive pass: the publisher joins the schedule between passes.
fn register<T: Component + Clone>(world: &mut World) {
    let mut registry = world.resource_mut::<ReactiveRegistry>();
    if !registry.tracked.insert(TypeId::of::<T>()) {
        return;
    }
    registry.events.insert(TypeId::of::<ValueChanged<T>>());
    registry.events.insert(TypeId::of::<ValueRemoved<T>>());
    registry.pending.push(|schedule| {
        schedule.add_systems(publish::<T>);
    });
    install_subscription::<ValueChanged<T>>(world);
    register_routed::<ValueRemoved<T>>(world);
    world.add_observer(publish_removed::<T>);
}

fn add_pending_publishers(world: &mut World) {
    let pending = std::mem::take(&mut world.resource_mut::<ReactiveRegistry>().pending);
    if !pending.is_empty() {
        world.schedule_scope(ReactivePublish, |_, schedule| {
            for add in pending {
                add(schedule);
            }
        });
    }
}

/// Replays queued by new subscriptions, run at the start of the next pass.
#[derive(Default, Resource)]
struct PendingReplays(Vec<Box<dyn FnOnce(&mut World) + Send + Sync>>);

/// Queues a subscription replay for the start of the next reactive pass.
pub(crate) fn defer_replay(
    world: &mut World,
    replay: impl FnOnce(&mut World) + Send + Sync + 'static,
) {
    if let Some(mut pending) = world.get_resource_mut::<PendingReplays>() {
        pending.0.push(Box::new(replay));
    } else {
        world.commands().queue(replay);
    }
}

fn run_replays(world: &mut World) -> usize {
    let replays = std::mem::take(&mut world.resource_mut::<PendingReplays>().0);
    let count = replays.len();
    for replay in replays {
        replay(world);
    }
    if count > 0 {
        world.flush();
    }
    count
}

/// Counts publications in the current pass.
#[derive(Default, Resource)]
struct PublishCounter(AtomicUsize);

/// Counters describing the reactive runtime, inserted by [`RadioactivePlugin`].
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ReactiveDiagnostics {
    /// Passes run in the last frame.
    pub passes: usize,
    /// Effects, component renders and region updates run in the last frame.
    pub effects_run: usize,
    /// [`ValueChanged`] events published in the last frame.
    pub publications: usize,
    /// Frames that stopped at [`ReactiveSettings::max_passes`] with work left.
    pub exhausted_frames: u64,
}

fn run_reactive(world: &mut World) {
    let max_passes = world.resource::<ReactiveSettings>().max_passes.max(1);
    let mut frame = ReactiveDiagnostics::default();
    let mut settled = false;
    for _ in 0..max_passes {
        add_pending_publishers(world);
        let replayed = run_replays(world);
        world
            .resource::<PublishCounter>()
            .0
            .store(0, Ordering::Relaxed);
        let _ = world.try_run_schedule(ReactivePublish);
        world.flush();
        let published = world.resource::<PublishCounter>().0.load(Ordering::Relaxed);
        let ran = crate::runtime::run_effects(world);
        frame.passes += 1;
        frame.publications += published;
        frame.effects_run += ran;
        if published == 0 && ran == 0 && replayed == 0 {
            settled = true;
            break;
        }
    }
    let mut diagnostics = world.resource_mut::<ReactiveDiagnostics>();
    frame.exhausted_frames = diagnostics.exhausted_frames + u64::from(!settled);
    *diagnostics = frame;
    if !settled {
        warn_once!(
            "bevy_radioactive: reached `ReactiveSettings::max_passes` ({max_passes}) with              reactive work left; it continues next frame. This usually means a feedback              cycle, such as an effect that writes a value it reads."
        );
    }
}

/// A snapshot published after a tracked value is inserted or mutably
/// dereferenced.
///
/// The event targets the entity that owns the value; for a resource, that is
/// its singleton resource entity. Global observers receive this event once;
/// BSN watchers receive a separate, source-preserving [`Subscription`].
pub struct ValueChanged<T: Component> {
    source: Entity,
    value: Arc<T>,
}

impl<T: Component> ValueChanged<T> {
    /// Returns the entity that owns the changed value.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.source
    }

    /// Returns the captured value.
    #[must_use]
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Clones the shared snapshot without cloning `T`.
    #[must_use]
    pub fn snapshot(&self) -> Arc<T> {
        Arc::clone(&self.value)
    }
}

impl<T: Component> Clone for ValueChanged<T> {
    fn clone(&self) -> Self {
        Self {
            source: self.source,
            value: Arc::clone(&self.value),
        }
    }
}

impl<T: Component> Deref for ValueChanged<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Component> Event for ValueChanged<T> {
    type Trigger<'a> = EntityTrigger;
}

impl<T: Component> EntityEvent for ValueChanged<T> {
    fn event_target(&self) -> Entity {
        self.source
    }
}

/// Published when a tracked value is removed from its source, including when
/// the source is despawned or a resource is removed.
///
/// Routed while the source still exists, before its relationships are torn
/// down, so despawning a source reaches its subscribers. Handlers run at the
/// next command flush, after the removal or despawn has finished.
pub struct ValueRemoved<T: Component> {
    source: Entity,
    marker: PhantomData<fn() -> T>,
}

impl<T: Component> ValueRemoved<T> {
    /// Returns the entity that lost the value.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.source
    }
}

impl<T: Component> Clone for ValueRemoved<T> {
    fn clone(&self) -> Self {
        Self {
            source: self.source,
            marker: PhantomData,
        }
    }
}

impl<T: Component> Event for ValueRemoved<T> {
    type Trigger<'a> = EntityTrigger;
}

impl<T: Component> EntityEvent for ValueRemoved<T> {
    fn event_target(&self) -> Entity {
        self.source
    }
}

/// Observer input used by [`watch`].
///
/// Dereferences through [`ValueChanged`] to the captured `T`. Use
/// `target()` for the watching entity and `source()` for the entity that
/// owns `T`.
pub type Reaction<'w, 't, T> = Subscription<'w, 't, ValueChanged<T>>;

/// Observer input used by [`watch_removed`].
pub type Removal<'w, 't, T> = Subscription<'w, 't, ValueRemoved<T>>;

/// App construction helpers for registering reactive value publishers.
///
/// Resources are singleton components, so one API covers both.
pub trait ReactiveAppExt {
    /// Publishes changes and removals of `T` on every entity that has it,
    /// including the singleton entity of resource `T`.
    ///
    /// Insertion and mutable dereferencing count as changes. Prefer Bevy's
    /// `set_if_neq` to suppress no-op notifications. Registration is
    /// idempotent and adds [`RadioactivePlugin`] if needed.
    ///
    /// Optional for scenes: [`watch`], [`bind`] and [`watch_removed`] track their
    /// type on first use. Call it for types only observed globally, or to
    /// register everything up front.
    fn track<T: Component + Clone>(&mut self) -> &mut Self;

    /// Tracks `T` and adds one global [`ValueChanged`] observer.
    fn observe_value<T: Component + Clone, M>(
        &mut self,
        observer: impl IntoObserverSystem<ValueChanged<T>, M>,
    ) -> &mut Self;
}

impl ReactiveAppExt for App {
    fn track<T: Component + Clone>(&mut self) -> &mut Self {
        if !self.is_plugin_added::<RadioactivePlugin>() {
            self.add_plugins(RadioactivePlugin);
        }
        register::<T>(self.world_mut());
        self
    }

    fn observe_value<T: Component + Clone, M>(
        &mut self,
        observer: impl IntoObserverSystem<ValueChanged<T>, M>,
    ) -> &mut Self {
        self.track::<T>().add_observer(observer)
    }
}

/// Warns when a raw [`subscribe_to`](crate::subscribe_to) names a value
/// event whose type is not tracked, which would never be delivered.
pub(crate) fn warn_if_untracked<E: 'static>(world: &World) {
    if !type_name::<E>().starts_with("bevy_radioactive::reactivity::Value") {
        return;
    }
    let tracked = world
        .get_resource::<ReactiveRegistry>()
        .is_some_and(|registry| registry.events.contains(&TypeId::of::<E>()));
    if !tracked {
        warn_once!(
            "bevy_radioactive: `subscribe_to::<{}>` is delivered only once its value type is              tracked; use `watch`/`watch_removed`, or call `app.track::<T>()`",
            type_name::<E>(),
        );
    }
}

/// Tracks `T` on first use, so scenes need no app-time `track` call.
fn ensure_tracked<T: Component + Clone>(context: &mut TemplateContext) -> Result {
    context.entity.world_scope(|world| {
        if !world.contains_resource::<ReactiveRegistry>() {
            return Err(format!(
                "watching `{}` needs the reactive runtime; add `RadioactivePlugin` to the app",
                type_name::<T>(),
            )
            .into());
        }
        register::<T>(world);
        Ok(())
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are passed by value"
)]
fn publish<T: Component + Clone>(
    values: Query<(Entity, &T, Has<Subscribers<ValueChanged<T>>>), (Changed<T>, Allow<Disabled>)>,
    world: &World,
    counter: Res<PublishCounter>,
    mut commands: Commands,
) {
    if values.is_empty() {
        return;
    }
    // Skip the snapshot for sources nobody listens to: no subscription, no
    // global observer and no observer on the source itself.
    let observers = world
        .event_key::<ValueChanged<T>>()
        .and_then(|key| world.observers().try_get_observers(key));
    let forwarder = forwarder::<ValueChanged<T>>(world);
    let global = observers.is_some_and(|observers| {
        observers
            .global_observers()
            .keys()
            .any(|&observer| Some(observer) != forwarder)
    });
    for (source, value, subscribed) in &values {
        let observed = global
            || subscribed
            || observers
                .is_some_and(|observers| observers.entity_observers().contains_key(&source));
        if !observed {
            continue;
        }
        counter.0.fetch_add(1, Ordering::Relaxed);
        commands.trigger(ValueChanged {
            source,
            value: Arc::new(value.clone()),
        });
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn publish_removed<T: Component>(event: On<Remove<T>>, mut world: bevy::ecs::world::DeferredWorld) {
    let removed = ValueRemoved::<T> {
        source: event.entity,
        marker: PhantomData,
    };
    route(world.reborrow(), removed.source, &removed);
    world.trigger(removed);
}

/// Source marker for the singleton entity of resource `T`.
///
/// `watch(Singleton, handler)` watches resource `T` exactly like
/// `watch(#Entity, handler)` watches a component. The resource must exist
/// when the scene resolves.
#[derive(Clone, Copy, Debug, Default)]
pub struct Singleton;

fn resolve_singleton<T: Resource>(context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
    let exists = context
        .entity
        .world_scope(|world| world.contains_resource::<T>());
    let entity = context
        .resource_entity::<T>()
        .filter(|_| exists)
        .ok_or_else(|| {
            format!(
                "resource `{}` must exist before resolving a `Singleton` source",
                type_name::<T>(),
            )
        })?;
    out.push(entity);
    Ok(())
}

impl<T: Resource> Source<ValueChanged<T>> for Singleton {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        resolve_singleton::<T>(context, out)
    }

    fn clone_source(&self) -> Self {
        Self
    }
}

impl<T: Resource> Source<ValueRemoved<T>> for Singleton {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        resolve_singleton::<T>(context, out)
    }

    fn clone_source(&self) -> Self {
        Self
    }
}

/// Watches tracked `T` on `source` from the current BSN entity.
///
/// `source` is a `#Name`, an [`Entity`], [`Singleton`] for a resource, or a
/// list such as `[#Player, #Enemy]`. One entity may declare any number of
/// watches. By default only changes are delivered; call
/// [`immediate`](Subscribe::immediate) to also receive each source's current
/// value once the scene has spawned. Changes made while the scene is spawned
/// are published in the same frame.
pub fn watch<T, S, I, M>(source: S, observer: I) -> Watch<T, S, I, M>
where
    T: Component + Clone,
    S: Source<ValueChanged<T>>,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, M>,
{
    Subscribe::new(source, observer).with_check(ensure_tracked::<T>)
}

impl<T: Component + Clone, S, I, M> Subscribe<ValueChanged<T>, S, I, M> {
    /// Also delivers each source's current value once, after the scene has
    /// spawned, like Vue's `immediate` watchers.
    #[must_use]
    pub fn immediate(mut self) -> Self {
        self.replay = Some(current_value::<T>);
        self
    }
}

/// Like [`watch`], and also delivers each source's current value once,
/// after the scene has spawned.
///
/// Same as `watch(..).immediate()`. Use this form with `#Name` sources:
/// `bsn!` resolves `#Name` only in direct arguments of a call, not in a
/// call followed by a method.
pub fn watch_immediate<T, S, I, M>(source: S, observer: I) -> Watch<T, S, I, M>
where
    T: Component + Clone,
    S: Source<ValueChanged<T>>,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, M>,
{
    watch(source, observer).immediate()
}

fn current_value<T: Component + Clone>(world: &World, source: Entity) -> Option<ValueChanged<T>> {
    let value = world.get::<T>(source)?;
    Some(ValueChanged {
        source,
        value: Arc::new(value.clone()),
    })
}

fn missing_value<T: Component>(world: &World, source: Entity) -> Option<ValueRemoved<T>> {
    world.get::<T>(source).is_none().then_some(ValueRemoved {
        source,
        marker: PhantomData,
    })
}

/// Observes removal of tracked `T` from `source`, including despawn.
///
/// ```ignore
/// watch_removed(#Enemy, |removal: Removal<Health>, mut q: Query<&mut Text>| {
///     if let Ok(mut text) = q.get_mut(removal.target()) {
///         **text = "Enemy defeated".into();
///     }
/// })
/// ```
pub fn watch_removed<T, S, I, M>(source: S, observer: I) -> WatchRemoval<T, S, I, M>
where
    T: Component + Clone,
    S: Source<ValueRemoved<T>>,
    I: IntoObserverSystem<SubscriptionEvent<ValueRemoved<T>>, M>,
{
    Subscribe::new(source, observer).with_check(ensure_tracked::<T>)
}

/// Old name of [`watch_removed`].
#[deprecated(since = "0.5.0", note = "renamed to `watch_removed`")]
pub fn on_removed<T, S, I, M>(source: S, observer: I) -> WatchRemoval<T, S, I, M>
where
    T: Component + Clone,
    S: Source<ValueRemoved<T>>,
    I: IntoObserverSystem<SubscriptionEvent<ValueRemoved<T>>, M>,
{
    watch_removed(source, observer)
}

/// Writes each published `T` from `source` into component `C` on the current
/// BSN entity.
///
/// ```ignore
/// Text::default()
/// bind(#Player, |hp: &Health, text: &mut Text| **text = hp.current.to_string())
/// ```
///
/// Like [`watch`], `source` may be a `#Name`, [`Singleton`], or a list. The
/// current value of each source is written once the scene has spawned, so
/// `C` needs no hand-written initial value; call
/// [`deferred`](Bind::deferred) to write only on changes.
pub fn bind<T, C, S, F>(source: S, write: F) -> Bind<T, C, S, F, ()>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
{
    Bind {
        source,
        write,
        or_else: (),
        replay: true,
        marker: PhantomData,
    }
}

/// Like [`bind`], without writing the current value when the scene spawns.
///
/// Same as `bind(..).deferred()`, in a form that accepts `#Name` sources
/// (see [`watch_immediate`]).
pub fn bind_deferred<T, C, S, F>(source: S, write: F) -> Bind<T, C, S, F, ()>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
{
    bind(source, write).deferred()
}

/// Like [`bind`], and also writes `C` with `or_else` when `T` is removed
/// from a source or the source is despawned, or is missing when the scene
/// spawns.
///
/// ```ignore
/// Text::default()
/// bind_or_else(
///     #Player,
///     |hp: &Health, text: &mut Text| **text = hp.current.to_string(),
///     |text: &mut Text| **text = "DEAD".into(),
/// )
/// ```
pub fn bind_or_else<T, C, S, F, R>(source: S, write: F, or_else: R) -> Bind<T, C, S, F, OrElse<R>>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: Fn(&mut C) + Clone + Send + Sync + 'static,
{
    Bind {
        source,
        write,
        or_else: OrElse(or_else),
        replay: true,
        marker: PhantomData,
    }
}

/// Old name of [`bind_or_else`].
#[deprecated(since = "0.5.0", note = "renamed to `bind_or_else`")]
pub fn bind_or<T, C, S, F, R>(source: S, write: F, or_else: R) -> Bind<T, C, S, F, OrElse<R>>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: Fn(&mut C) + Clone + Send + Sync + 'static,
{
    bind_or_else(source, write, or_else)
}

/// BSN template returned by [`bind`] and [`bind_or_else`].
pub struct Bind<T, C, S, F, R> {
    source: S,
    write: F,
    or_else: R,
    replay: bool,
    marker: PhantomData<fn() -> (T, C)>,
}

impl<T, C, S, F, R> Bind<T, C, S, F, R> {
    /// Writes only when a value changes, without writing the current value
    /// when the scene spawns.
    #[must_use]
    pub fn deferred(mut self) -> Self {
        self.replay = false;
        self
    }
}

/// Removal fallback of a [`bind_or_else`] binding.
#[derive(Clone)]
pub struct OrElse<R>(R);

/// What a [`Bind`] does when `T` is removed: nothing for [`bind`], or the
/// fallback of [`bind_or_else`].
pub trait BindFallback<T, C>: Clone + Send + Sync + 'static {
    /// Subscribes the entity being built to removals of `T` from `sources`.
    fn subscribe(&self, context: &mut TemplateContext, sources: Vec<Entity>, replay: bool);
}

impl<T, C> BindFallback<T, C> for () {
    fn subscribe(&self, _: &mut TemplateContext, _: Vec<Entity>, _: bool) {}
}

impl<T, C, R> BindFallback<T, C> for OrElse<R>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    R: Fn(&mut C) + Clone + Send + Sync + 'static,
{
    fn subscribe(&self, context: &mut TemplateContext, sources: Vec<Entity>, replay: bool) {
        let or_else = self.0.clone();
        let observer = move |removal: Removal<T>, mut targets: Query<&mut C, Allow<Disabled>>| {
            if let Ok(mut target) = targets.get_mut(removal.target()) {
                or_else(&mut target);
            }
        };
        let replay = replay.then_some(missing_value::<T> as Replay<ValueRemoved<T>>);
        spawn_subscriptions(context, sources, &observer, replay);
    }
}

impl<T, C, S, F, R> Template for Bind<T, C, S, F, R>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: BindFallback<T, C>,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        ensure_tracked::<T>(context)?;
        let mut sources = Vec::new();
        self.source.resolve(context, &mut sources)?;
        let write = self.write.clone();
        let observer = move |change: Reaction<T>, mut targets: Query<&mut C, Allow<Disabled>>| {
            if let Ok(mut target) = targets.get_mut(change.target()) {
                write(&change, &mut target);
            }
        };
        let replay = self
            .replay
            .then_some(current_value::<T> as Replay<ValueChanged<T>>);
        spawn_subscriptions(context, sources.clone(), &observer, replay);
        self.or_else.subscribe(context, sources, self.replay);
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            source: self.source.clone_source(),
            write: self.write.clone(),
            or_else: self.or_else.clone(),
            replay: self.replay,
            marker: PhantomData,
        }
    }
}

impl<T, C, S, F, R> Scene for Bind<T, C, S, F, R>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: BindFallback<T, C>,
{
    fn resolve(
        self,
        _context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        scene.push_template(self);
        Ok(())
    }
}
