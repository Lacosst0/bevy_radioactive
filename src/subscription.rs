//! Source-preserving routing of entity events to related listener entities.

use std::{any::TypeId, marker::PhantomData, ops::Deref, sync::Arc};

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
/// Unlike retargeting the original event, this wrapper preserves the original
/// event and source. Its entity-event target is the listener that receives it.
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

    /// Original target of the source event.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.source
    }

    /// Original, unmodified source event.
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
/// This dereferences to the original event while exposing both the original
/// source entity and the subscribed listener target.
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

    /// Original target of the source event.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.on.event().source()
    }

    /// Original, unmodified source event.
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

/// Relationship from a listener entity to its event source entity.
#[derive(Component, FromTemplate, Clone, Copy, Debug)]
#[component(on_add = queue_subscription_forwarder::<E>)]
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
        context.entity.insert(SubscribedTo::<E>::new(source));
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

fn queue_subscription_forwarder<E>(mut world: DeferredWorld, _context: HookContext)
where
    E: EntityEvent + Clone,
{
    world.commands().queue(|world: &mut World| {
        install_subscription::<E>(world);
    });
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn forward_subscribed_event<E>(
    event: On<E>,
    subscribers: Query<&Subscribers<E>>,
    mut commands: Commands,
) where
    E: EntityEvent + Clone,
{
    let source = event.event_target();
    let Ok(subscribers) = subscribers.get(source) else {
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
