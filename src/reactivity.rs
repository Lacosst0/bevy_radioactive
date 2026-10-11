use std::{
    any::{TypeId, type_name},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use bevy::{
    ecs::{
        entity_disabling::Disabled,
        event::{EntityEvent, EntityTrigger, Event},
        query::Allow,
        resource::IsResource,
        system::IntoObserverSystem,
        template::{EntityTemplate, Template, TemplateContext},
    },
    platform::collections::HashMap,
    prelude::*,
};

use crate::subscription::{Subscribe, Subscription, SubscriptionEvent};

/// BSN template returned by [`watch_resource`] and [`watch_component`].
pub type Watch<T, S, I, M> = Subscribe<ValueChanged<T>, S, I, M>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceKind {
    Component,
    Resource,
}

/// Tracked types and whether each is tracked as a component or a resource.
#[derive(Default, Resource)]
struct ReactiveRegistry(HashMap<TypeId, SourceKind>);

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

/// Observer input used by [`watch_resource`] and [`watch_component`].
///
/// Dereferences through [`ValueChanged`] to the captured `T`. Use
/// `target()` for the watching entity and `source()` for the entity that
/// owns `T`.
pub type Reaction<'w, 't, T> = Subscription<'w, 't, ValueChanged<T>>;

/// App construction helpers for registering reactive value publishers.
///
/// Registration must happen while constructing the app. BSN templates only
/// attach listeners; they never mutate a schedule at runtime.
pub trait ReactiveAppExt {
    /// Publishes changes to resource `T` from its singleton resource entity.
    ///
    /// Insertion and mutable dereferencing count as changes. Prefer Bevy's
    /// `set_if_neq` and `insert_resource_if_neq` to suppress no-op
    /// notifications.
    fn track_resource<T: Resource + Clone>(&mut self) -> &mut Self;

    /// Publishes changes to ordinary component `T` instances.
    ///
    /// Resource entities are excluded. This uses Bevy's non-archetypal
    /// `Changed<T>` filter and should be reserved for presentation-scale work.
    /// A type cannot be tracked as both a resource and a component; use
    /// distinct wrapper types when both storage roles are needed.
    fn track_component<T: Component + Clone>(&mut self) -> &mut Self;

    /// Tracks resource `T` and adds one global observer.
    fn observe_resource<T: Resource + Clone, M>(
        &mut self,
        observer: impl IntoObserverSystem<ValueChanged<T>, M>,
    ) -> &mut Self;

    /// Tracks component `T` and adds one global observer.
    fn observe_component<T: Component + Clone, M>(
        &mut self,
        observer: impl IntoObserverSystem<ValueChanged<T>, M>,
    ) -> &mut Self;
}

impl ReactiveAppExt for App {
    fn track_resource<T: Resource + Clone>(&mut self) -> &mut Self {
        if register::<T>(self.world_mut(), SourceKind::Resource) {
            self.add_systems(Last, publish_resource::<T>.in_set(ReactiveSet::Publish));
        }
        self
    }

    fn track_component<T: Component + Clone>(&mut self) -> &mut Self {
        if register::<T>(self.world_mut(), SourceKind::Component) {
            self.add_systems(Last, publish_components::<T>.in_set(ReactiveSet::Publish));
        }
        self
    }

    fn observe_resource<T: Resource + Clone, M>(
        &mut self,
        observer: impl IntoObserverSystem<ValueChanged<T>, M>,
    ) -> &mut Self {
        self.track_resource::<T>().add_observer(observer)
    }

    fn observe_component<T: Component + Clone, M>(
        &mut self,
        observer: impl IntoObserverSystem<ValueChanged<T>, M>,
    ) -> &mut Self {
        self.track_component::<T>().add_observer(observer)
    }
}

/// Registers `T` as `kind`, returning `true` the first time.
fn register<T: 'static>(world: &mut World, kind: SourceKind) -> bool {
    world.init_resource::<ReactiveRegistry>();
    let Some(existing) = world
        .resource_mut::<ReactiveRegistry>()
        .0
        .insert(TypeId::of::<T>(), kind)
    else {
        return true;
    };
    assert!(
        existing == kind,
        "`{}` cannot be tracked as both a resource and a component; use distinct wrapper types",
        type_name::<T>(),
    );
    false
}

