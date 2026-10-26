//! Source-preserving routing of entity events to related listener entities.

use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use bevy::{
    ecs::{
        event::EntityEvent,
        lifecycle::HookContext,
        system::{FromInput, IntoObserverSystem, SystemInput},
        template::{EntityTemplate, Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::HashMap,
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

/// A source event delivered to a subscribed listener.
///
/// The delivery has its own target; the source event is shared as the
/// forwarding observer saw it, without this crate rewriting its fields.
#[derive(Clone, Debug, EntityEvent)]
pub struct SubscriptionEvent<E: EntityEvent + Clone> {
    /// Entity holding the [`SubscribedTo`] relationship and the observer.
    #[event_target]
    delivery: Entity,
    target: Entity,
    source: Entity,
    event: Arc<E>,
}

impl<E: EntityEvent + Clone> SubscriptionEvent<E> {
    /// Wraps an event for delivery to the target listener.
    pub fn new(target: Entity, event: E) -> Self {
        Self {
            delivery: target,
            target,
            source: event.event_target(),
            event: Arc::new(event),
        }
    }

    /// Listener entity receiving this subscription event.
    ///
    /// For subscriptions created by BSN helpers this is the scene entity that
    /// declared the subscription, not the internal subscription entity.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.target
    }

    /// Source entity whose subscription relationship routed this delivery.
    ///
    /// For an event that propagates, this is the current propagation entity.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.source
    }

    /// Snapshot of the source event as observed by the forwarding observer.
    ///
    /// Bevy does not order unrelated observers, so changes made by an earlier
    /// observer can already be present in this snapshot.
    #[must_use]
    pub fn event(&self) -> &E {
        &self.event
    }
}

impl<E: EntityEvent + Clone> Deref for SubscriptionEvent<E> {
    type Target = E;

    fn deref(&self) -> &E {
        &self.event
    }
}

/// Observer input for events forwarded through a subscription.
///
/// Dereferences to [`SubscriptionEvent`], and through it to the source event.
pub struct Subscription<'w, 't, E: EntityEvent + Clone>(On<'w, 't, SubscriptionEvent<E>>);

impl<E: EntityEvent + Clone> SystemInput for Subscription<'_, '_, E> {
    type Param<'i> = Subscription<'i, 'i, E>;
    type Inner<'i> = On<'i, 'i, SubscriptionEvent<E>>;

    fn wrap(on: Self::Inner<'_>) -> Self::Param<'_> {
        Subscription(on)
    }
}

impl<E: EntityEvent + Clone> FromInput<On<'_, '_, SubscriptionEvent<E>>>
    for Subscription<'_, '_, E>
{
    fn from_inner<'i>(
        on: <On<'_, '_, SubscriptionEvent<E>> as SystemInput>::Inner<'i>,
    ) -> Self::Inner<'i> {
        on
    }
}

impl<E: EntityEvent + Clone> Subscription<'_, '_, E> {
    /// Entity containing the observer that is handling this event.
    #[must_use]
    pub fn observer(&self) -> Entity {
        self.0.observer()
    }
}

impl<E: EntityEvent + Clone> Deref for Subscription<'_, '_, E> {
    type Target = SubscriptionEvent<E>;

    fn deref(&self) -> &SubscriptionEvent<E> {
        self.0.event()
    }
}

/// Relationship from a delivery entity to its one event source entity.
///
/// Call [`SubscriptionAppExt::register_subscription`] before inserting this
/// relationship directly. Prefer [`subscribe_to`] in BSN scenes.
///
/// When the source is despawned the relationship is removed. Internal
/// subscription entities spawned by BSN helpers (those with
/// [`SubscriptionOf`]) are then despawned too; entities holding the
/// relationship directly are kept.
#[derive(Component, FromTemplate, Clone, Copy, Debug)]
#[component(
    on_add = assert_subscription_registered::<E>,
    on_remove = despawn_internal_subscription,
)]
#[relationship(relationship_target = Subscribers<E>)]
pub struct SubscribedTo<E: EntityEvent + Clone> {
    #[relationship]
    #[entities]
    source: Entity,
    marker: PhantomData<fn() -> E>,
}

