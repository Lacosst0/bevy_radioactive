//! React to components and resources changing on other entities.
//!
//! [`bind`] keeps a component of the declaring entity in sync with a value on
//! a source, and [`watch`] runs a handler instead:
//!
//! ```ignore
//! use bevy_radioactive::watch::*;
//!
//! app.add_plugins(WatchPlugin);
//!
//! bsn! {
//!     Children [
//!         #Player Health(100)
//!         --
//!         Text("")
//!         bind_or(
//!             #Player,
//!             |hp: &Health, text: &mut Text| **text = hp.0.to_string(),
//!             |text: &mut Text| **text = "DEAD".into(),
//!         )
//!         --
//!         Text("")
//!         bind(Singleton, |score: &Score, text: &mut Text| **text = score.0.to_string())
//!         --
//!         watch(#Player, |hp: Reaction<Health>, mut log: ResMut<Log>| log.push(hp.0))
//!     ]
//! }
//! ```
//!
//! A source is a `#Name`, an [`Entity`], [`Singleton`] for resource `T`, or a
//! list such as `[#A, #B]`. Resources are singleton components in Bevy, so one
//! API covers both.
//!
//! Insertion and mutable access count as changes; use `set_if_neq` to skip
//! no-op writes. Changes are published in [`WatchSystems`] in `PostUpdate`,
//! before UI layout. A reaction that writes another watched value is published
//! in the same frame, for up to 8 rounds.

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
        system::{FromInput, IntoObserverSystem, SystemInput},
        template::{EntityTemplate, Template, TemplateContext},
        world::DeferredWorld,
    },
    platform::collections::HashSet,
    prelude::*,
    scene::EmptySceneEffect,
};

/// Rounds of publishing per frame, so chained reactions settle in one frame.
const MAX_ROUNDS: usize = 8;

/// Publishes watched values each frame. Required by [`watch`] and [`bind`].
#[derive(Default)]
pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Watched>()
            .init_resource::<Published>()
            .init_schedule(Publish)
            .add_systems(PostUpdate, publish_all.in_set(WatchSystems));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            WatchSystems.before(bevy::ui::UiSystems::Prepare),
        );
    }
}

/// System set in `PostUpdate` that publishes watched values.
///
/// Order systems that write watched values in `PostUpdate`
/// `.before(WatchSystems)` to have them shown in the same frame.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WatchSystems;

/// App helper to watch a type up front.
pub trait WatchAppExt {
    /// Publishes [`ValueChanged<T>`] and [`ValueRemoved<T>`] for every entity
    /// with `T`, so global observers can listen to them. Adds [`WatchPlugin`]
    /// if needed.
    ///
    /// Scenes don't need this: [`watch`] and [`bind`] start tracking their
    /// type on first use.
    fn track<T: Component + Clone>(&mut self) -> &mut Self;
}

impl WatchAppExt for App {
    fn track<T: Component + Clone>(&mut self) -> &mut Self {
        if !self.is_plugin_added::<WatchPlugin>() {
            self.add_plugins(WatchPlugin);
        }
        track::<T>(self.world_mut());
        self
    }
}

/// Runs `handler` whenever `T` changes on `source`.
///
/// The handler receives a [`Reaction<T>`]: it dereferences to the new value,
/// `target()` is the declaring entity and `source()` the entity that owns `T`.
/// The current value is not replayed when the scene spawns.
pub fn watch<T, S, I, M>(source: S, handler: I) -> impl Template<Output = EmptySceneEffect>
where
    T: Component + Clone,
    S: ValueSource<T>,
    I: IntoObserverSystem<Delivery<ValueChanged<T>>, M> + Clone,
    M: 'static,
{
    Watch {
        source,
        handler,
        marker: PhantomData::<fn() -> (ValueChanged<T>, M)>,
    }
}

/// Runs `handler` when `T` is removed from `source`, or `source` despawns.
///
/// The handler receives a [`Removal<T>`].
pub fn watch_removed<T, S, I, M>(source: S, handler: I) -> impl Template<Output = EmptySceneEffect>
where
    T: Component + Clone,
    S: ValueSource<T>,
    I: IntoObserverSystem<Delivery<ValueRemoved<T>>, M> + Clone,
    M: 'static,
{
    Watch {
        source,
        handler,
        marker: PhantomData::<fn() -> (ValueRemoved<T>, M)>,
    }
}

