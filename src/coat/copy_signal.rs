//! `Copy` signal handles whose values are owned by entities.

use std::{
    any::{Any, type_name},
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    sync::{PoisonError, RwLock},
};

use bevy::{
    ecs::{
        component::{Immutable, StorageType},
        lifecycle::{ComponentHook, HookContext},
        template::{Template, TemplateContext},
        world::DeferredWorld,
    },
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

use super::{Signal, Step, Value};

// ---------------------------------------------------------------------------
// Arena
// ---------------------------------------------------------------------------

struct Slot {
    generation: u32,
    /// A `Signal<T>`, or `None` once disposed.
    signal: Option<Box<dyn Any + Send + Sync>>,
    /// The entity holding the signal's [`SignalCell`], once owned.
    cell: Option<Entity>,
}

/// Every live [`CopySignal`], indexed by handle. Handles carry a
/// generation, so a handle to a disposed signal never reaches a newer one
/// that reuses its slot. The arena is process-wide, so handles from
/// different `World`s never collide.
struct Arena {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

static ARENA: RwLock<Arena> = RwLock::new(Arena {
    slots: Vec::new(),
    free: Vec::new(),
});

fn read<R>(f: impl FnOnce(&Arena) -> R) -> R {
    f(&ARENA.read().unwrap_or_else(PoisonError::into_inner))
}

fn write<R>(f: impl FnOnce(&mut Arena) -> R) -> R {
    f(&mut ARENA.write().unwrap_or_else(PoisonError::into_inner))
}

impl Arena {
    fn insert(&mut self, signal: Box<dyn Any + Send + Sync>) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.signal = Some(signal);
            (index, slot.generation)
        } else {
            let index = u32::try_from(self.slots.len()).expect("fewer than 2^32 copy signals");
            self.slots.push(Slot {
                generation: 0,
                signal: Some(signal),
                cell: None,
            });
            (index, 0)
        }
    }

    fn slot(&self, index: u32, generation: u32) -> Option<&Slot> {
        self.slots
            .get(index as usize)
            .filter(|slot| slot.generation == generation && slot.signal.is_some())
    }

    fn slot_mut(&mut self, index: u32, generation: u32) -> Option<&mut Slot> {
        self.slots
            .get_mut(index as usize)
            .filter(|slot| slot.generation == generation && slot.signal.is_some())
    }

    /// Frees a slot; its value drops once no read in progress holds it.
    fn release(&mut self, index: u32, generation: u32) -> Option<Box<dyn Any + Send + Sync>> {
        let slot = self.slot_mut(index, generation)?;
        let signal = slot.signal.take();
        slot.cell = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index);
        signal
    }
}

// ---------------------------------------------------------------------------
// CopySignal
// ---------------------------------------------------------------------------

/// A [`Signal`] behind a `Copy` handle, owned by an entity.
///
/// The handle is two integers, so closures capture it by copy: several
/// `on(..)` handlers, `bsn!` closures and `coat!` reads can use one signal
/// without `clone`s. Reads and writes behave exactly like [`Signal`]'s:
/// `get`/`with` inside an effect, `computed`, a `coat!` region or a
/// `webby` render subscribe it, and `set` pushes those readers onto the
/// runtime's dirty queue.
///
/// The value lives in a [`SignalCell<T>`] component on its own entity, which
/// a [`SignalOf`] relationship links to an owner and despawns with it (like
/// `ChildOf`), so the value is visible to inspectors and freed with the
/// entity that created it. Create one with:
///
/// - [`SignalCommandsExt::spawn_signal`] on `Commands` or `World`, owned by
///   a given entity;
/// - [`Cx::use_copy_signal`](crate::webby::Cx::use_copy_signal), owned by
///   the component's host;
/// - [`copy_signal`], unowned until a scene places [`owns`] on an entity.
///   An unowned signal lives until [`dispose`](Self::dispose).
///
/// Once the owner despawns (or `dispose` is called), the handle is dead:
/// [`try_get`](Self::try_get), [`try_with`](Self::try_with) and
/// [`try_set`](Self::try_set) return `None`/`false`, and the other methods
/// panic with a message naming the type. A new signal never reuses a dead
/// handle's identity.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_radioactive::prelude::*;
///
/// let mut world = World::new();
/// let owner = world.spawn_empty().id();
/// let count = world.spawn_signal(owner, 0);
/// let (a, b) = (move || count.increment(), move || count.get());
/// a();
/// assert_eq!(b(), 1);
/// world.despawn(owner);
/// assert_eq!(count.try_get(), None);
/// ```
pub struct CopySignal<T> {
    index: u32,
    generation: u32,
    marker: PhantomData<fn() -> T>,
}

