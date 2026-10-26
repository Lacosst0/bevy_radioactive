//! Single selection by moving a marker component, like Solid's
//! `createSelector`.
//!
//! A [`Selection`] on a group entity, such as a list, remembers which entity
//! is selected, and that entity holds the marker component `M` ([`Selected`]
//! by default). Selecting another entity moves the marker, so readers of the
//! marker on the old and the new entity re-run, and every other row does
//! nothing: the cost is independent of the list's length.
//!
//! ```ignore
//! use bevy_radioactive::prelude::*;
//!
//! fn row(label: &'static str) -> impl Scene {
//!     bsn! {
//!         Button
//!         // select this row in the nearest ancestor's `Selection`
//!         on(|activate: On<Activate>, mut commands: Commands| {
//!             commands.entity(activate.entity).select::<Selected>();
//!         })
//!         // only the old and the new row re-run
//!         computed(|r: &Reads| BackgroundColor(if r.this_has::<Selected>() { ACCENT } else { IDLE }))
//!         Children [ Text({label}) ]
//!     }
//! }
//!
//! fn list() -> impl Scene {
//!     bsn! { Node Selection Children [ @row("a") -- @row("b") -- @row("c") ] }
//! }
//! ```
//!
//! The selection state lives on the entities themselves, so despawning the
//! selected entity clears the selection, and despawning the group removes
//! the marker from its selected entity. Nothing keyed by entity is left
//! behind.

use std::{any::TypeId, fmt, marker::PhantomData};

use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    platform::collections::HashSet,
    prelude::*,
};

/// Default selection marker, held by the entity a [`Selection`] selects.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selected;

/// The entity of a group that holds marker `M`, such as the selected row of
/// a list. Put it on the group entity and change it with
/// [`SelectCommandsExt::select`] or [`SelectEntityCommandsExt::select`].
///
/// Reading it through `Reads` (`r.get::<Selection>(list)`) depends on which
/// entity is selected; reading the marker (`r.this_has::<Selected>()`)
/// depends only on whether that one entity is selected.
#[derive(Component)]
#[component(on_add = install_marker_observer::<M>, on_remove = release_marker::<M>)]
pub struct Selection<M: Component + Default = Selected> {
    current: Option<Entity>,
    marker: PhantomData<fn() -> M>,
}

impl<M: Component + Default> Selection<M> {
    /// The selected entity, if any.
    #[must_use]
    pub fn current(&self) -> Option<Entity> {
        self.current
    }

    /// Whether `entity` is selected.
    #[must_use]
    pub fn is_selected(&self, entity: Entity) -> bool {
        self.current == Some(entity)
    }
}

impl<M: Component + Default> Default for Selection<M> {
    fn default() -> Self {
        Self {
            current: None,
            marker: PhantomData,
        }
    }
}

impl<M: Component + Default> Clone for Selection<M> {
    fn clone(&self) -> Self {
        Self {
            current: self.current,
            marker: PhantomData,
        }
    }
}

impl<M: Component + Default> PartialEq for Selection<M> {
    fn eq(&self, other: &Self) -> bool {
        self.current == other.current
    }
}

impl<M: Component + Default> fmt::Debug for Selection<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selection")
            .field("current", &self.current)
            .finish()
    }
}

/// Marker types whose removal observer is installed.
#[derive(Resource, Default)]
struct MarkerObservers(HashSet<TypeId>);

fn install_marker_observer<M: Component + Default>(mut world: DeferredWorld, _: HookContext) {
    world.commands().queue(|world: &mut World| {
        let installed = world
            .get_resource_or_init::<MarkerObservers>()
            .0
            .insert(TypeId::of::<M>());
        if installed {
            world.add_observer(clear_on_marker_removed::<M>);
        }
    });
}

/// The group went away: take the marker off its selected entity.
fn release_marker<M: Component + Default>(mut world: DeferredWorld, context: HookContext) {
    let current = world
        .get::<Selection<M>>(context.entity)
        .and_then(Selection::current);
    if let Some(current) = current {
        world.commands().queue(move |world: &mut World| {
            if let Ok(mut entity) = world.get_entity_mut(current) {
                entity.remove::<M>();
            }
        });
    }
}

