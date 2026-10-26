//! Signals and a reactive template language for BSN, modelled on
//! [Topcoat](https://github.com/tokio-rs/topcoat).
//!
//! Topcoat's `view!` mixes markup with a small subset of Rust — `if`/`else`,
//! `for` with `#[key(...)]`, `match` and `let` — and marks the parts that
//! re-run when signals change with `$`. [`coat!`] does the same on top of
//! `bsn!`:
//!
//! ```ignore
//! use bevy_radioactive::coat::*;
//!
//! fn todo_list() -> impl Scene {
//!     let todos = signal(|| vec![Todo::new(1, "Write docs")]);
//!     let filter = signal(|| Filter::All);
//!     let open = signal(|| true);
//!
//!     coat! {
//!         Node { flex_direction: FlexDirection::Column }
//!         Children [
//!             Button
//!             on(move |_: On<Pointer<Click>>| open.set(!$open))
//!             Children [ Text("Toggle") ]
//!             --
//!             // rebuilt only when the branch changes; the text is patched
//!             if $open {
//!                 Text($(format!("{} todos", $&todos.len())))
//!             } else {
//!                 Text("hidden")
//!             }
//!             --
//!             // keyed: unchanged rows keep their entities
//!             #[key(todo.id)]
//!             for todo in $todos.into_iter().filter(|t| $filter.accepts(t)) {
//!                 Text({todo.title.clone()})
//!             }
//!             --
//!             match $filter {
//!                 Filter::All => Text("Showing all"),
//!                 Filter::Done => Text("Showing done"),
//!             }
//!         ]
//!     }
//! }
//! ```
//!
//! # Template language
//!
//! `coat!` accepts everything `bsn!` accepts. Inside relationship lists
//! (`Children [ ... ]`) an item may also be:
//!
//! | Item                                       | Meaning                                  |
//! | ------------------------------------------ | ---------------------------------------- |
//! | `if c { .. } else if c { .. } else { .. }` | conditional entities                     |
//! | `#[key(expr)] for pat in iter { .. }`      | one set of entities per item, keyed      |
//! | `for pat in iter { .. }`                   | one set per item, identified by position |
//! | `match value { pat => .., }`               | entities for the matching arm            |
//! | `let pat = expr;`                          | binding visible to the following items   |
//!
//! Bodies are themselves `--` separated lists. Control flow is plain Rust
//! evaluated once, unless its condition, iterator or scrutinee reads a
//! signal with `$`. Then it becomes a *region*: a [`Region`] entity (a
//! `GhostNode` with the `ui` feature, so it does not affect layout) whose
//! children are rebuilt when that value changes. An `if` region rebuilds
//! when the chosen branch changes. A `match` region rebuilds when the
//! matched arm changes, or a value its pattern bound and the arm uses;
//! otherwise the arm's patched components update in place. `for` regions
//! keep the entities of rows whose key and value are unchanged.
//!
//! `$name` reads signal `name` (it becomes `name.get()`, a clone of the
//! value), `$&name.method(..)` borrows it instead (it becomes
//! `name.with(|name| name.method(..))`), and `$(expr)` groups an
//! expression. A component written with `$` in its fields, such
//! as `Text($(format!("{}", $count)))` or `Node { width: px($width) }`, is
//! initialised from the current values and patched in place whenever those
//! signals change. Elsewhere `$` is an ordinary read. Every closure the
//! macro generates clones the signals it reads, so one signal can be used
//! in any number of places without manual clones.
//!
//! | Topcoat                              | `coat`                                       |
//! | ------------------------------------ | -------------------------------------------- |
//! | `let open = signal(cx, \|\| false)`  | `let open = signal(\|\| false)`              |
//! | `$(expr)`, `:attr=$(...)`            | `$name`, `$(expr)` in component fields       |
//! | `@click=$(\|_\| ...)`                | BSN `on(move \|_: On<Pointer<Click>>\| ...)` |
//! | `if` / `for` + `#[key]` / `match`    | the same, reactive when they read `$`        |
//! | `#[shard]`                           | [`shard`]                                    |
//! | `#[memoize]`                         | [`memo`]                                     |
//!
//! # Signals
//!
//! A [`Signal`] is a cheaply cloneable shared cell that lives outside the
//! ECS. Reading it inside a region, a patched component, [`effect`],
//! [`shard`] arguments, a [`memo`], or a `webby` component subscribes that
//! code to later `set` calls. Reading or writing a signal from inside its
//! own `update`/`with` closure, or a memo from inside its own computation,
//! panics instead of deadlocking. Signals are `Send + Sync`, so they can also be
//! stored in components or resources and set from systems. Everything runs
//! in [`ReactiveSet::Publish`], before UI layout.
//!
//! [`ReactiveSet::Publish`]: crate::reactivity::ReactiveSet::Publish