/// Creates an unowned [`CopySignal`] holding the value `init` returns.
///
/// Give it an owner by placing [`owns`] on an entity of the scene that uses
/// it, or call [`CopySignal::dispose`] when done; otherwise it lives as long
/// as the process.
///
/// ```ignore
/// fn counter() -> impl Scene {
///     let count = copy_signal(|| 0);
///     bsn! {
///         Button
///         owns(count)
///         on(move |_: On<Pointer<Click>>| count.increment())
///         Children [ Text("+") ]
///     }
/// }
/// ```
pub fn copy_signal<T: Send + Sync + 'static>(init: impl FnOnce() -> T) -> CopySignal<T> {
    CopySignal::new(init())
}

impl<T: Send + Sync + 'static> CopySignal<T> {
    /// Creates an unowned signal holding `value`. See [`copy_signal`].
    #[must_use]
    pub fn new(value: T) -> Self {
        let (index, generation) = write(|arena| arena.insert(Box::new(Signal::new(value))));
        Self {
            index,
            generation,
            marker: PhantomData,
        }
    }

    /// The shared [`Signal`] behind this handle, or `None` once disposed.
    ///
    /// Useful to pass it where a `Signal<T>` is expected, such as a
    /// [`Value<T>`] prop.
    #[must_use]
    pub fn try_signal(self) -> Option<Signal<T>> {
        read(|arena| {
            arena
                .slot(self.index, self.generation)
                .and_then(|slot| slot.signal.as_ref())
                .and_then(|signal| signal.downcast_ref::<Signal<T>>())
                .cloned()
        })
    }

    /// The shared [`Signal`] behind this handle.
    ///
    /// # Panics
    ///
    /// If the signal was disposed, which happens when its owner despawns.
    #[must_use]
    pub fn signal(self) -> Signal<T> {
        self.try_signal().unwrap_or_else(|| self.dead())
    }

    #[cold]
    #[track_caller]
    fn dead(self) -> ! {
        panic!(
            "`CopySignal<{}>` used after it was disposed: its owner entity was despawned, or \
             `dispose` was called. Use `try_get`, `try_with` or `try_set` where the owner may \
             be gone.",
            type_name::<T>()
        )
    }

    /// Whether the signal still exists.
    #[must_use]
    pub fn is_alive(self) -> bool {
        read(|arena| arena.slot(self.index, self.generation).is_some())
    }

    /// The entity holding the signal's [`SignalCell`], once it has an owner.
    #[must_use]
    pub fn cell(self) -> Option<Entity> {
        read(|arena| arena.slot(self.index, self.generation)?.cell)
    }

    /// Reads the value through `f` and tracks the read, or returns `None`
    /// once the signal is disposed.
    pub fn try_with<R>(self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.try_signal().map(|signal| signal.with(f))
    }

    /// Reads the value through `f` and tracks the read.
    ///
    /// # Panics
    ///
    /// If the signal was disposed, and like [`Signal::with`].
    #[track_caller]
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        self.signal().with(f)
    }

    /// Reads the value through `f` without tracking.
    ///
    /// # Panics
    ///
    /// If the signal was disposed, and like [`Signal::with_untracked`].
    #[track_caller]
    pub fn with_untracked<R>(self, f: impl FnOnce(&T) -> R) -> R {
        self.signal().with_untracked(f)
    }

    /// Clones the value and tracks the read, or returns `None` once the
    /// signal is disposed.
    #[must_use]
    pub fn try_get(self) -> Option<T>
    where
        T: Clone,
    {
        self.try_with(T::clone)
    }

    /// Clones the value and tracks the read.
    ///
    /// # Panics
    ///
    /// If the signal was disposed (its owner despawned).
    #[must_use]
    #[track_caller]
    pub fn get(self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// Clones the value without tracking.
    ///
    /// # Panics
    ///
    /// If the signal was disposed.
    #[must_use]
    #[track_caller]
    pub fn get_untracked(self) -> T
    where
        T: Clone,
    {
        self.with_untracked(T::clone)
    }

    /// Replaces the value and notifies readers. Returns `false`, doing
    /// nothing, once the signal is disposed.
    pub fn try_set(self, value: T) -> bool {
        self.try_signal().map(|signal| signal.set(value)).is_some()
    }

    /// Replaces the value and notifies readers.
    ///
    /// # Panics
    ///
    /// If the signal was disposed, and like [`Signal::set`].
    #[track_caller]
    pub fn set(self, value: T) {
        self.signal().set(value);
    }

    /// Replaces the value, notifying readers only if it changed. Returns
    /// whether it changed.
    ///
    /// # Panics
    ///
    /// If the signal was disposed, and like [`Signal::set_if_neq`].
    #[track_caller]
    pub fn set_if_neq(self, value: T) -> bool
    where
        T: PartialEq,
    {
        self.signal().set_if_neq(value)
    }

    /// Mutates the value in place and notifies readers, or returns `None`
    /// once the signal is disposed.
    pub fn try_update<R>(self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.try_signal().map(|signal| signal.update(f))
    }

    /// Mutates the value in place and notifies readers.
    ///
    /// # Panics
    ///
    /// If the signal was disposed, and like [`Signal::update`].
    #[track_caller]
    pub fn update<R>(self, f: impl FnOnce(&mut T) -> R) -> R {
        self.signal().update(f)
    }

    /// Marks the signal changed without modifying it.
    ///
    /// # Panics
    ///
    /// If the signal was disposed.
    #[track_caller]
    pub fn notify(self) {
        self.signal().notify();
    }

    /// Spawns this signal's cell owned by `owner`, unless it has one.
    pub(crate) fn own(self, world: &mut World, owner: Entity) -> bool {
        own(world, self, owner)
    }

    /// Frees the signal now. Its owner relationship, if any, is left for
    /// the owner's despawn to clean up. Later uses of any copy of the
    /// handle see a dead signal.
    pub fn dispose(self) {
        drop(write(|arena| arena.release(self.index, self.generation)));
    }
}

