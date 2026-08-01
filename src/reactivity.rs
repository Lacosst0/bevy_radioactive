use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use bevy::{
    ecs::{
        bundle::Bundle,
        entity_disabling::Disabled,
        event::{EntityEvent, EntityTrigger, Event},
        query::{Allow, Changed as BevyChanged, Without},
        resource::IsResource,
        system::IntoObserverSystem,
        template::{EntityTemplate, Template, TemplateContext},
    },
    platform::collections::HashSet,
    prelude::*,
    scene::{ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

use crate::subscription::{
    SubscribedTo, Subscription, SubscriptionEvent, ensure_subscription_source_available,
    install_subscription,
};

type ObserverMarker<E, B, M> = fn() -> (E, B, M);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SourceKind {
    Component,
    Resource,
}

#[derive(Default, Resource)]
struct ReactiveRegistry {
    sources: HashSet<(TypeId, SourceKind)>,
}

impl ReactiveRegistry {
    fn register<T: 'static>(&mut self, kind: SourceKind) -> bool {
        let opposite = match kind {
            SourceKind::Component => SourceKind::Resource,
            SourceKind::Resource => SourceKind::Component,
        };
        assert!(
            !self.contains::<T>(opposite),
            "`{}` cannot be tracked as both a resource and a component; use distinct wrapper types",
            type_name::<T>(),
        );
        self.sources.insert((TypeId::of::<T>(), kind))
    }

    fn contains<T: 'static>(&self, kind: SourceKind) -> bool {
        self.sources.contains(&(TypeId::of::<T>(), kind))
    }
}

/// Scheduling set containing change publishers installed by this crate.
///
/// Publishers run in [`Last`]. Systems that also mutate watched values in
/// `Last` can order themselves before this set when same-frame delivery is
/// required.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum ReactiveSet {
    /// Reads Bevy change ticks and publishes [`ValueChanged`] events.
    Publish,
}

/// A snapshot published after a component or resource is inserted or mutably
/// dereferenced.
///
/// The event targets the entity that owns the value. For a resource, that is
/// Bevy 0.19's singleton resource entity. Global observers receive this event
/// once; BSN watchers receive a separate, source-preserving [`Subscription`].
pub struct ValueChanged<T: Component> {
    source: Entity,
    value: Arc<T>,
}

impl<T: Component> ValueChanged<T> {
    fn new(source: Entity, value: Arc<T>) -> Self {
        Self { source, value }
    }

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

    fn deref(&self) -> &Self::Target {
        self.value()
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

/// Observer input used by [`watch_resource`] and [`watch_component`].
///
/// It dereferences through [`ValueChanged`] to the captured `T`. Use
/// [`Subscription::target`] for the watching entity and
/// [`Subscription::source`] for the entity that owns `T`.
pub type Reaction<'w, 't, T, B = ()> = Subscription<'w, 't, ValueChanged<T>, B>;

/// App construction helpers for registering reactive value publishers.
///
/// Registration must happen while constructing the app. BSN templates only
/// attach listeners; they never mutate a schedule at runtime.
pub trait ReactiveAppExt {
    /// Publishes changes to resource `T` from its singleton resource entity.
    ///
    /// This uses `Res<T>` change ticks, so the unchanged-frame check is O(1).
    /// Insertion counts as a change, as does mutable dereferencing. Prefer
    /// Bevy's `set_if_neq` and `insert_resource_if_neq` APIs to suppress no-op
    /// notifications.
    fn track_resource<T>(&mut self) -> &mut Self
    where
        T: Resource + Clone;

    /// Publishes changes to ordinary component `T` instances.
    ///
    /// Resource entities are excluded. This uses Bevy's non-archetypal
    /// `Changed<T>` filter and should be reserved for presentation-scale work.
    /// A type cannot be tracked as both a resource and an ordinary component;
    /// use distinct wrapper types when both storage roles are needed.
    fn track_component<T>(&mut self) -> &mut Self
    where
        T: Component + Clone;

    /// Registers resource tracking and adds one global observer.
    fn observe_resource<T, B, M, I>(&mut self, observer: I) -> &mut Self
    where
        T: Resource + Clone,
        B: Bundle,
        I: IntoObserverSystem<ValueChanged<T>, B, M>;

    /// Registers component tracking and adds one global observer.
    fn observe_component<T, B, M, I>(&mut self, observer: I) -> &mut Self
    where
        T: Component + Clone,
        B: Bundle,
        I: IntoObserverSystem<ValueChanged<T>, B, M>;
}

impl ReactiveAppExt for App {
    fn track_resource<T>(&mut self) -> &mut Self
    where
        T: Resource + Clone,
    {
        if register_source::<T>(self.world_mut(), SourceKind::Resource) {
            self.add_systems(
                Last,
                publish_resource_changes::<T>.in_set(ReactiveSet::Publish),
            );
        }
        self
    }

    fn track_component<T>(&mut self) -> &mut Self
    where
        T: Component + Clone,
    {
        if register_source::<T>(self.world_mut(), SourceKind::Component) {
            self.add_systems(
                Last,
                publish_component_changes::<T>.in_set(ReactiveSet::Publish),
            );
        }
        self
    }

    fn observe_resource<T, B, M, I>(&mut self, observer: I) -> &mut Self
    where
        T: Resource + Clone,
        B: Bundle,
        I: IntoObserverSystem<ValueChanged<T>, B, M>,
    {
        self.track_resource::<T>().add_observer(observer)
    }

    fn observe_component<T, B, M, I>(&mut self, observer: I) -> &mut Self
    where
        T: Component + Clone,
        B: Bundle,
        I: IntoObserverSystem<ValueChanged<T>, B, M>,
    {
        self.track_component::<T>().add_observer(observer)
    }
}

fn register_source<T: 'static>(world: &mut World, kind: SourceKind) -> bool {
    world.init_resource::<ReactiveRegistry>();
    world.resource_mut::<ReactiveRegistry>().register::<T>(kind)
}

fn source_is_registered<T: 'static>(world: &World, kind: SourceKind) -> bool {
    world
        .get_resource::<ReactiveRegistry>()
        .is_some_and(|registry| registry.contains::<T>(kind))
}