impl<E: EntityEvent + Clone> SubscribedTo<E> {
    /// Creates a relationship targeting the source.
    #[must_use]
    pub fn new(source: Entity) -> Self {
        Self {
            source,
            marker: PhantomData,
        }
    }

    /// Event source entity targeted by this relationship.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.source
    }
}

/// Relationship target stored on an event source entity.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = SubscribedTo<E>)]
pub struct Subscribers<E: EntityEvent + Clone>(#[relationship] Vec<Entity>, PhantomData<fn() -> E>);

impl<E: EntityEvent + Clone> Subscribers<E> {
    /// Iterates the delivery entities subscribed to this source.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.0.iter().copied()
    }
}

/// Links an internal subscription entity to the listener that declared it.
///
/// BSN helpers spawn one subscription entity per source, so one listener can
/// subscribe to any number of sources for the same event type. Despawning the
/// listener despawns its subscriptions.
#[derive(Component, Clone, Copy, Debug)]
#[relationship(relationship_target = Subscriptions)]
pub struct SubscriptionOf(#[entities] pub Entity);

/// Subscription entities owned by a listener.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = SubscriptionOf, linked_spawn)]
pub struct Subscriptions(Vec<Entity>);

impl Subscriptions {
    /// Iterates the subscription entities owned by this listener.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.0.iter().copied()
    }
}

/// One or more event sources accepted by BSN subscription helpers.
///
/// Implemented for `#Name` references ([`EntityTemplate`]), [`Entity`],
/// arrays and vectors of sources (`[#A, #B]`), and — for state events —
/// [`Singleton`](crate::reactivity::Singleton).
pub trait Source<E>: Send + Sync + 'static {
    /// Resolves the source entities, appending them to `out`.
    ///
    /// # Errors
    /// Returns an error when a source cannot be resolved.
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result;

    /// Clones the unresolved source.
    #[must_use]
    fn clone_source(&self) -> Self
    where
        Self: Sized;
}

impl<E> Source<E> for EntityTemplate {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        out.push(self.build_template(context)?);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        self.clone_template()
    }
}

impl<E> Source<E> for Entity {
    fn resolve(&self, _context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        out.push(*self);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        *self
    }
}

impl<E, S: Source<E>, const N: usize> Source<E> for [S; N] {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        self.iter()
            .try_for_each(|source| source.resolve(context, out))
    }

    fn clone_source(&self) -> Self {
        std::array::from_fn(|index| self[index].clone_source())
    }
}

impl<E, S: Source<E>> Source<E> for Vec<S> {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        self.iter()
            .try_for_each(|source| source.resolve(context, out))
    }

    fn clone_source(&self) -> Self {
        self.iter().map(Source::clone_source).collect()
    }
}

/// BSN template that subscribes the current entity to the entities resolved
/// by `S` and observes forwarded `E` events. See [`subscribe_to`].
pub struct Subscribe<E, S, I, M> {
    source: S,
    observer: I,
    check: Option<fn(&mut TemplateContext) -> Result>,
    pub(crate) replay: Option<Replay<E>>,
    marker: PhantomData<fn() -> (E, M)>,
}

/// Builds the event a new subscription replays from a source's current
/// state, or `None` to replay nothing.
pub(crate) type Replay<E> = fn(&World, Entity) -> Option<E>;

impl<E, S, I, M> Subscribe<E, S, I, M> {
    pub(crate) fn new(source: S, observer: I) -> Self {
        Self {
            source,
            observer,
            check: None,
            replay: None,
            marker: PhantomData,
        }
    }

    pub(crate) fn with_check(mut self, check: fn(&mut TemplateContext) -> Result) -> Self {
        self.check = Some(check);
        self
    }
}

