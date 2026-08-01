//! Source-preserving routing of entity events to related listener entities.

use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use bevy::{
    ecs::{
        bundle::Bundle,
        event::EntityEvent,
        lifecycle::HookContext,
        system::{FromInput, IntoObserverSystem, SystemInput},
        template::{EntityTemplate, Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::HashSet,
    prelude::*,
    scene::{ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

/// A source event delivered to a subscribed listener.
///
/// Unlike retargeting the source event, this wrapper gives the delivery its own
/// listener target. The source event is cloned as the forwarding observer sees
/// it, without this crate rewriting its fields.
#[derive(Clone, Debug, EntityEvent)]
pub struct SubscriptionEvent<E>
where
    E: EntityEvent + Clone,
{
    #[event_target]
    target: Entity,
    source: Entity,
    event: Arc<E>,
}

impl<E> SubscriptionEvent<E>
where
    E: EntityEvent + Clone,
{
    /// Wraps an event for delivery to the target listener.
    pub fn new(target: Entity, event: E) -> Self {
        let source = event.event_target();
        Self {
            target,
            source,
            event: Arc::new(event),
        }
    }

    fn from_shared(target: Entity, source: Entity, event: Arc<E>) -> Self {
        Self {
            target,
            source,
            event,
        }
    }

    /// Listener entity receiving this subscription event.
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
    /// The crate does not retarget or otherwise modify this clone. Because Bevy
    /// does not define ordering between unrelated observers, changes made by an
    /// earlier observer can already be present in this snapshot.
    #[must_use]
    pub fn event(&self) -> &E {
        &self.event
    }
}

impl<E> Deref for SubscriptionEvent<E>
where
    E: EntityEvent + Clone,
{
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.event()
    }
}

/// Observer input for events forwarded through a subscription.
///
/// This dereferences to the source-event snapshot while exposing the routing
/// source and subscribed listener target.
pub struct Subscription<'w, 't, E, B = ()>
where
    E: EntityEvent + Clone,
    B: Bundle,
{
    on: On<'w, 't, SubscriptionEvent<E>, B>,
}

impl<E, B> SystemInput for Subscription<'_, '_, E, B>
where
    E: EntityEvent + Clone,
    B: Bundle,
{
    type Param<'i> = Subscription<'i, 'i, E, B>;
    type Inner<'i> = On<'i, 'i, SubscriptionEvent<E>, B>;

    fn wrap(on: Self::Inner<'_>) -> Self::Param<'_> {
        Subscription { on }
    }
}

impl<E, B> FromInput<On<'_, '_, SubscriptionEvent<E>, B>> for Subscription<'_, '_, E, B>
where
    E: EntityEvent + Clone,
    B: Bundle,
{
    fn from_inner<'i>(
        on: <On<'_, '_, SubscriptionEvent<E>, B> as SystemInput>::Inner<'i>,
    ) -> Self::Inner<'i> {
        on
    }
}

impl<E, B> Subscription<'_, '_, E, B>
where
    E: EntityEvent + Clone,
    B: Bundle,
{
    /// Listener entity receiving this subscription event.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.on.event().target()
    }

    /// Source entity whose subscription relationship routed this delivery.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.on.event().source()
    }

    /// Snapshot of the source event as observed by the forwarding observer.
    #[must_use]
    pub fn event(&self) -> &E {
        self.on.event().event()
    }

    /// Entity containing the observer that is handling this event.
    #[must_use]
    pub fn observer(&self) -> Entity {
        self.on.observer()
    }
}

impl<E, B> Deref for Subscription<'_, '_, E, B>
where
    E: EntityEvent + Clone,
    B: Bundle,
{
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.event()
    }
}

/// Relationship from a listener entity to its one event source entity.
///
/// Call [`SubscriptionAppExt::register_subscription`] before inserting this
/// relationship directly. Prefer [`subscribe_to`] in BSN scenes.
#[derive(Component, FromTemplate, Clone, Copy, Debug)]
#[component(on_add = assert_subscription_registered::<E>)]
#[relationship(relationship_target = Subscribers<E>)]
pub struct SubscribedTo<E>
where
    E: EntityEvent + Clone,
{
    #[relationship]
    #[entities]
    source: Entity,
    marker: PhantomData<fn() -> E>,
}