use std::{
    any::Any,
    cell::RefCell,
    fmt,
    hash::Hash,
    ops::{Add, Sub},
    sync::{
        Arc, Mutex, PoisonError, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use bevy::{
    ecs::{component::Mutable, world::CommandQueue},
    platform::collections::HashMap,
    prelude::*,
    scene::{EmptySceneEffect, Scene, SceneList, WorldSceneExt},
};

pub use crate::macros::{coat, coat_list};

pub use crate::runtime::{Cleanup, IntoCleanup, Reads};
use crate::runtime::{Ran, RunFn, effect_template, signal_deps, spawn_child, tracked};

// ---------------------------------------------------------------------------
// Dependency tracking
// ---------------------------------------------------------------------------

trait Versioned: Send + Sync {
    fn version(&self) -> u64;
}

/// A signal or memo read by a tracking scope, with the version it had.
#[derive(Clone)]
pub(crate) struct SignalDep {
    source: Arc<dyn Versioned>,
    version: u64,
}

impl SignalDep {
    pub(crate) fn is_dirty(&self) -> bool {
        self.source.version() != self.version
    }
}

thread_local! {
    static SCOPES: RefCell<Vec<Vec<SignalDep>>> = const { RefCell::new(Vec::new()) };
}

struct ScopeGuard;

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        SCOPES.with(|scopes| scopes.borrow_mut().pop());
    }
}

/// Runs `f`, collecting every signal and memo it reads.
pub(crate) fn scope<R>(f: impl FnOnce() -> R) -> (R, Vec<SignalDep>) {
    SCOPES.with(|scopes| scopes.borrow_mut().push(Vec::new()));
    let guard = ScopeGuard;
    let value = f();
    let deps = SCOPES.with(|scopes| scopes.borrow_mut().last_mut().map(std::mem::take));
    drop(guard);
    (value, deps.unwrap_or_default())
}

fn record(source: Arc<dyn Versioned>) {
    // Read the version before borrowing the scope stack: a memo may refresh,
    // which opens a nested scope.
    let version = source.version();
    SCOPES.with(|scopes| {
        if let Some(scope) = scopes.borrow_mut().last_mut() {
            scope.push(SignalDep { source, version });
        }
    });
}

fn untracked<R>(f: impl FnOnce() -> R) -> R {
    scope(f).0
}

// ---------------------------------------------------------------------------
// Re-entrancy checks
// ---------------------------------------------------------------------------

thread_local! {
    /// Signals and memos this thread is accessing, and whether exclusively.
    static HELD: RefCell<Vec<(usize, bool)>> = const { RefCell::new(Vec::new()) };
}

/// Marks a signal or memo as accessed by this thread until dropped.
struct Held(usize);

impl Drop for Held {
    fn drop(&mut self) {
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(index) = held.iter().rposition(|&(cell, _)| cell == self.0) {
                held.remove(index);
            }
        });
    }
}

/// Records an access to the cell at `cell`, panicking when it conflicts
/// with an access this thread already holds. Locking again would deadlock.
fn hold(cell: usize, exclusive: bool, conflict: &str) -> Held {
    let conflicts = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let conflicts = held
            .iter()
            .any(|&(other, other_exclusive)| other == cell && (exclusive || other_exclusive));
        if !conflicts {
            held.push((cell, exclusive));
        }
        conflicts
    });
    assert!(!conflicts, "{conflict}");
    Held(cell)
}

const SIGNAL_REENTERED: &str = "a `Signal` was set or read while this thread is updating or     reading it, for example `s.set(..)` inside `s.with(..)` or `s.get()` inside `s.update(..)`;     read the value first, or use the closure's argument";

const MEMO_REENTERED: &str = "a `Memo` was read while it is being computed or read on this     thread; a memo cannot depend on itself";

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