/// Spawns one subscription entity per source for the entity being built.
///
/// With `replay`, each subscription also receives the event `replay` builds
/// from its source, once the whole scene has been spawned.
pub(crate) fn spawn_subscriptions<E, I, M>(
    context: &mut TemplateContext,
    sources: Vec<Entity>,
    observer: &I,
    replay: Option<Replay<E>>,
) where
    E: EntityEvent + Clone,
    I: IntoObserverSystem<SubscriptionEvent<E>, M> + Clone,
    M: 'static,
{
    let listener = context.entity.id();
    context.entity.world_scope(|world| {
        install_subscription::<E>(world);
        let mut spawned = Vec::with_capacity(sources.len());
        for source in sources {
            let delivery = world
                .spawn((
                    SubscriptionOf(listener),
                    Deliveries::default(),
                    SubscribedTo::<E>::new(source),
                ))
                .observe(observer.clone())
                .id();
            spawned.push((delivery, source));
        }
        if let Some(replay) = replay {
            // Deferred to the reactive runtime, so the scene, including
            // sources and target components after this template, is
            // complete when the current state is read.
            crate::reactivity::defer_replay(world, move |world: &mut World| {
                for (delivery, source) in spawned {
                    if world.get_entity(delivery).is_ok()
                        && let Some(event) = replay(world, source)
                    {
                        world.trigger(SubscriptionEvent {
                            delivery,
                            target: listener,
                            source,
                            event: Arc::new(event),
                        });
                    }
                }
            });
        }
    });
}

impl<E, S, I, M> Template for Subscribe<E, S, I, M>
where
    E: EntityEvent + Clone,
    S: Source<E>,
    I: IntoObserverSystem<SubscriptionEvent<E>, M> + Clone,
    M: 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        if let Some(check) = self.check {
            check(context)?;
        } else {
            context
                .entity
                .world_scope(|world| crate::reactivity::warn_if_untracked::<E>(world));
        }
        let mut sources = Vec::new();
        self.source.resolve(context, &mut sources)?;
        spawn_subscriptions(context, sources, &self.observer, self.replay);
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            source: self.source.clone_source(),
            observer: self.observer.clone(),
            check: self.check,
            replay: self.replay,
            marker: PhantomData,
        }
    }
}

impl<E, S, I, M> Scene for Subscribe<E, S, I, M>
where
    E: EntityEvent + Clone,
    S: Source<E>,
    I: IntoObserverSystem<SubscriptionEvent<E>, M> + Clone + Sync,
    M: 'static,
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

/// Subscribes the current BSN entity to `source` and observes the forwarded
/// [`SubscriptionEvent`].
///
/// `source` may be one `#Name`, an [`Entity`], or a list such as `[#A, #B]`.
/// Any number of subscriptions, for any sources and event types, may be
/// declared on one entity; each handler receives each routed event once.
pub fn subscribe_to<E, S, I, M>(source: S, observer: I) -> Subscribe<E, S, I, M>
where
    E: EntityEvent + Clone,
    S: Source<E>,
    I: IntoObserverSystem<SubscriptionEvent<E>, M>,
{
    Subscribe::new(source, observer)
}

/// App construction helper for direct [`SubscribedTo`] relationship use.
pub trait SubscriptionAppExt {
    /// Registers the source-event forwarder for direct `SubscribedTo<E>` use.
    ///
    /// [`subscribe_to`] registers its event type automatically.
    fn register_subscription<E: EntityEvent + Clone>(&mut self) -> &mut Self;
}

impl SubscriptionAppExt for App {
    fn register_subscription<E: EntityEvent + Clone>(&mut self) -> &mut Self {
        install_subscription::<E>(self.world_mut());
        self
    }
}

/// Registered event types, with the entity of their generic forwarder, or
/// `None` for events the crate routes itself.
#[derive(Default, Resource)]
struct SubscriptionRegistry(HashMap<TypeId, Option<Entity>>);

