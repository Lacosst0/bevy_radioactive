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
    scene::EmptySceneEffect,
};

use crate::subscription::{
    Source, Subscribe, Subscription, SubscriptionEvent, install_subscription, register_routed,
    route,
};

/// BSN template returned by [`watch`].
pub type Watch<T, S, I, M> = Subscribe<ValueChanged<T>, S, I, M>;

/// BSN template returned by [`on_removed`].
pub type WatchRemoval<T, S, I, M> = Subscribe<ValueRemoved<T>, S, I, M>;

/// Scheduling set containing the reactive runtime.
///
/// The runtime runs in [`PostUpdate`]. With the `ui` feature it is ordered
/// before `UiSystems::Prepare`, so text and style written by reactions are
/// laid out in the same frame. Order systems that mutate watched values
/// in `PostUpdate` `.before(ReactiveSet::Publish)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum ReactiveSet {
    /// Publishes [`ValueChanged`] events and runs `webby` effects until stable.
    Publish,
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
    /// is reached; remaining changes are published on the next frame.
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
            .init_schedule(ReactivePublish)
            .add_systems(PostUpdate, run_reactive.in_set(ReactiveSet::Publish));
        crate::runtime::init(app.world_mut());
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            ReactiveSet::Publish.before(bevy::ui::UiSystems::Prepare),
        );
    }
}

/// Tracked value types, and publishers waiting to be added to
/// [`ReactivePublish`] at the start of the next pass.
#[derive(Default, Resource)]
struct ReactiveRegistry {
    tracked: HashSet<TypeId>,
    pending: Vec<fn(&mut Schedule)>,
}

/// Registers `T` once. Safe at any time, including while scenes spawn
/// during a reactive pass: the publisher joins the schedule between passes.
fn register<T: Component + Clone>(world: &mut World) {
    let mut registry = world.resource_mut::<ReactiveRegistry>();
    if !registry.tracked.insert(TypeId::of::<T>()) {
        return;
    }
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

/// Counts publications in the current pass.
#[derive(Default, Resource)]
struct PublishCounter(AtomicUsize);

fn run_reactive(world: &mut World) {
    let max_passes = world.resource::<ReactiveSettings>().max_passes.max(1);
    for _ in 0..max_passes {
        add_pending_publishers(world);
        world
            .resource::<PublishCounter>()
            .0
            .store(0, Ordering::Relaxed);
        let _ = world.try_run_schedule(ReactivePublish);
        world.flush();
        let published = world.resource::<PublishCounter>().0.load(Ordering::Relaxed);
        let ran = crate::runtime::run_effects(world);
        if published == 0 && ran == 0 {
            return;
        }
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
/// Delivered synchronously while the source still exists, so subscriptions
/// are routed before the source's relationships are torn down.
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

/// Observer input used by [`on_removed`].
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
    /// Optional for scenes: [`watch`], [`bind`] and [`on_removed`] track their
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
    values: Query<(Entity, &T), (Changed<T>, Allow<Disabled>)>,
    counter: Res<PublishCounter>,
    mut commands: Commands,
) {
    for (source, value) in &values {
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
/// watches. Current state is not replayed; changes made while the scene is
/// spawned are published in the same frame.
pub fn watch<T, S, I, M>(source: S, observer: I) -> Watch<T, S, I, M>
where
    T: Component + Clone,
    S: Source<ValueChanged<T>>,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, M>,
{
    Subscribe::new(source, observer).with_check(ensure_tracked::<T>)
}

/// Observes removal of tracked `T` from `source`, including despawn.
pub fn on_removed<T, S, I, M>(source: S, observer: I) -> WatchRemoval<T, S, I, M>
where
    T: Component + Clone,
    S: Source<ValueRemoved<T>>,
    I: IntoObserverSystem<SubscriptionEvent<ValueRemoved<T>>, M>,
{
    Subscribe::new(source, observer).with_check(ensure_tracked::<T>)
}

/// Writes each published `T` from `source` into component `C` on the current
/// BSN entity.
///
/// ```ignore
/// Text("")
/// bind(#Player, |hp: &Health, text: &mut Text| **text = hp.current.to_string())
/// ```
///
/// Like [`watch`], `source` may be a `#Name`, [`Singleton`], or a list.
pub fn bind<T, C, S, F>(source: S, write: F) -> impl Template<Output = EmptySceneEffect>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
{
    watch(
        source,
        move |change: Reaction<T>, mut targets: Query<&mut C, Allow<Disabled>>| {
            if let Ok(mut target) = targets.get_mut(change.target()) {
                write(&change, &mut target);
            }
        },
    )
}

/// Like [`bind`], and also writes `C` with `removed` when `T` is removed from
/// a source or the source is despawned.
///
/// ```ignore
/// Text("")
/// bind_or(
///     #Player,
///     |hp: &Health, text: &mut Text| **text = hp.current.to_string(),
///     |text: &mut Text| **text = "DEAD".into(),
/// )
/// ```
pub fn bind_or<T, C, S, F, R>(
    source: S,
    write: F,
    removed: R,
) -> impl Template<Output = EmptySceneEffect>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: Source<ValueChanged<T>> + Source<ValueRemoved<T>>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: Fn(&mut C) + Clone + Send + Sync + 'static,
{
    let removal_source = <S as Source<ValueRemoved<T>>>::clone_source(&source);
    Both(
        bind(source, write),
        on_removed(
            removal_source,
            move |removal: Removal<T>, mut targets: Query<&mut C, Allow<Disabled>>| {
                if let Ok(mut target) = targets.get_mut(removal.target()) {
                    removed(&mut target);
                }
            },
        ),
    )
}

/// Applies two templates in order.
struct Both<A, B>(A, B);

impl<A, B> Template for Both<A, B>
where
    A: Template<Output = EmptySceneEffect>,
    B: Template<Output = EmptySceneEffect>,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        self.0.build_template(context)?;
        self.1.build_template(context)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone_template(), self.1.clone_template())
    }
}