/// Writes component `C` of the current BSN entity whenever `T` changes on
/// `source`.
///
/// ```ignore
/// Text("")
/// bind(#Player, |hp: &Health, text: &mut Text| **text = hp.0.to_string())
/// ```
pub fn bind<T, C, S, F>(source: S, write: F) -> impl Template<Output = EmptySceneEffect>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: ValueSource<T>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
{
    Bind {
        source,
        write,
        removed: None::<fn(&mut C)>,
        marker: PhantomData,
    }
}

/// Like [`bind`], and also writes `C` with `removed` when `T` is removed from
/// `source` or it despawns.
///
/// ```ignore
/// Text("")
/// bind_or(
///     #Player,
///     |hp: &Health, text: &mut Text| **text = hp.0.to_string(),
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
    S: ValueSource<T>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: Fn(&mut C) + Clone + Send + Sync + 'static,
{
    Bind {
        source,
        write,
        removed: Some(removed),
        marker: PhantomData,
    }
}

struct Bind<T, C, S, F, R> {
    source: S,
    write: F,
    removed: Option<R>,
    marker: PhantomData<fn() -> (T, C)>,
}

impl<T, C, S, F, R> Template for Bind<T, C, S, F, R>
where
    T: Component + Clone,
    C: Component<Mutability = Mutable>,
    S: ValueSource<T>,
    F: Fn(&T, &mut C) + Clone + Send + Sync + 'static,
    R: Fn(&mut C) + Clone + Send + Sync + 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let write = self.write.clone();
        watch(
            self.source.clone_source(),
            move |change: Reaction<T>, mut targets: Query<&mut C, Allow<Disabled>>| {
                if let Ok(mut target) = targets.get_mut(change.target()) {
                    write(&change, &mut target);
                }
            },
        )
        .build_template(context)?;
        if let Some(removed) = self.removed.clone() {
            watch_removed(
                self.source.clone_source(),
                move |removal: Removal<T>, mut targets: Query<&mut C, Allow<Disabled>>| {
                    if let Ok(mut target) = targets.get_mut(removal.target()) {
                        removed(&mut target);
                    }
                },
            )
            .build_template(context)?;
        }
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            source: self.source.clone_source(),
            write: self.write.clone(),
            removed: self.removed.clone(),
            marker: PhantomData,
        }
    }
}

/// Source for resource `T`: `bind(Singleton, |score: &Score, ..| ..)`.
///
/// The resource must exist when the scene spawns.
#[derive(Clone, Copy, Debug, Default)]
pub struct Singleton;

/// Entities accepted as a watch source: a `#Name`, an [`Entity`],
/// [`Singleton`], or an array or `Vec` of sources.
pub trait ValueSource<T>: Send + Sync + 'static {
    #[doc(hidden)]
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result;

    #[doc(hidden)]
    #[must_use]
    fn clone_source(&self) -> Self
    where
        Self: Sized;
}

impl<T> ValueSource<T> for EntityTemplate {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        out.push(self.build_template(context)?);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        self.clone_template()
    }
}

impl<T> ValueSource<T> for Entity {
    fn resolve(&self, _: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        out.push(*self);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        *self
    }
}

impl<T: Resource> ValueSource<T> for Singleton {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        let exists = context
            .entity
            .world_scope(|world| world.contains_resource::<T>());
        let entity = context
            .resource_entity::<T>()
            .filter(|_| exists)
            .ok_or_else(|| {
                format!(
                    "resource `{}` must exist before a scene binds to it with `Singleton`",
                    type_name::<T>(),
                )
            })?;
        out.push(entity);
        Ok(())
    }

    fn clone_source(&self) -> Self {
        Self
    }
}

impl<T, S: ValueSource<T>, const N: usize> ValueSource<T> for [S; N] {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        self.iter()
            .try_for_each(|source| source.resolve(context, out))
    }

    fn clone_source(&self) -> Self {
        std::array::from_fn(|index| self[index].clone_source())
    }
}

impl<T, S: ValueSource<T>> ValueSource<T> for Vec<S> {
    fn resolve(&self, context: &mut TemplateContext, out: &mut Vec<Entity>) -> Result {
        self.iter()
            .try_for_each(|source| source.resolve(context, out))
    }