impl<T: Step + Copy + Send + Sync + 'static> CopySignal<T> {
    /// Adds one and notifies readers.
    ///
    /// # Panics
    ///
    /// If the signal was disposed.
    #[track_caller]
    pub fn increment(self) {
        self.signal().increment();
    }

    /// Subtracts one and notifies readers.
    ///
    /// # Panics
    ///
    /// If the signal was disposed.
    #[track_caller]
    pub fn decrement(self) {
        self.signal().decrement();
    }
}

impl<T> Clone for CopySignal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for CopySignal<T> {}

/// Handles are equal when they refer to the same signal.
impl<T> PartialEq for CopySignal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for CopySignal<T> {}

impl<T> Hash for CopySignal<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.index, self.generation).hash(state);
    }
}

impl<T> fmt::Debug for CopySignal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopySignal")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// # Panics
///
/// If the signal was disposed.
impl<T: Send + Sync + 'static> From<CopySignal<T>> for Value<T> {
    fn from(signal: CopySignal<T>) -> Self {
        Self::Signal(signal.signal())
    }
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

/// Links the entity holding a [`SignalCell`] to the entity that owns it.
/// Despawning the owner despawns the cell and disposes the signal.
#[derive(Component, Debug, PartialEq, Eq)]
#[relationship(relationship_target = OwnedSignals)]
pub struct SignalOf(#[entities] pub Entity);

/// The [`CopySignal`] cells an entity owns; despawned with it.
#[derive(Component, Debug, Default, PartialEq, Eq)]
#[relationship_target(relationship = SignalOf, linked_spawn)]
pub struct OwnedSignals(Vec<Entity>);

/// The value of a [`CopySignal`], held by an entity linked to its owner by
/// [`SignalOf`]. Removing it (usually by the owner despawning) disposes the
/// signal.
pub struct SignalCell<T: Send + Sync + 'static> {
    signal: Signal<T>,
    handle: CopySignal<T>,
}

