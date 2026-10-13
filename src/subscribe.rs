//! Listen to entity events fired on *other* entities.
//!
//! BSN's `on(...)` observes events on the entity that declares it.
//! [`subscribe_to`] observes events on a source entity instead, and
//! [`bind_event`] writes a component of the declaring entity in response:
//!
//! ```ignore
//! use bevy_radioactive::subscribe::*;
//!
//! bsn! {
//!     Children [
//!         #Plus Button
//!         --
//!         Count(0)
//!         bind_event(#Plus, |_: &Activate, count: &mut Count| count.0 += 1)
//!         --
//!         Text("")
//!         subscribe_to(#Plus, |click: Subscription<Activate>, mut log: ResMut<Log>| {
//!             log.push(click.source());
//!         })
//!     ]
//! }
//! ```
//!
//! A source is a `#Name`, an [`Entity`], or a list such as `[#A, #B]`. One
//! entity may declare any number of subscriptions. They are despawned with the
//! entity that declared them. Nothing needs to be registered up front.

use std::{any::TypeId, marker::PhantomData, ops::Deref, sync::Arc};

use bevy::{
    ecs::{
        component::Mutable,
        entity_disabling::Disabled,
        event::EntityEvent,
        query::Allow,
        system::{FromInput, IntoObserverSystem, SystemInput},
        template::{EntityTemplate, Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::HashSet,
    prelude::*,
    scene::EmptySceneEffect,
};

/// Observes `E` fired on `source` from the current BSN entity.
///
/// The handler receives a [`Subscription`]: `target()` is the declaring entity,
/// `source()` is the entity the event fired on, and it dereferences to the
/// event itself.
pub fn subscribe_to<E, S, I, M>(source: S, handler: I) -> impl Template<Output = EmptySceneEffect>
where
    E: EntityEvent + Clone,
    S: Source,
    I: IntoObserverSystem<SubscriptionEvent<E>, M> + Clone,
    M: 'static,
{
    Subscribe {
        source,
        handler,
        marker: PhantomData::<fn() -> (E, M)>,
    }
}

/// Writes component `C` of the current BSN entity whenever `E` fires on
/// `source`.
///
/// ```ignore
/// Count(0)
/// bind_event(#Plus, |_: &Activate, count: &mut Count| count.0 += 1)
/// ```
pub fn bind_event<E, C, S, F>(source: S, write: F) -> impl Template<Output = EmptySceneEffect>
where
    E: EntityEvent + Clone,
    C: Component<Mutability = Mutable>,
    S: Source,
    F: Fn(&E, &mut C) + Clone + Send + Sync + 'static,
{
    subscribe_to(
        source,
        move |event: Subscription<E>, mut targets: Query<&mut C, Allow<Disabled>>| {
            if let Ok(mut target) = targets.get_mut(event.target()) {
                write(&event, &mut target);
            }
        },
    )
}

/// Handler input for [`subscribe_to`].
///
/// Dereferences to the source event.
pub struct Subscription<'w, 't, E: EntityEvent + Clone>(On<'w, 't, SubscriptionEvent<E>>);

impl<E: EntityEvent + Clone> Subscription<'_, '_, E> {
    /// Entity that declared the subscription.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.0.event().target
    }

    /// Entity the event fired on.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.0.event().source
    }

    /// The source event.
    #[must_use]
    pub fn event(&self) -> &E {
        &self.0.event().event
    }
}

impl<E: EntityEvent + Clone> Deref for Subscription<'_, '_, E> {
    type Target = E;

    fn deref(&self) -> &E {
        self.event()
    }
}

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

/// The event delivered to a subscription. Use [`Subscription`] in handlers.
#[doc(hidden)]
#[derive(Clone, EntityEvent)]
pub struct SubscriptionEvent<E: EntityEvent + Clone> {
    #[event_target]
    delivery: Entity,
    target: Entity,
    source: Entity,
    event: Arc<E>,
}

/// Entities accepted as a subscription source: a `#Name`, an [`Entity`], or an
/// array or `Vec` of sources.
pub trait Source: Send + Sync + 'static {
    #[doc(hidden)]
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result;

    #[doc(hidden)]
    #[must_use]
    fn clone_source(&self) -> Self
    where
        Self: Sized;
}

impl Source for EntityTemplate {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        out.push(self.build_template(context)?);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        self.clone_template()
    }
}

impl Source for Entity {
    fn resolve(&self, _: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        out.push(*self);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        *self
    }
}

impl<S: Source, const N: usize> Source for [S; N] {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        self.iter()
            .try_for_each(|source| source.resolve(context, out))
    }

    fn clone_source(&self) -> Self {
        std::array::from_fn(|index| self[index].clone_source())
    }
}

impl<S: Source> Source for Vec<S> {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        self.iter()
            .try_for_each(|source| source.resolve(context, out))
    }

    fn clone_source(&self) -> Self {
        self.iter().map(Source::clone_source).collect()
    }
}

struct Subscribe<E, S, I, M> {
    source: S,
    handler: I,
    marker: PhantomData<fn() -> (E, M)>,
}

impl<E, S, I, M> Template for Subscribe<E, S, I, M>
where
    E: EntityEvent + Clone,
    S: Source,
    I: IntoObserverSystem<SubscriptionEvent<E>, M> + Clone,
    M: 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let mut sources = Vec::new();
        self.source.resolve(context, &mut sources)?;
        let listener = context.entity.id();
        context.entity.world_scope(|world| {
            install_forwarder::<E>(world);
            for source in sources {
                world
                    .spawn((DeliveryOf(listener), SubscribedTo::<E>::new(source)))
                    .observe(self.handler.clone());
            }
        });
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            source: self.source.clone_source(),
            handler: self.handler.clone(),
            marker: PhantomData,
        }
    }
}

/// One delivery entity per (listener, source) pair, pointing at the source.
#[derive(Component)]
#[relationship(relationship_target = Subscribers<E>)]
struct SubscribedTo<E: EntityEvent + Clone> {
    #[relationship]
    source: Entity,
    marker: PhantomData<fn() -> E>,
}

impl<E: EntityEvent + Clone> SubscribedTo<E> {
    fn new(source: Entity) -> Self {
        Self {
            source,
            marker: PhantomData,
        }
    }
}

#[derive(Component)]
#[relationship_target(relationship = SubscribedTo<E>)]
struct Subscribers<E: EntityEvent + Clone>(#[relationship] Vec<Entity>, PhantomData<fn() -> E>);

/// Owns delivery entities, so they despawn with the listener.
#[derive(Component)]
#[relationship(relationship_target = Deliveries)]
struct DeliveryOf(Entity);

#[derive(Component)]
#[relationship_target(relationship = DeliveryOf, linked_spawn)]
struct Deliveries(Vec<Entity>);

#[derive(Default, Resource)]
struct Forwarders(HashSet<TypeId>);

fn install_forwarder<E: EntityEvent + Clone>(world: &mut World) {
    if world
        .get_resource_or_init::<Forwarders>()
        .0
        .insert(TypeId::of::<E>())
    {
        world.add_observer(forward::<E>);
    }
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
    let shared = Arc::new(event.event().clone());
    for &delivery in &subscribers.0 {
        let target = entities
            .get(delivery)
            .ok()
            .and_then(|entity| entity.get::<DeliveryOf>())
            .map_or(delivery, |owner| owner.0);
        commands.trigger(SubscriptionEvent {
            delivery,
            target,
            source,
            event: Arc::clone(&shared),
        });
    }
}