pub(crate) fn install_subscription<E: EntityEvent + Clone>(world: &mut World) {
    world.init_resource::<SubscriptionRegistry>();
    if world
        .resource::<SubscriptionRegistry>()
        .0
        .contains_key(&TypeId::of::<E>())
    {
        return;
    }
    let forwarder = world.add_observer(forward::<E>).id();
    world
        .resource_mut::<SubscriptionRegistry>()
        .0
        .insert(TypeId::of::<E>(), Some(forwarder));
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy component hooks receive DeferredWorld by value"
)]
fn assert_subscription_registered<E: EntityEvent + Clone>(world: DeferredWorld, _: HookContext) {
    assert!(
        world
            .get_resource::<SubscriptionRegistry>()
            .is_some_and(|registry| registry.0.contains_key(&TypeId::of::<E>())),
        "subscription event `{0}` is not registered; call `app.register_subscription::<{0}>()` during app construction or use `subscribe_to(...)`",
        type_name::<E>(),
    );
}

/// Bookkeeping on internal subscription entities: deliveries routed but not
/// yet run, and whether the source is gone.
#[derive(Component, Default)]
struct Deliveries {
    pending: u32,
    orphaned: bool,
}

/// Despawns a BSN subscription entity once it no longer has a source.
///
/// Despawning a source routes its removal events before the relationship is
/// removed, but runs them later; the entity then stays until they ran.
fn despawn_internal_subscription(mut world: DeferredWorld, context: HookContext) {
    let Some(mut deliveries) = world.get_mut::<Deliveries>(context.entity) else {
        return;
    };
    if deliveries.pending == 0 {
        world.commands().entity(context.entity).try_despawn();
    } else {
        deliveries.orphaned = true;
    }
}

/// Runs one routed delivery, then despawns an orphaned subscription entity
/// once its last delivery ran.
fn deliver<E: EntityEvent + Clone>(world: &mut World, event: SubscriptionEvent<E>) {
    let delivery = event.delivery;
    world.trigger(event);
    let Some(mut deliveries) = world
        .get_entity_mut(delivery)
        .ok()
        .and_then(EntityWorldMut::into_mut::<Deliveries>)
    else {
        return;
    };
    deliveries.pending = deliveries.pending.saturating_sub(1);
    if deliveries.pending == 0 && deliveries.orphaned {
        world.despawn(delivery);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn forward<E: EntityEvent + Clone>(event: On<E>, world: DeferredWorld) {
    route(world, event.event_target(), event.event());
}

/// The generic forwarder observer installed for `E`, if any.
pub(crate) fn forwarder<E: 'static>(world: &World) -> Option<Entity> {
    world
        .get_resource::<SubscriptionRegistry>()?
        .0
        .get(&TypeId::of::<E>())
        .copied()
        .flatten()
}

/// Marks `E` as registered without the generic forwarder; the caller routes
/// `E` itself with [`route`].
///
/// A forwarder installed earlier, by subscribing to `E` before the crate
/// routed it, is removed so events are not delivered twice.
pub(crate) fn register_routed<E: EntityEvent + Clone>(world: &mut World) {
    world.init_resource::<SubscriptionRegistry>();
    let previous = world
        .resource_mut::<SubscriptionRegistry>()
        .0
        .insert(TypeId::of::<E>(), None);
    if let Some(Some(forwarder)) = previous {
        world.despawn(forwarder);
    }
}

/// Queues one [`SubscriptionEvent`] per subscriber of `source`.
pub(crate) fn route<E: EntityEvent + Clone>(mut world: DeferredWorld, source: Entity, event: &E) {
    let (entities, mut commands) = world.entities_and_commands();
    let Some(subscribers) = entities
        .get(source)
        .ok()
        .and_then(|entity| entity.get::<Subscribers<E>>())
    else {
        return;
    };
    let event = Arc::new(event.clone());
    let mut internal = Vec::new();
    for delivery in subscribers.iter() {
        let owner = entities
            .get(delivery)
            .ok()
            .and_then(|entity| entity.get::<SubscriptionOf>());
        if owner.is_some() {
            internal.push(delivery);
        }
        let event = SubscriptionEvent {
            delivery,
            target: owner.map_or(delivery, |owner| owner.0),
            source,
            event: Arc::clone(&event),
        };
        commands.queue(move |world: &mut World| deliver(world, event));
    }
    for delivery in internal {
        if let Some(mut deliveries) = world.get_mut::<Deliveries>(delivery) {
            deliveries.pending += 1;
        }
    }
}
