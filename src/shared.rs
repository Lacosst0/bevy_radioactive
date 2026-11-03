//! Small pieces shared by the `reactive`, `hooks` and `view` runtimes.

// Each item is used by some of the runtimes, so with only some enabled, the
// rest goes unused.
#![cfg_attr(
    not(all(feature = "reactive", feature = "hooks", feature = "view")),
    allow(dead_code)
)]

use std::{
    marker::PhantomData,
    sync::{Mutex, MutexGuard, PoisonError},
};

use bevy::{
    ecs::{
        entity::EntityHashSet, error::ErrorContext, lifecycle::HookContext, world::DeferredWorld,
    },
    prelude::*,
    scene::{SceneList, WorldSceneExt},
};

/// Locks `mutex`, ignoring poisoning: a panic elsewhere doesn't stop updates.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Host entity of a reactive region: its children are the region's current
/// content. With the `ui` feature it is a `GhostNode`, so its children are
/// laid out as children of its parent.
#[derive(Component, Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "ui", require(bevy::ui::experimental::GhostNode))]
#[cfg_attr(feature = "ui", component(on_add = strip_transform))]
pub struct Region;

/// `GhostNode` requires `Transform`, which UI layout doesn't use and Bevy
/// warns about under a UI parent.
#[cfg(feature = "ui")]
fn strip_transform(mut world: DeferredWorld, context: HookContext) {
    world
        .commands()
        .entity(context.entity)
        .remove::<(Transform, GlobalTransform)>();
}

/// Makes `host` layout-transparent like a [`Region`], unless it is a UI
/// node already.
#[cfg_attr(not(feature = "ui"), allow(unused_variables))]
pub(crate) fn make_layout_transparent(world: &mut World, host: Entity) {
    #[cfg(feature = "ui")]
    if let Ok(mut entity) = world.get_entity_mut(host)
        && !entity.contains::<bevy::ui::Node>()
        && !entity.contains::<bevy::ui::experimental::GhostNode>()
    {
        entity.insert(bevy::ui::experimental::GhostNode);
        entity.remove::<(Transform, GlobalTransform)>();
    }
}

/// Passes a spawn error to the world's fallback error handler.
pub(crate) fn report(world: &World, error: impl Into<BevyError>, name: &'static str) {
    let handler = world.fallback_error_handler();
    handler(error.into(), ErrorContext::Command { name: name.into() });
}

/// Spawns `list` as children of `parent`.
pub(crate) fn spawn_children(
    world: &mut World,
    parent: Entity,
    list: Box<dyn SceneList>,
    name: &'static str,
) -> Vec<Entity> {
    match world.spawn_scene_list(list) {
        Ok(entities) => {
            for &entity in &entities {
                world.entity_mut(entity).insert(ChildOf(parent));
            }
            entities
        }
        Err(error) => {
            report(world, error, name);
            Vec::new()
        }
    }
}

pub(crate) fn despawn_all(world: &mut World, entities: &[Entity]) {
    for &entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
}

/// Entities with component `C`, kept by `C`'s hooks
/// (`#[component(on_add = add_host::<Self>, on_remove = remove_host::<Self>)]`)
/// so a runtime visits only them.
#[derive(Resource)]
pub(crate) struct Hosts<C>(pub(crate) EntityHashSet, PhantomData<fn() -> C>);

impl<C> Default for Hosts<C> {
    fn default() -> Self {
        Self(EntityHashSet::default(), PhantomData)
    }
}

impl<C: 'static> Hosts<C> {
    pub(crate) fn snapshot(world: &World) -> Vec<Entity> {
        world.resource::<Self>().0.iter().copied().collect()
    }
}

pub(crate) fn add_host<C: 'static>(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<Hosts<C>>() {
        hosts.0.insert(context.entity);
    }
}

pub(crate) fn remove_host<C: 'static>(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<Hosts<C>>() {
        hosts.0.remove(&context.entity);
    }
}

/// Runs `round` until it reports no work, up to `max_rounds` times, then
/// warns that `what` is still changing.
pub(crate) fn settle(
    world: &mut World,
    max_rounds: usize,
    mut round: impl FnMut(&mut World) -> usize,
    what: &str,
) {
    for _ in 0..max_rounds {
        if round(world) == 0 {
            return;
        }
    }
    warn!(
        "{what} still changing after {max_rounds} rounds; the rest runs next frame. Something \
         may be writing a value it reads."
    );
}