impl<T: Send + Sync + 'static> SignalCell<T> {
    /// The handle of the signal this cell holds.
    #[must_use]
    pub fn handle(&self) -> CopySignal<T> {
        self.handle
    }

    /// The signal this cell holds.
    #[must_use]
    pub fn signal(&self) -> &Signal<T> {
        &self.signal
    }
}

impl<T: fmt::Debug + Send + Sync + 'static> fmt::Debug for SignalCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SignalCell").field(&self.signal).finish()
    }
}

impl<T: Send + Sync + 'static> Component for SignalCell<T> {
    const STORAGE_TYPE: StorageType = StorageType::SparseSet;
    type Mutability = Immutable;

    fn on_remove() -> Option<ComponentHook> {
        Some(|world: DeferredWorld, context: HookContext| {
            if let Some(cell) = world.get::<Self>(context.entity) {
                let handle = cell.handle;
                drop(write(|arena| {
                    arena.release(handle.index, handle.generation)
                }));
            }
        })
    }
}

/// Spawns the cell of `signal` owned by `owner`, unless it already has one.
/// Returns whether it did.
fn own<T: Send + Sync + 'static>(world: &mut World, signal: CopySignal<T>, owner: Entity) -> bool {
    let Some(inner) = signal.try_signal() else {
        return false;
    };
    if signal.cell().is_some() {
        return false;
    }
    let cell = world
        .spawn((
            SignalCell {
                signal: inner,
                handle: signal,
            },
            SignalOf(owner),
        ))
        .id();
    write(|arena| {
        if let Some(slot) = arena.slot_mut(signal.index, signal.generation) {
            slot.cell = Some(cell);
        }
    });
    true
}

/// Creates [`CopySignal`]s owned by an entity.
pub trait SignalCommandsExt {
    /// Creates a signal holding `value`, owned by `owner`: it is disposed
    /// when `owner` despawns. The handle is usable right away; with
    /// `Commands`, the cell entity spawns when the commands are applied.
    fn spawn_signal<T: Send + Sync + 'static>(&mut self, owner: Entity, value: T) -> CopySignal<T>;
}

impl SignalCommandsExt for World {
    fn spawn_signal<T: Send + Sync + 'static>(&mut self, owner: Entity, value: T) -> CopySignal<T> {
        let signal = CopySignal::new(value);
        own(self, signal, owner);
        signal
    }
}

impl SignalCommandsExt for Commands<'_, '_> {
    fn spawn_signal<T: Send + Sync + 'static>(&mut self, owner: Entity, value: T) -> CopySignal<T> {
        let signal = CopySignal::new(value);
        self.queue(move |world: &mut World| {
            if world.get_entity(owner).is_ok() {
                own(world, signal, owner);
            } else {
                signal.dispose();
            }
        });
        signal
    }
}

/// BSN template that makes the current entity the owner of an unowned
/// [`CopySignal`], returned by [`owns`].
pub struct Owns<T>(CopySignal<T>);

/// Makes the entity this is placed on own `signal`, so the signal is
/// disposed when that entity despawns: `bsn! { Node owns(count) .. }`.
///
/// A signal has one owner; placing `owns` for a signal that already has
/// one (for example by spawning the same scene twice) does nothing.
pub fn owns<T: Send + Sync + 'static>(signal: CopySignal<T>) -> Owns<T> {
    Owns(signal)
}

impl<T: Send + Sync + 'static> Template for Owns<T> {
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let (signal, owner) = (self.0, context.entity.id());
        context
            .entity
            .world_scope(|world| own(world, signal, owner));
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self(self.0)
    }
}

impl<T: Send + Sync + 'static> Scene for Owns<T> {
    fn resolve(
        self,
        _context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        scene.push_template(self);
        Ok(())
    }
}