    fn clone_source(&self) -> Self {
        self.iter().map(ValueSource::clone_source).collect()
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Global event: `T` was inserted or changed on an entity.
///
/// Observe it with `app.track::<T>().add_observer(..)`. In scenes, use
/// [`watch`] or [`bind`].
pub struct ValueChanged<T: Component> {
    source: Entity,
    value: Arc<T>,
}

impl<T: Component> ValueChanged<T> {
    /// Entity that owns the value; for a resource, its singleton entity.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.source
    }
}

impl<T: Component> Deref for ValueChanged<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
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

impl<T: Component> Event for ValueChanged<T> {
    type Trigger<'a> = EntityTrigger;
}

impl<T: Component> EntityEvent for ValueChanged<T> {
    fn event_target(&self) -> Entity {
        self.source
    }
}

/// Global event: `T` was removed from an entity, or the entity despawned.
pub struct ValueRemoved<T: Component> {
    source: Entity,
    marker: PhantomData<fn() -> T>,
}

impl<T: Component> ValueRemoved<T> {
    /// Entity that lost the value.
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

/// A global event delivered to one watcher. Use [`Reaction`] or [`Removal`]
/// in handlers.
#[doc(hidden)]
#[derive(Clone, EntityEvent)]
pub struct Delivery<E: Clone + Send + Sync + 'static> {
    #[event_target]
    entity: Entity,
    target: Entity,
    event: E,
}

/// Handler input for [`watch`]. Dereferences to the new value.
pub struct Reaction<'w, 't, T: Component>(On<'w, 't, Delivery<ValueChanged<T>>>);

impl<T: Component> Reaction<'_, '_, T> {
    /// Entity that declared the watch.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.0.event().target
    }

    /// Entity that owns the value.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.0.event().event.source
    }
}

impl<T: Component> Deref for Reaction<'_, '_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0.event().event.value
    }
}

/// Handler input for [`watch_removed`].
pub struct Removal<'w, 't, T: Component>(On<'w, 't, Delivery<ValueRemoved<T>>>);

impl<T: Component> Removal<'_, '_, T> {
    /// Entity that declared the watch.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.0.event().target
    }

    /// Entity that lost the value.
    #[must_use]
    pub fn source(&self) -> Entity {
        self.0.event().event.source
    }
}

macro_rules! handler_input {
    ($input:ident, $event:ident) => {
        impl<T: Component> SystemInput for $input<'_, '_, T> {
            type Param<'i> = $input<'i, 'i, T>;
            type Inner<'i> = On<'i, 'i, Delivery<$event<T>>>;

            fn wrap(on: Self::Inner<'_>) -> Self::Param<'_> {
                $input(on)
            }
        }

        impl<T: Component> FromInput<On<'_, '_, Delivery<$event<T>>>> for $input<'_, '_, T> {
            fn from_inner<'i>(
                on: <On<'_, '_, Delivery<$event<T>>> as SystemInput>::Inner<'i>,
            ) -> Self::Inner<'i> {
                on
            }
        }
    };
}

handler_input!(Reaction, ValueChanged);
handler_input!(Removal, ValueRemoved);

// ---------------------------------------------------------------------------
// Watch template and routing
// ---------------------------------------------------------------------------

struct Watch<E, S, I, M> {
    source: S,
    handler: I,
    marker: PhantomData<fn() -> (E, M)>,
}

/// The watched type `T` of a [`ValueChanged`] or [`ValueRemoved`] event.
trait ValueEvent: Clone + Send + Sync + 'static {
    type Value: Component + Clone;
}

impl<T: Component + Clone> ValueEvent for ValueChanged<T> {
    type Value = T;
}

impl<T: Component + Clone> ValueEvent for ValueRemoved<T> {
    type Value = T;
}