struct SignalInner<T> {
    value: RwLock<T>,
    version: AtomicU64,
}

impl<T: Send + Sync> Versioned for SignalInner<T> {
    fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

/// Reactive state shared by the closures of a scene.
///
/// Cloning is cheap and every clone refers to the same value. Reading with
/// [`get`](Self::get) or [`with`](Self::with) inside a `coat` or `webby`
/// closure subscribes that closure to later [`set`](Self::set) calls.
pub struct Signal<T>(Arc<SignalInner<T>>);

/// Creates a signal holding the value `init` returns.
///
/// Mirrors Topcoat's `signal(cx, init)`. Scenes own their signals through the
/// closures that capture them; there is no context argument.
pub fn signal<T: Send + Sync + 'static>(init: impl FnOnce() -> T) -> Signal<T> {
    Signal::new(init())
}

impl<T: Send + Sync + 'static> Signal<T> {
    /// Creates a signal holding `value`.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self(Arc::new(SignalInner {
            value: RwLock::new(value),
            version: AtomicU64::new(0),
        }))
    }

    /// Reads the value through `f` and tracks the read.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        record(self.0.clone());
        self.with_untracked(f)
    }

    /// Reads the value through `f` without tracking.
    ///
    /// # Panics
    ///
    /// If `f` sets or updates this signal, and if called inside this
    /// signal's own [`update`](Self::update). Nested reads are allowed.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let _held = hold(self.cell(), false, SIGNAL_REENTERED);
        f(&self.0.value.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn cell(&self) -> usize {
        Arc::as_ptr(&self.0).addr()
    }

    /// Clones the value and tracks the read.
    #[must_use]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// Clones the value without tracking.
    #[must_use]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        self.with_untracked(T::clone)
    }

    /// Replaces the value and notifies readers.
    pub fn set(&self, value: T) {
        self.update(|current| *current = value);
    }

    /// Replaces the value, notifying readers only if it changed.
    ///
    /// Returns whether the value changed.
    ///
    /// # Panics
    ///
    /// Like [`update`](Self::update).
    pub fn set_if_neq(&self, value: T) -> bool
    where
        T: PartialEq,
    {
        let held = hold(self.cell(), true, SIGNAL_REENTERED);
        let mut current = self.0.value.write().unwrap_or_else(PoisonError::into_inner);
        if *current == value {
            return false;
        }
        *current = value;
        drop(current);
        drop(held);
        self.notify();
        true
    }

    /// Mutates the value in place and notifies readers.
    ///
    /// # Panics
    ///
    /// When called while this thread is reading or updating the signal,
    /// such as `s.set(..)` inside `s.with(..)`, or when `f` reads it. Locking
    /// again would deadlock.
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let held = hold(self.cell(), true, SIGNAL_REENTERED);
        let result = f(&mut self.0.value.write().unwrap_or_else(PoisonError::into_inner));
        drop(held);
        self.notify();
        result
    }

    /// Marks the signal changed without modifying it.
    pub fn notify(&self) {
        self.0.version.fetch_add(1, Ordering::AcqRel);
    }

    /// Returns `true` if both handles refer to the same signal.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Values that [`Signal::increment`] and [`Signal::decrement`] can step.
pub trait Step: Add<Output = Self> + Sub<Output = Self> + Sized {
    /// The step size.
    const ONE: Self;
}

macro_rules! step {
    ($($ty:ty => $one:expr),* $(,)?) => {
        $(impl Step for $ty { const ONE: Self = $one; })*
    };
}

step!(
    u8 => 1, u16 => 1, u32 => 1, u64 => 1, u128 => 1, usize => 1,
    i8 => 1, i16 => 1, i32 => 1, i64 => 1, i128 => 1, isize => 1,
    f32 => 1.0, f64 => 1.0,
);

impl<T: Step + Copy + Send + Sync + 'static> Signal<T> {
    /// Adds one and notifies readers.
    pub fn increment(&self) {
        self.update(|value| *value = *value + T::ONE);
    }

    /// Subtracts one and notifies readers.
    pub fn decrement(&self) {
        self.update(|value| *value = *value - T::ONE);
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Default + Send + Sync + 'static> Default for Signal<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _held = hold(Arc::as_ptr(&self.0).addr(), false, SIGNAL_REENTERED);
        let value = self.0.value.read().unwrap_or_else(PoisonError::into_inner);
        f.debug_tuple("Signal").field(&*value).finish()
    }
}