impl<E> SubscribedTo<E>
where
    E: EntityEvent + Clone,
{
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
pub struct Subscribers<E>(#[relationship] Vec<Entity>, PhantomData<fn() -> E>)
where
    E: EntityEvent + Clone;

impl<E> Subscribers<E>
where
    E: EntityEvent + Clone,
{
    /// Iterates the listener entities subscribed to this source.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.0.iter().copied()
    }
}

struct SubscribeTo<E, B, M, I>
where
    E: EntityEvent + Clone,
    B: Bundle,
    I: IntoObserverSystem<SubscriptionEvent<E>, B, M>,
{
    source: EntityTemplate,
    observer: I,
    marker: PhantomData<fn() -> (E, B, M)>,
}

impl<E, B, M, I> Template for SubscribeTo<E, B, M, I>
where
    E: EntityEvent + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<E>, B, M> + Clone,
{
    type Output = ();

    fn build_template(&self, context: &mut TemplateContext) -> bevy::ecs::error::Result<()> {
        context.entity.world_scope(install_subscription::<E>);
        let source = self.source.build_template(context)?;
        if ensure_subscription_source_available::<E>(context, source)? {
            context.entity.insert(SubscribedTo::<E>::new(source));
        }
        context.entity.observe(self.observer.clone());
        Ok(())
    }

    fn clone_template(&self) -> Self {
        Self {
            source: self.source.clone_template(),
            observer: self.observer.clone(),
            marker: PhantomData,
        }
    }
}

impl<E, B, M, I> Scene for SubscribeTo<E, B, M, I>
where
    E: EntityEvent + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<E>, B, M> + Clone + Send + Sync,
{
    fn resolve(
        self,
        _context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        scene.push_bundle_template(self);
        Ok(())
    }
}

/// BSN helper that subscribes the current entity to a source and observes the
/// forwarded `SubscriptionEvent`.
///
/// A listener can have one source per event type. Use separate listener
/// entities when the same event type must be received from multiple sources.
pub fn subscribe_to<I, E, B, M>(source: EntityTemplate, observer: I) -> impl Scene
where
    I: IntoObserverSystem<SubscriptionEvent<E>, B, M> + Clone + Send + Sync + 'static,
    E: EntityEvent + Clone,
    B: Bundle,
    M: 'static,
{
    SubscribeTo {
        source,
        observer,
        marker: PhantomData,
    }
}

/// App construction helper for direct [`SubscribedTo`] relationship use.
///
/// [`subscribe_to`] registers its event type automatically. Call this method
/// before the app starts only when relationships will be inserted directly.
pub trait SubscriptionAppExt {
    /// Registers one source-event forwarder for direct `SubscribedTo<E>` use.
    fn register_subscription<E>(&mut self) -> &mut Self
    where
        E: EntityEvent + Clone;
}

impl SubscriptionAppExt for App {
    fn register_subscription<E>(&mut self) -> &mut Self
    where
        E: EntityEvent + Clone,
    {
        install_subscription::<E>(self.world_mut());
        self
    }
}

#[derive(Default, Resource)]
struct SubscriptionRegistry(HashSet<TypeId>);

pub(crate) fn install_subscription<E>(world: &mut World)
where
    E: EntityEvent + Clone,
{
    world.init_resource::<SubscriptionRegistry>();
    let should_install = world
        .resource_mut::<SubscriptionRegistry>()
        .0
        .insert(TypeId::of::<E>());

    if should_install {
        world.add_observer(forward_subscribed_event::<E>);
    }
}

pub(crate) fn ensure_subscription_source_available<E>(
    context: &TemplateContext,
    source: Entity,
) -> bevy::ecs::error::Result<bool>
where
    E: EntityEvent + Clone,
{
    let Some(existing) = context.entity.get::<SubscribedTo<E>>() else {
        return Ok(true);
    };
    if existing.source() == source {
        return Ok(false);
    }
    Err(format!(
        "an entity can subscribe to only one source for `{}`; use separate listener entities",
        type_name::<E>(),
    )
    .into())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy component hooks receive DeferredWorld by value"
)]
fn assert_subscription_registered<E>(world: DeferredWorld, _context: HookContext)
where
    E: EntityEvent + Clone,
{
    let registered = world
        .get_resource::<SubscriptionRegistry>()
        .is_some_and(|registry| registry.0.contains(&TypeId::of::<E>()));
    assert!(
        registered,
        "subscription event `{}` is not registered; call `app.register_subscription::<{}>()` during app construction or use `subscribe_to(...)`",
        type_name::<E>(),
        type_name::<E>(),
    );
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn forward_subscribed_event<E>(event: On<E>, mut world: DeferredWorld)
where
    E: EntityEvent + Clone,
{
    let source = event.event_target();
    let (entities, mut commands) = world.entities_and_commands();
    let Ok(source_entity) = entities.get(source) else {
        return;
    };
    let Some(subscribers) = source_entity.get::<Subscribers<E>>() else {
        return;
    };
    let event = Arc::new(event.event().clone());

    for target in subscribers.iter() {
        commands.trigger(SubscriptionEvent::from_shared(
            target,
            source,
            Arc::clone(&event),
        ));
    }
}
