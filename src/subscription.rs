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
    platform::collections::HashSet,
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

/// A source event delivered to a subscribed listener.
///
/// The delivery has its own listener target; the source event is shared as
/// the forwarding observer saw it, without this crate rewriting its fields.
#[derive(Clone, Debug, EntityEvent)]
pub struct SubscriptionEvent<E: EntityEvent + Clone> {
    #[event_target]
    target: Entity,
    source: Entity,
    event: Arc<E>,
}

impl<E: EntityEvent + Clone> SubscriptionEvent<E> {
    /// Wraps an event for delivery to the target listener.
    pub fn new(target: Entity, event: E) -> Self {
        Self {
            target,
            source: event.event_target(),
            event: Arc::new(event),
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

/// Relationship from a listener entity to its one event source entity.
///
/// Call [`SubscriptionAppExt::register_subscription`] before inserting this
/// relationship directly. Prefer [`subscribe_to`] in BSN scenes.
#[derive(Component, FromTemplate, Clone, Copy, Debug)]
#[component(on_add = assert_subscription_registered::<E>)]
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
    /// Iterates the listener entities subscribed to this source.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.0.iter().copied()
    }
}

/// BSN template that subscribes the current entity to the entity built by `S`
/// and observes forwarded `E` events. See [`subscribe_to`].
pub struct Subscribe<E, S, I, M> {
    source: S,
    observer: I,
    marker: PhantomData<fn() -> (E, M)>,
}

impl<E, S, I, M> Subscribe<E, S, I, M> {
    pub(crate) fn new(source: S, observer: I) -> Self {
        Self {
            source,
            observer,
            marker: PhantomData,
        }
    }
}

impl<E, S, I, M> Template for Subscribe<E, S, I, M>
where
    E: EntityEvent + Clone,
    S: Template<Output = Entity>,
    I: IntoObserverSystem<SubscriptionEvent<E>, M> + Clone,
    M: 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let source = self.source.build_template(context)?;
        context.entity.world_scope(install_subscription::<E>);
        match context.entity.get::<SubscribedTo<E>>() {
            Some(existing) if existing.source == source => {}
            Some(_) => {
                return Err(format!(
                    "an entity can subscribe to only one source for `{}`; use separate listener entities",
                    type_name::<E>(),
                )
                .into());
            }
            None => {
                context.entity.insert(SubscribedTo::<E>::new(source));
            }
        }
        context.entity.observe(self.observer.clone());
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self::new(self.source.clone_template(), self.observer.clone())
    }
}

impl<E, S, I, M> Scene for Subscribe<E, S, I, M>
where
    E: EntityEvent + Clone,
    S: Template<Output = Entity> + Send + Sync + 'static,
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
/// A listener can have one source per event type. Use separate listener
/// entities when the same event type must be received from multiple sources.
pub fn subscribe_to<E, I, M>(
    source: EntityTemplate,
    observer: I,
) -> Subscribe<E, EntityTemplate, I, M>
where
    E: EntityEvent + Clone,
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

#[derive(Default, Resource)]
struct SubscriptionRegistry(HashSet<TypeId>);

fn install_subscription<E: EntityEvent + Clone>(world: &mut World) {
    world.init_resource::<SubscriptionRegistry>();
    if world
        .resource_mut::<SubscriptionRegistry>()
        .0
        .insert(TypeId::of::<E>())
    {
        world.add_observer(forward::<E>);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy component hooks receive DeferredWorld by value"
)]
fn assert_subscription_registered<E: EntityEvent + Clone>(world: DeferredWorld, _: HookContext) {
    assert!(
        world
            .get_resource::<SubscriptionRegistry>()
            .is_some_and(|registry| registry.0.contains(&TypeId::of::<E>())),
        "subscription event `{0}` is not registered; call `app.register_subscription::<{0}>()` during app construction or use `subscribe_to(...)`",
        type_name::<E>(),
    );
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn forward<E: EntityEvent + Clone>(event: On<E>, mut world: DeferredWorld) {
    let source = event.event_target();
    let (entities, mut commands) = world.entities_and_commands();
    let Some(subscribers) = entities
        .get(source)
        .ok()
        .and_then(|entity| entity.get::<Subscribers<E>>())
    else {
        return;
    };
    let event = Arc::new(event.event().clone());
    for target in subscribers.iter() {
        commands.trigger(SubscriptionEvent {
            target,
            source,
            event: Arc::clone(&event),
        });
    }
}