// ---------------------------------------------------------------------------
// Memo
// ---------------------------------------------------------------------------

struct MemoState<T> {
    value: Option<T>,
    deps: Vec<SignalDep>,
    version: u64,
}

struct MemoInner<T> {
    compute: Box<dyn Fn() -> T + Send + Sync>,
    state: Mutex<MemoState<T>>,
}

impl<T: PartialEq + Send + Sync> MemoInner<T> {
    fn cell(&self) -> usize {
        std::ptr::from_ref(self).addr()
    }

    fn refresh(&self) {
        let needs_refresh = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.value.is_none() || state.deps.iter().any(SignalDep::is_dirty)
        };
        if !needs_refresh {
            return;
        }
        let held = hold(self.cell(), true, MEMO_REENTERED);
        let (value, deps) = scope(|| (self.compute)());
        drop(held);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.value.as_ref() != Some(&value) {
            state.value = Some(value);
            state.version += 1;
        }
        state.deps = deps;
    }
}

impl<T: PartialEq + Send + Sync> Versioned for MemoInner<T> {
    fn version(&self) -> u64 {
        self.refresh();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .version
    }
}

/// A cached value derived from signals, recomputed lazily when a signal it
/// read changes. Readers are notified only when the result differs.
pub struct Memo<T>(Arc<MemoInner<T>>);

/// Creates a [`Memo`]. Like Topcoat's `#[memoize]`, scoped to signals.
pub fn memo<T, F>(compute: F) -> Memo<T>
where
    T: PartialEq + Send + Sync + 'static,
    F: Fn() -> T + Send + Sync + 'static,
{
    Memo(Arc::new(MemoInner {
        compute: Box::new(compute),
        state: Mutex::new(MemoState {
            value: None,
            deps: Vec::new(),
            version: 0,
        }),
    }))
}

impl<T: PartialEq + Send + Sync + 'static> Memo<T> {
    /// Reads the value through `f` and tracks the read.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        record(self.0.clone());
        self.with_untracked(f)
    }

    /// Reads the value through `f` without tracking.
    ///
    /// # Panics
    ///
    /// When the memo reads itself, directly or through `f`.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.refresh();
        let _held = hold(self.0.cell(), true, MEMO_REENTERED);
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        f(state.value.as_ref().expect("memo was refreshed"))
    }

    /// Clones the value and tracks the read.
    #[must_use]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// Clones the value without tracking.
    #[must_use]
    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        self.with_untracked(T::clone)
    }
}

impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

/// Clones the listed signals into a closure, so several closures in one
/// scene can capture the same signal.
///
/// ```ignore
/// on(cloned!(count => move |_: On<Pointer<Click>>| count.increment()))
/// text(cloned!(count => move || count.get().to_string()))
/// ```
#[macro_export]
macro_rules! cloned {
    ($($name:ident),+ $(,)? => $body:expr) => {{
        $(let $name = $name.clone();)+
        $body
    }};
}

pub use crate::cloned;

// ---------------------------------------------------------------------------
// Scene helpers
// ---------------------------------------------------------------------------

/// Runs `f` now and again whenever something it read changes: a signal or
/// memo, or ECS data read through the [`Reads`] argument. Side effects go
/// through `Commands`, applied right after the run.
///
/// `f` may return a cleanup closure, which runs before the next run and when
/// the entity is despawned:
///
/// ```ignore
/// effect(move |reads: &Reads, commands: &mut Commands| {
///     let hp = reads.get::<Health>(player).map_or(0, |hp| hp.0);
///     let tooltip = commands.spawn(Text(format!("HP {hp}"))).id();
///     move |commands: &mut Commands| commands.entity(tooltip).despawn()
/// })
/// ```
pub fn effect<F, C>(f: F) -> impl Template<Output = EmptySceneEffect>
where
    F: Fn(&Reads, &mut Commands) -> C + Clone + Send + Sync + 'static,
    C: IntoCleanup,
{
    effect_template((), move |()| -> RunFn {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let mut queue = CommandQueue::default();
            let (cleanup, deps) = tracked(world, target, |reads| {
                let mut commands = Commands::new(&mut queue, world);
                f(reads, &mut commands).into_cleanup()
            });
            queue.apply(world);
            Ran { deps, cleanup }
        })
    })
}