/// The selected entity lost its marker, by despawn or by hand: clear the
/// selections that pointed at it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn clear_on_marker_removed<M: Component + Default>(
    event: On<Remove<M>>,
    mut selections: Query<&mut Selection<M>>,
) {
    for mut selection in &mut selections {
        if selection.current == Some(event.entity) {
            selection.current = None;
        }
    }
}

/// Selects `target` in `owner`'s [`Selection<M>`], or clears it on `None`.
///
/// Removes `M` from the previously selected entity and inserts it on
/// `target`. Does nothing when `target` is already selected. A missing
/// `target` clears the selection. Logs a warning when `owner` has no
/// `Selection<M>`.
pub fn select<M: Component + Default>(world: &mut World, owner: Entity, target: Option<Entity>) {
    let target = target.filter(|&target| world.get_entity(target).is_ok());
    let Some(mut selection) = world.get_mut::<Selection<M>>(owner) else {
        warn!(
            "bevy_radioactive: cannot select in {owner}: it has no `Selection<{}>`",
            std::any::type_name::<M>(),
        );
        return;
    };
    if selection.current == target {
        return;
    }
    // Update `current` first, so the marker's removal observer sees that the
    // old entity is no longer selected.
    let previous = std::mem::replace(&mut selection.current, target);
    if let Some(previous) = previous
        && let Ok(mut entity) = world.get_entity_mut(previous)
    {
        entity.remove::<M>();
    }
    if let Some(target) = target {
        world.entity_mut(target).insert(M::default());
    }
}

/// The nearest entity, starting at `entity` and walking up `ChildOf`, that
/// has a [`Selection<M>`].
fn nearest_selection<M: Component + Default>(world: &World, mut entity: Entity) -> Option<Entity> {
    loop {
        if world.get::<Selection<M>>(entity).is_some() {
            return Some(entity);
        }
        entity = world.get::<ChildOf>(entity)?.parent();
    }
}

/// [`Commands`] methods for [`Selection`]s.
pub trait SelectCommandsExt {
    /// Selects `target` in `owner`'s [`Selection<M>`], or clears the
    /// selection on `None`. See [`select`].
    ///
    /// ```ignore
    /// commands.select::<Selected>(list, Some(row));
    /// ```
    fn select<M: Component + Default>(&mut self, owner: Entity, target: Option<Entity>);
}

impl SelectCommandsExt for Commands<'_, '_> {
    fn select<M: Component + Default>(&mut self, owner: Entity, target: Option<Entity>) {
        self.queue(move |world: &mut World| select::<M>(world, owner, target));
    }
}

/// [`EntityCommands`] methods for [`Selection`]s.
pub trait SelectEntityCommandsExt {
    /// Selects this entity in the [`Selection<M>`] of its nearest ancestor
    /// that has one, such as the list a row belongs to.
    ///
    /// ```ignore
    /// on(|click: On<Pointer<Click>>, mut commands: Commands| {
    ///     commands.entity(click.entity).select::<Selected>();
    /// })
    /// ```
    fn select<M: Component + Default>(&mut self) -> &mut Self;

    /// Selects this entity like [`select`](Self::select), or clears the
    /// selection if this entity is already selected.
    fn toggle_select<M: Component + Default>(&mut self) -> &mut Self;
}

impl SelectEntityCommandsExt for EntityCommands<'_> {
    fn select<M: Component + Default>(&mut self) -> &mut Self {
        let entity = self.id();
        self.commands().queue(move |world: &mut World| {
            if let Some(owner) = world.get::<ChildOf>(entity).map(ChildOf::parent)
                && let Some(owner) = nearest_selection::<M>(world, owner)
            {
                select::<M>(world, owner, Some(entity));
            } else {
                warn!(
                    "bevy_radioactive: cannot select {entity}: no ancestor has a `Selection<{}>`",
                    std::any::type_name::<M>(),
                );
            }
        });
        self
    }

    fn toggle_select<M: Component + Default>(&mut self) -> &mut Self {
        let entity = self.id();
        self.commands().queue(move |world: &mut World| {
            let Some(owner) = world
                .get::<ChildOf>(entity)
                .map(ChildOf::parent)
                .and_then(|parent| nearest_selection::<M>(world, parent))
            else {
                return;
            };
            let selected = world
                .get::<Selection<M>>(owner)
                .is_some_and(|selection| selection.is_selected(entity));
            select::<M>(world, owner, (!selected).then_some(entity));
        });
        self
    }
}
