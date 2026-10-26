//! Query monitors: a marker component that tracks whether an entity matches
//! a filter, after flecs monitors.
//!
//! [`MonitorAppExt::add_monitor`] keeps marker `M` on exactly the entities
//! that have every `with` component and none of the `without` components.
//! Observers on each term's `Add` and `Remove` re-check only the entity that
//! changed, so the cost is per structural change, never per frame.
//!
//! The marker then works with everything else in the crate and in Bevy:
//! `On<Add<M>>` and `On<Remove<M>>` observers are the "entered" and "left"
//! callbacks, `r.this_has::<M>()` in a `computed` or effect depends on it,
//! and `Query<.., With<M>>` lists the current members.
//!
//! ```
//! use bevy::prelude::*;
//! use bevy_radioactive::monitor::{Monitor, MonitorAppExt};
//!
//! #[derive(Component)]
//! struct Health(u32);
//! #[derive(Component)]
//! struct Dead;
//! #[derive(Component, Default)]
//! struct Alive;
//!
//! let mut app = App::new();
//! app.add_monitor::<Alive>(Monitor::new().with::<Health>().without::<Dead>())
//!     .add_observer(|left: On<Remove<Alive>>| info!("{} left", left.entity));
//! ```

use std::sync::Arc;

use bevy::{
    ecs::{component::ComponentId, query::QueryBuilder},
    prelude::*,
};

/// The filter of a monitor: components an entity must have, and components
/// it must not have. Built with [`with`](Self::with) and
/// [`without`](Self::without) and installed with
/// [`MonitorAppExt::add_monitor`].
#[derive(Clone, Default)]
pub struct Monitor {
    with: Vec<fn(&mut World) -> ComponentId>,
    without: Vec<fn(&mut World) -> ComponentId>,
}

impl Monitor {
    /// An empty filter; add at least one [`with`](Self::with) term.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requires component `T`.
    #[must_use]
    pub fn with<T: Component>(mut self) -> Self {
        self.with.push(World::register_component::<T>);
        self
    }

    /// Excludes entities with component `T`.
    #[must_use]
    pub fn without<T: Component>(mut self) -> Self {
        self.without.push(World::register_component::<T>);
        self
    }
}

/// A monitor's filter resolved to component ids.
struct Terms {
    with: Vec<ComponentId>,
    without: Vec<ComponentId>,
}

impl Terms {
    fn matches(&self, entity: &EntityWorldMut) -> bool {
        self.with.iter().all(|&id| entity.contains_id(id))
            && !self.without.iter().any(|&id| entity.contains_id(id))
    }
}

/// Inserts or removes `M` on `entity` to match `terms`.
fn update<M: Component + Default>(world: &mut World, entity: Entity, terms: &Terms) {
    let Ok(mut entity) = world.get_entity_mut(entity) else {
        // Despawned since the change was observed.
        return;
    };
    match (terms.matches(&entity), entity.contains::<M>()) {
        (true, false) => {
            entity.insert(M::default());
        }
        (false, true) => {
            entity.remove::<M>();
        }
        _ => {}
    }
}

/// App construction helpers for query monitors.
pub trait MonitorAppExt {
    /// Keeps marker `M` on the entities that match `monitor`, including the
    /// ones that already match.
    ///
    /// `M` is inserted when an entity starts matching and removed when it
    /// stops, so observe `On<Add<M>>` and `On<Remove<M>>` for enter and
    /// leave callbacks. An entity is re-checked after the command flush that
    /// changed it, because `Remove` observers run before the component is
    /// actually removed. Despawning an entity removes `M` with it.
    ///
    /// ```ignore
    /// app.add_monitor::<Targetable>(Monitor::new().with::<Enemy>().with::<Visible>().without::<Dead>());
    /// ```
    ///
    /// # Panics
    ///
    /// When `monitor` has no `with` term, or names `M` itself.
    fn add_monitor<M: Component + Default>(&mut self, monitor: Monitor) -> &mut Self;
}

impl MonitorAppExt for App {
    fn add_monitor<M: Component + Default>(&mut self, monitor: Monitor) -> &mut Self {
        add_monitor::<M>(self.world_mut(), &monitor);
        self
    }
}

fn add_monitor<M: Component + Default>(world: &mut World, monitor: &Monitor) {
    let marker = world.register_component::<M>();
    let terms = Terms {
        with: monitor.with.iter().map(|id| id(world)).collect(),
        without: monitor.without.iter().map(|id| id(world)).collect(),
    };
    assert!(
        !terms.with.is_empty(),
        "a monitor needs at least one `with` term"
    );
    assert!(
        !terms.with.contains(&marker) && !terms.without.contains(&marker),
        "a monitor's marker `{}` cannot be one of its own terms",
        std::any::type_name::<M>(),
    );
    let ids: Vec<ComponentId> = terms.with.iter().chain(&terms.without).copied().collect();
    let terms = Arc::new(terms);

    // Entities that already match.
    let mut existing = QueryBuilder::<Entity>::new(world);
    for &id in &terms.with {
        existing.with_id(id);
    }
    for &id in terms.without.iter().chain([&marker]) {
        existing.without_id(id);
    }
    let existing: Vec<Entity> = existing.build().iter(world).collect();
    for entity in existing {
        world.entity_mut(entity).insert(M::default());
    }

    let added = Arc::clone(&terms);
    world.spawn((
        Observer::new(move |event: On<Add<()>>, mut commands: Commands| {
            let (entity, terms) = (event.entity, Arc::clone(&added));
            commands.queue(move |world: &mut World| update::<M>(world, entity, &terms));
        })
        .with_components(ids.iter().copied()),
        Name::new(format!("monitor {}: add", std::any::type_name::<M>())),
    ));
    world.spawn((
        Observer::new(move |event: On<Remove<()>>, mut commands: Commands| {
            let (entity, terms) = (event.entity, Arc::clone(&terms));
            commands.queue(move |world: &mut World| update::<M>(world, entity, &terms));
        })
        .with_components(ids),
        Name::new(format!("monitor {}: remove", std::any::type_name::<M>())),
    ));
}