/// Re-renders a child scene whenever `args()` changes. Like a Topcoat
/// `#[shard]` called with `$(...)` arguments.
///
/// `render` receives the arguments and `&World`, so it can query any state
/// the way a shard queries the database. Reads of the world are not
/// tracked: only argument changes re-render the shard. The current entity
/// should have no other children.
pub fn shard<A, AF, S, R>(args: AF, render: R) -> impl Template<Output = EmptySceneEffect>
where
    A: PartialEq + Send + Sync + 'static,
    AF: Fn() -> A + Clone + Send + Sync + 'static,
    S: Scene,
    R: Fn(&A, &World) -> S + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let (args, render) = (args.clone(), render.clone());
        let mut current: Option<(A, Option<Entity>)> = None;
        Box::new(move |world: &mut World, target| {
            let (next, deps) = tracked(world, target, |_| args());
            if current.as_ref().is_none_or(|(args, _)| *args != next) {
                if let Some((_, Some(child))) = current.take() {
                    world.despawn(child);
                }
                let scene = untracked(|| render(&next, world));
                let child = spawn_child(world, target, scene);
                current = Some((next, child));
            }
            deps.into()
        })
    })
}

/// Host entity of a reactive `coat!` region.
///
/// With the `ui` feature it is a `GhostNode`, so its children are laid out
/// as if they were children of the region's parent.
#[derive(Component, Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "ui", require(bevy::ui::experimental::GhostNode))]
#[cfg_attr(feature = "ui", component(on_add = crate::runtime::strip_ghost_transform_hook))]
pub struct Region;

type BoxedList = Box<dyn SceneList>;

fn spawn_list(world: &mut World, parent: Entity, list: BoxedList) -> Vec<Entity> {
    match world.spawn_scene_list(list) {
        Ok(entities) => {
            for &entity in &entities {
                world.entity_mut(entity).insert(ChildOf(parent));
            }
            entities
        }
        Err(error) => {
            let handler = world.fallback_error_handler();
            handler(
                BevyError::from(error),
                bevy::ecs::error::ErrorContext::Command {
                    name: "coat::spawn_list".into(),
                },
            );
            Vec::new()
        }
    }
}

fn despawn_all(world: &mut World, entities: &[Entity]) {
    for &entity in entities {
        world.despawn(entity);
    }
}

/// Patches component `C` whenever a signal read by `f` changes.
///
/// `f` returns whether it changed a field; only then is `C` marked changed,
/// so tracked components are not republished for identical values.
#[doc(hidden)]
pub fn patch<C, F>(f: F) -> impl Template<Output = EmptySceneEffect>
where
    C: Component<Mutability = Mutable>,
    F: Fn(&mut C) -> bool + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let ((), signals) = scope(|| {
                if let Some(mut component) = world.get_mut::<C>(target)
                    && f(component.bypass_change_detection())
                {
                    component.set_changed();
                }
            });
            signal_deps(signals).into()
        })
    })
}

/// Writes `value` into `field` if it differs. Returns whether it did.
#[doc(hidden)]
pub fn assign<T: PartialEq>(field: &mut T, value: T) -> bool {
    if *field == value {
        false
    } else {
        *field = value;
        true
    }
}

/// Rebuilds the region's children whenever `key()` changes.
#[doc(hidden)]
pub fn region<K, KF, R>(key: KF, render: R) -> impl Template<Output = EmptySceneEffect>
where
    K: PartialEq + Send + Sync + 'static,
    KF: Fn() -> K + Clone + Send + Sync + 'static,
    R: Fn(&K) -> Option<BoxedList> + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let (key, render) = (key.clone(), render.clone());
        let mut current: Option<(K, Vec<Entity>)> = None;
        Box::new(move |world: &mut World, target| {
            let (next, deps) = tracked(world, target, |_| key());
            if current.as_ref().is_none_or(|(key, _)| *key != next) {
                if let Some((_, children)) = current.take() {
                    despawn_all(world, &children);
                }
                let children = untracked(|| render(&next))
                    .map(|list| spawn_list(world, target, list))
                    .unwrap_or_default();
                current = Some((next, children));
            }
            deps.into()
        })
    })
}