fn ensure_tracked<T: 'static>(context: &mut TemplateContext, kind: SourceKind) -> Result {
    let tracked = context.entity.world_scope(|world| {
        world
            .get_resource::<ReactiveRegistry>()
            .and_then(|registry| registry.0.get(&TypeId::of::<T>()))
            == Some(&kind)
    });
    if tracked {
        return Ok(());
    }
    let (noun, method) = match kind {
        SourceKind::Component => ("component", "track_component"),
        SourceKind::Resource => ("resource", "track_resource"),
    };
    Err(format!(
        "{noun} `{0}` is not tracked; call `app.{method}::<{0}>()` during app construction",
        type_name::<T>(),
    )
    .into())
}

fn publish_resource<T: Resource + Clone>(
    value: Option<Res<T>>,
    sources: Query<Entity, (With<T>, With<IsResource>, Allow<Disabled>)>,
    mut commands: Commands,
) {
    if let Some(value) = value.filter(Res::is_changed)
        && let Ok(source) = sources.single()
    {
        commands.trigger(ValueChanged {
            source,
            value: Arc::new(value.clone()),
        });
    }
}

fn publish_components<T: Component + Clone>(
    values: Query<(Entity, &T), (Changed<T>, Without<IsResource>, Allow<Disabled>)>,
    mut commands: Commands,
) {
    for (source, value) in &values {
        commands.trigger(ValueChanged {
            source,
            value: Arc::new(value.clone()),
        });
    }
}

/// Resolves to the singleton entity of tracked resource `T`.
pub struct ResourceSource<T>(PhantomData<fn() -> T>);

impl<T: Resource> Template for ResourceSource<T> {
    type Output = Entity;

    fn build_template(&self, context: &mut TemplateContext) -> Result<Entity> {
        ensure_tracked::<T>(context, SourceKind::Resource)?;
        let exists = context
            .entity
            .world_scope(|world| world.contains_resource::<T>());
        context
            .resource_entity::<T>()
            .filter(|_| exists)
            .ok_or_else(|| {
                format!(
                    "resource `{}` must exist before resolving `watch_resource(...)`",
                    type_name::<T>(),
                )
                .into()
            })
    }

    fn clone_template(&self) -> Self {
        Self(PhantomData)
    }
}

/// Resolves to a scene entity whose tracked component `T` is watched.
pub struct ComponentSource<T>(EntityTemplate, PhantomData<fn() -> T>);

impl<T: Component> Template for ComponentSource<T> {
    type Output = Entity;

    fn build_template(&self, context: &mut TemplateContext) -> Result<Entity> {
        ensure_tracked::<T>(context, SourceKind::Component)?;
        self.0.build_template(context)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone_template(), PhantomData)
    }
}

/// Watches resource `T` from the current BSN entity.
///
/// Call [`ReactiveAppExt::track_resource`] during app construction and ensure
/// the resource exists before this scene resolves. Current state is not
/// replayed: initialize the node from `Res<T>`, then react to later changes.
pub fn watch_resource<T, I, M>(observer: I) -> Watch<T, ResourceSource<T>, I, M>
where
    T: Resource + Clone,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, M>,
{
    Subscribe::new(ResourceSource::<T>(PhantomData), observer)
}

/// Watches component `T` on `source` from the current BSN entity.
///
/// Call [`ReactiveAppExt::track_component`] during app construction. The
/// source may be created by the same BSN scene.
pub fn watch_component<T, I, M>(
    source: EntityTemplate,
    observer: I,
) -> Watch<T, ComponentSource<T>, I, M>
where
    T: Component + Clone,
    I: IntoObserverSystem<SubscriptionEvent<ValueChanged<T>>, M>,
{
    Subscribe::new(ComponentSource::<T>(source, PhantomData), observer)
}