impl<E, S, I, M> Template for Watch<E, S, I, M>
where
    E: ValueEvent,
    S: ValueSource<E::Value>,
    I: IntoObserverSystem<Delivery<E>, M> + Clone,
    M: 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        context.entity.world_scope(|world| {
            if world.contains_resource::<Watched>() {
                track::<E::Value>(world);
                Ok(())
            } else {
                Err(format!(
                    "watching `{}` needs `WatchPlugin`; add it to the app",
                    type_name::<E::Value>(),
                ))
            }
        })?;
        let mut sources = Vec::new();
        self.source.resolve(context, &mut sources)?;
        let listener = context.entity.id();
        context.entity.world_scope(|world| {
            for source in sources {
                world
                    .spawn((WatchOf(listener), Watches::<E::Value>::new(source)))
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

/// One delivery entity per (watcher, source) pair, pointing at the source.
#[derive(Component)]
#[relationship(relationship_target = Watchers<T>)]
struct Watches<T: Component> {
    #[relationship]
    source: Entity,
    marker: PhantomData<fn() -> T>,
}

impl<T: Component> Watches<T> {
    fn new(source: Entity) -> Self {
        Self {
            source,
            marker: PhantomData,
        }
    }
}

#[derive(Component)]
#[relationship_target(relationship = Watches<T>)]
struct Watchers<T: Component>(#[relationship] Vec<Entity>, PhantomData<fn() -> T>);

/// Owns delivery entities, so they despawn with the watcher.
#[derive(Component)]
#[relationship(relationship_target = WatchDeliveries)]
struct WatchOf(Entity);

#[derive(Component)]
#[relationship_target(relationship = WatchOf, linked_spawn)]
struct WatchDeliveries(Vec<Entity>);

/// Sends `event` to every watcher of `T` on `source`.
fn route<T: Component, E: Clone + Send + Sync + 'static>(
    world: &mut DeferredWorld,
    source: Entity,
    event: &E,
) {
    let (entities, mut commands) = world.entities_and_commands();
    let Some(watchers) = entities
        .get(source)
        .ok()
        .and_then(|entity| entity.get::<Watchers<T>>())
    else {
        return;
    };
    for &delivery in &watchers.0 {
        let target = entities
            .get(delivery)
            .ok()
            .and_then(|entity| entity.get::<WatchOf>())
            .map_or(delivery, |owner| owner.0);
        commands.trigger(Delivery {
            entity: delivery,
            target,
            event: event.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------

#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Publish;

/// Tracked types, and publishers to add to [`Publish`] before the next round.
#[derive(Default, Resource)]
struct Watched {
    types: HashSet<TypeId>,
    pending: Vec<fn(&mut Schedule)>,
}

#[derive(Default, Resource)]
struct Published(AtomicUsize);

/// Starts publishing `T`. Safe while scenes spawn mid-round: the publisher
/// joins the schedule before the next round.
fn track<T: Component + Clone>(world: &mut World) {
    let mut watched = world.resource_mut::<Watched>();
    if !watched.types.insert(TypeId::of::<T>()) {
        return;
    }
    watched.pending.push(|schedule| {
        schedule.add_systems(publish::<T>);
    });
    world.add_observer(publish_removed::<T>);
}

fn publish_all(world: &mut World) {
    for _ in 0..MAX_ROUNDS {
        let pending = std::mem::take(&mut world.resource_mut::<Watched>().pending);
        if !pending.is_empty() {
            world.schedule_scope(Publish, |_, schedule| {
                for add in pending {
                    add(schedule);
                }
            });
        }
        world.resource::<Published>().0.store(0, Ordering::Relaxed);
        let _ = world.try_run_schedule(Publish);
        world.flush();
        if world.resource::<Published>().0.load(Ordering::Relaxed) == 0 {
            return;
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are passed by value"
)]
fn publish<T: Component + Clone>(
    values: Query<(Entity, &T), (Changed<T>, Allow<Disabled>)>,
    published: Res<Published>,
    mut commands: Commands,
) {
    for (source, value) in &values {
        published.0.fetch_add(1, Ordering::Relaxed);
        let event = ValueChanged {
            source,
            value: Arc::new(value.clone()),
        };
        commands.queue(move |world: &mut World| {
            route::<T, _>(&mut DeferredWorld::from(&mut *world), source, &event);
            world.trigger(event);
        });
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn publish_removed<T: Component>(event: On<Remove<T>>, mut world: DeferredWorld) {
    let removed = ValueRemoved::<T> {
        source: event.entity,
        marker: PhantomData,
    };
    route::<T, _>(&mut world, removed.source, &removed);
    world.trigger(removed);
}