/// Keeps one set of children per item, reusing unchanged rows.
#[doc(hidden)]
pub fn for_keyed<V, K, IF, KF, R>(
    items: IF,
    key: KF,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
    IF: Fn() -> Vec<V> + Clone + Send + Sync + 'static,
    KF: Fn(&V) -> K + Clone + Send + Sync + 'static,
    R: Fn(&V) -> BoxedList + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let (items, key, render) = (items.clone(), key.clone(), render.clone());
        let mut rows: HashMap<K, (V, Vec<Entity>)> = HashMap::default();
        let mut order: Vec<Entity> = Vec::new();
        Box::new(move |world: &mut World, target| {
            let (next, deps) = tracked(world, target, |_| items());
            let mut next_rows = HashMap::default();
            let mut next_order = Vec::new();
            for item in next {
                let item_key = untracked(|| key(&item));
                let children = match rows.remove(&item_key) {
                    Some((old, children)) if old == item => children,
                    previous => {
                        if let Some((_, children)) = previous {
                            despawn_all(world, &children);
                        }
                        let list = untracked(|| render(&item));
                        spawn_list(world, target, list)
                    }
                };
                next_order.extend(children.iter().copied());
                if let Some((_, duplicate)) = next_rows.insert(item_key, (item, children)) {
                    warn_once!(
                        "coat!: two items in a keyed `for` have the same `#[key]`; only the last                          one is shown"
                    );
                    next_order.retain(|entity| !duplicate.contains(entity));
                    despawn_all(world, &duplicate);
                }
            }
            for (_, (_, children)) in rows.drain() {
                despawn_all(world, &children);
            }
            rows = next_rows;
            if next_order != order
                && let Ok(mut entity) = world.get_entity_mut(target)
            {
                entity.replace_children(&next_order);
            }
            order = next_order;
            deps.into()
        })
    })
}

#[doc(hidden)]
pub fn for_indexed<V, IF, R>(items: IF, render: R) -> impl Template<Output = EmptySceneEffect>
where
    V: PartialEq + Send + Sync + 'static,
    IF: Fn() -> Vec<V> + Clone + Send + Sync + 'static,
    R: Fn(&V) -> BoxedList + Clone + Send + Sync + 'static,
{
    for_keyed(
        move || items().into_iter().enumerate().collect::<Vec<_>>(),
        |(index, _): &(usize, V)| *index,
        move |(_, item): &(usize, V)| render(item),
    )
}

/// Region key of a reactive `match`: the matched arm, and the values its
/// pattern bound that the arm uses.
#[doc(hidden)]
pub struct ArmKey {
    arm: usize,
    values: Box<dyn DynEq>,
}

trait DynEq: Any + Send + Sync {
    fn eq_dyn(&self, other: &dyn Any) -> bool;
    fn as_any(&self) -> &dyn Any;
}

impl<T: PartialEq + Send + Sync + 'static> DynEq for T {
    fn eq_dyn(&self, other: &dyn Any) -> bool {
        other.downcast_ref::<T>() == Some(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl PartialEq for ArmKey {
    fn eq(&self, other: &Self) -> bool {
        self.arm == other.arm && self.values.eq_dyn(other.values.as_any())
    }
}

#[doc(hidden)]
pub fn arm_key<T: PartialEq + Send + Sync + 'static>(arm: usize, values: T) -> ArmKey {
    ArmKey {
        arm,
        values: Box::new(values),
    }
}

#[doc(hidden)]
pub mod __private {
    pub use super::{
        ArmKey as __CoatArmKey, Region as __CoatRegion, arm_key as __coat_arm_key,
        assign as __coat_assign, for_indexed as __coat_for_indexed, for_keyed as __coat_for,
        patch as __coat_patch, region as __coat_region,
    };
    pub use bevy::scene::{bsn, bsn_list};
}

// `bsn!` checks that its call site's source text starts with `bsn!`. These
// wrappers give it that call site when invoked from `coat!` output, whose
// own call site is `coat!`. `bsn`/`bsn_list` resolve through `__private`,
// which `coat!` output glob-imports.

#[doc(hidden)]
#[macro_export]
macro_rules! __coat_bsn {
    ($($tokens:tt)*) => { bsn! { $($tokens)* } };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __coat_bsn_list {
    ($($tokens:tt)*) => { bsn_list! { $($tokens)* } };
}