fn publish_resource_changes<T>(
    value: Option<Res<T>>,
    sources: Query<Entity, (With<T>, With<IsResource>, Allow<Disabled>)>,
    mut commands: Commands,
) where
    T: Resource + Clone,
{
    let Some(value) = value else {
        return;
    };
    if !value.is_changed() {
        return;
    }
    let Ok(source) = sources.single() else {
        return;
    };

    commands.trigger(ValueChanged::new(source, Arc::new(value.as_ref().clone())));
}

fn publish_component_changes<T>(
    values: Query<(Entity, &T), (BevyChanged<T>, Without<IsResource>, Allow<Disabled>)>,
    mut commands: Commands,
) where
    T: Component + Clone,
{
    for (source, value) in &values {
        commands.trigger(ValueChanged::new(source, Arc::new(value.clone())));
    }
}

/// BSN template returned by [`watch_resource`].
pub struct WatchResource<T, B, M, I>
where
    T: Resource + Clone,
    B: Bundle,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M>,
{
    observer: I,
    marker: PhantomData<ObserverMarker<T, B, M>>,
}

impl<T, B, M, I> Template for WatchResource<T, B, M, I>
where
    T: Resource + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M> + Clone,
{
    type Output = ();

    fn build_template(&self, context: &mut TemplateContext) -> bevy::ecs::error::Result<()> {
        let registered = context
            .entity
            .world_scope(|world| source_is_registered::<T>(world, SourceKind::Resource));
        if !registered {
            return Err(format!(
                "resource `{}` is not tracked; call `app.track_resource::<{}>()` during app construction",
                type_name::<T>(),
                type_name::<T>(),
            )
            .into());
        }

        let exists = context
            .entity
            .world_scope(|world| world.contains_resource::<T>());
        if !exists {
            return Err(format!(
                "resource `{}` must exist before resolving `watch_resource(...)`",
                type_name::<T>(),
            )
            .into());
        }

        let Some(source) = context.resource_entity::<T>() else {
            return Err(format!(
                "resource `{}` must exist before resolving `watch_resource(...)`",
                type_name::<T>(),
            )
            .into());
        };

        let insert_relationship =
            ensure_subscription_source_available::<ValueChanged<T>>(context, source)?;
        context
            .entity
            .world_scope(install_subscription::<ValueChanged<T>>);
        if insert_relationship {
            context
                .entity
                .insert(SubscribedTo::<ValueChanged<T>>::new(source));
        }
        context.entity.observe(self.observer.clone());
        Ok(())
    }

    fn clone_template(&self) -> Self {
        Self {
            observer: self.observer.clone(),
            marker: PhantomData,
        }
    }
}

impl<T, B, M, I> Scene for WatchResource<T, B, M, I>
where
    T: Resource + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M> + Clone + Send + Sync,
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

/// Watches resource `T` from the current BSN entity.
///
/// Call [`ReactiveAppExt::track_resource`] during app construction and ensure
/// the resource exists before this scene resolves. The helper does not replay
/// current state to late listeners: initialize the node from `Res<T>` when
/// building the scene, then use the reaction for later changes.
pub fn watch_resource<T, B, M, I>(observer: I) -> WatchResource<T, B, M, I>
where
    T: Resource + Clone,
    B: Bundle,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M>,
{
    WatchResource {
        observer,
        marker: PhantomData,
    }
}

struct WatchComponent<T, B, M, I>
where
    T: Component + Clone,
    B: Bundle,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M>,
{
    source: EntityTemplate,
    observer: I,
    marker: PhantomData<ObserverMarker<T, B, M>>,
}

impl<T, B, M, I> Template for WatchComponent<T, B, M, I>
where
    T: Component + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M> + Clone,
{
    type Output = ();

    fn build_template(&self, context: &mut TemplateContext) -> bevy::ecs::error::Result<()> {
        let registered = context
            .entity
            .world_scope(|world| source_is_registered::<T>(world, SourceKind::Component));
        if !registered {
            return Err(format!(
                "component `{}` is not tracked; call `app.track_component::<{}>()` during app construction",
                type_name::<T>(),
                type_name::<T>(),
            )
            .into());
        }

        let source = self.source.build_template(context)?;
        let insert_relationship =
            ensure_subscription_source_available::<ValueChanged<T>>(context, source)?;
        context
            .entity
            .world_scope(install_subscription::<ValueChanged<T>>);
        if insert_relationship {
            context
                .entity
                .insert(SubscribedTo::<ValueChanged<T>>::new(source));
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

impl<T, B, M, I> Scene for WatchComponent<T, B, M, I>
where
    T: Component + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M> + Clone + Send + Sync,
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

/// Watches component `T` on `source` from the current BSN entity.
///
/// Call [`ReactiveAppExt::track_component`] during app construction. Unlike
/// resource watching, the source is resolved through the scene's entity
/// references and may be created by the same BSN scene.
pub fn watch_component<T, B, M, I>(source: EntityTemplate, observer: I) -> impl Scene
where
    T: Component + Clone,
    B: Bundle,
    M: 'static,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, B, M> + Clone + Send + Sync + 'static,
{
    WatchComponent {
        source,
        observer,
        marker: PhantomData,
    }
}
