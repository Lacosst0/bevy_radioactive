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
//! | `#[key(expr)] for $name in iter { .. }`    | rows kept per key, `$name` a signal      |
//! | `for $name in iter { .. }`                 | rows per position, `$name` a signal      |
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
//! Writing the loop variable with `$`, as in `#[key(todo.id)] for $todo in
//! $todos`, gives each row its item as a [`Signal`] instead of a value, like
//! Solid's `<For>` (keyed) and `<Index>` (without a key). A row then keeps
//! its entities for as long as its key (or position) exists, and a changed
//! item is written to the row's signal with `set_if_neq`, so only the
//! row's `$todo` reads update: `Text($(format!("{}", $&todo.title)))` is
//! patched in place instead of the row being respawned. Inside `#[key]`
//! the loop variable is the plain item.
//!
//! [`Value<T>`] is a prop that is either a constant, a signal or a memo;
//! `$value` reads it like a signal.
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
//! A [`CopySignal`] is the same signal behind a `Copy` handle whose value is
//! owned by an entity, so closures capture it without `clone`s and it is
//! freed when its owner despawns.
//!
//! [`ReactiveSet::Publish`]: crate::reactivity::ReactiveSet::Publish

use std::{
    any::Any,
    cell::RefCell,
    fmt,
    hash::Hash,
    ops::{Add, Sub},
    sync::{
        Arc, Mutex, PoisonError, RwLock, Weak,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use bevy::{
    ecs::{component::Mutable, world::CommandQueue},
    platform::collections::HashMap,
    prelude::*,
    scene::{EmptySceneEffect, Scene, SceneList, WorldSceneExt},
};

mod copy_signal;

pub use copy_signal::{
    CopySignal, OwnedSignals, Owns, SignalCell, SignalCommandsExt, SignalOf, copy_signal, owns,
};

pub use crate::macros::{coat, coat_list};

pub use crate::runtime::{Cleanup, IntoCleanup, Reads};
use crate::runtime::{
    Phase, Ran, RunFn, effect_template, effect_template_in, signal_deps, spawn_child, tracked,
};

// ---------------------------------------------------------------------------
// Dependency tracking
// ---------------------------------------------------------------------------

/// Told when a signal or memo it read may have changed: a memo, or one run
/// of a runtime effect.
pub(crate) trait Subscriber: Send + Sync {
    /// `changed` is `true` when a signal it read changed, and `false` when a
    /// memo it read may have changed.
    fn mark(&self, changed: bool);
}

/// Subscribers of a signal or memo, held weakly: a dropped memo, or an
/// effect run superseded by a newer run, is pruned as the list grows.
#[derive(Default)]
struct SubscriberList(Mutex<Vec<Weak<dyn Subscriber>>>);

impl SubscriberList {
    fn add(&self, subscriber: Weak<dyn Subscriber>, unique: bool) {
        let mut list = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if unique && list.iter().any(|other| Weak::ptr_eq(other, &subscriber)) {
            return;
        }
        if list.len() == list.capacity() {
            list.retain(|other| other.strong_count() > 0);
        }
        list.push(subscriber);
    }

    /// Marks every live subscriber. Marks flow only from sources to their
    /// subscribers, so holding the lock while marking cannot deadlock.
    fn notify(&self, changed: bool) {
        let list = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for subscriber in list.iter().filter_map(Weak::upgrade) {
            subscriber.mark(changed);
        }
    }
}

trait Versioned: Send + Sync {
    fn version(&self) -> u64;
    fn subscribers(&self) -> &SubscriberList;
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

    pub(crate) fn same_source(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.source), Arc::as_ptr(&other.source))
    }

    /// Marks `subscriber` whenever the source changes from now on.
    pub(crate) fn subscribe(&self, subscriber: Weak<dyn Subscriber>) {
        self.source.subscribers().add(subscriber, false);
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
    subscribers: SubscriberList,
}

impl<T: Send + Sync> Versioned for SignalInner<T> {
    fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    fn subscribers(&self) -> &SubscriberList {
        &self.subscribers
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
            subscribers: SubscriberList::default(),
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
    ///
    /// Readers are pushed onto the runtime's dirty queue right away, so the
    /// next reactive pass runs only them.
    pub fn notify(&self) {
        self.0.version.fetch_add(1, Ordering::AcqRel);
        self.0.subscribers.notify(true);
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

/// Handles are equal when they refer to the same signal (see
/// [`ptr_eq`](Signal::ptr_eq)), whatever its value. This lets a `webby`
/// component that takes a signal prop skip re-rendering when its parent
/// re-renders with the same signal.
impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for Signal<T> {}

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

// Reactively-style colours of a memo. A changed signal marks the memos that
// read it `DIRTY` and, through them, every memo further down `CHECK`; reading
// a memo then recomputes it only if it is `DIRTY`, or `CHECK` with a source
// whose version moved. Each memo computes at most once per change, in source
// order, so a diamond never sees a half-updated graph.
const CLEAN: u8 = 0;
const CHECK: u8 = 1;
const DIRTY: u8 = 2;

struct MemoInner<T> {
    compute: Box<dyn Fn() -> T + Send + Sync>,
    state: Mutex<MemoState<T>>,
    color: AtomicU8,
    subscribers: SubscriberList,
    this: Weak<Self>,
}

impl<T: PartialEq + Send + Sync + 'static> MemoInner<T> {
    fn cell(&self) -> usize {
        std::ptr::from_ref(self).addr()
    }

    fn cached(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .value
            .is_some()
    }

    fn refresh(&self) {
        loop {
            match self.color.load(Ordering::Acquire) {
                CLEAN if self.cached() => return,
                CHECK if self.cached() => {
                    // Memo sources refresh here, in the order they were read.
                    let changed = self
                        .state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .deps
                        .iter()
                        .any(SignalDep::is_dirty);
                    if changed {
                        break;
                    }
                    let cleared = self.color.compare_exchange(
                        CHECK,
                        CLEAN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    if cleared.is_ok() {
                        return;
                    }
                    // Marked again while checking: look again.
                }
                _ => break,
            }
        }
        self.recompute();
    }

    fn recompute(&self) {
        // Cleared before computing, so a source changing meanwhile marks the
        // memo again instead of being lost.
        self.color.store(CLEAN, Ordering::Release);
        let held = hold(self.cell(), true, MEMO_REENTERED);
        let (value, deps) = scope(|| (self.compute)());
        drop(held);
        let this: Weak<dyn Subscriber> = self.this.clone();
        for dep in &deps {
            dep.source.subscribers().add(this.clone(), true);
        }
        // A source that changed between being read and the subscription
        // above did not mark this memo.
        let outdated = deps.iter().any(SignalDep::is_dirty);
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.value.as_ref() != Some(&value) {
                state.value = Some(value);
                state.version += 1;
            }
            state.deps = deps;
        }
        if outdated {
            self.mark(true);
        }
    }
}

impl<T: PartialEq + Send + Sync + 'static> Subscriber for MemoInner<T> {
    fn mark(&self, changed: bool) {
        let color = if changed { DIRTY } else { CHECK };
        if self.color.fetch_max(color, Ordering::AcqRel) == CLEAN {
            self.subscribers.notify(false);
        }
    }
}

impl<T: PartialEq + Send + Sync + 'static> Versioned for MemoInner<T> {
    fn version(&self) -> u64 {
        self.refresh();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .version
    }

    fn subscribers(&self) -> &SubscriberList {
        &self.subscribers
    }
}

/// A cached value derived from signals, recomputed lazily when a signal it
/// read changes. Readers are notified only when the result differs.
///
/// A change marks the memos downstream of it as possibly stale (Reactively's
/// colouring) without computing anything; the next read recomputes each
/// stale memo at most once, so derived values never see a half-updated
/// graph and a diamond of memos computes its bottom once per change.
pub struct Memo<T>(Arc<MemoInner<T>>);

/// Creates a [`Memo`]. Like Topcoat's `#[memoize]`, scoped to signals.
pub fn memo<T, F>(compute: F) -> Memo<T>
where
    T: PartialEq + Send + Sync + 'static,
    F: Fn() -> T + Send + Sync + 'static,
{
    Memo(Arc::new_cyclic(|this| MemoInner {
        compute: Box::new(compute),
        state: Mutex::new(MemoState {
            value: None,
            deps: Vec::new(),
            version: 0,
        }),
        color: AtomicU8::new(DIRTY),
        subscribers: SubscriberList::default(),
        this: this.clone(),
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

/// Handles are equal when they refer to the same memo, like [`Signal`].
impl<T> PartialEq for Memo<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for Memo<T> {}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// A prop that is either a constant or reactive, after `bevy_reactor`'s
/// `IntoSignal`.
///
/// Components and scene functions that take a `Value<T>` accept a plain
/// `T`, a [`Signal<T>`] or a [`Memo<T>`] through `.into()`. Reading it with
/// [`get`](Self::get) or [`with`](Self::with) tracks the read when it is
/// reactive, so `$value` in `coat!` and `value.get()` in a `webby`
/// component update when a signal behind it changes, and cost nothing extra
/// when it is a constant.
///
/// ```
/// use bevy_radioactive::coat::{Value, signal};
///
/// fn label(text: impl Into<Value<String>>) -> Value<String> {
///     text.into()
/// }
///
/// let fixed = label("Volume".to_string());
/// let name = signal(|| "Player".to_string());
/// let live = label(name.clone());
/// name.set("Ada".into());
/// assert_eq!(fixed.get(), "Volume");
/// assert_eq!(live.get(), "Ada");
/// assert!(live.is_reactive() && !fixed.is_reactive());
/// ```
pub enum Value<T> {
    /// A value that never changes.
    Const(T),
    /// The current value of a signal.
    Signal(Signal<T>),
    /// The current value of a memo.
    Memo(Memo<T>),
}

impl<T: PartialEq + Send + Sync + 'static> Value<T> {
    /// Reads the value through `f`, tracking the read when it is reactive.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self {
            Self::Const(value) => f(value),
            Self::Signal(signal) => signal.with(f),
            Self::Memo(memo) => memo.with(f),
        }
    }

    /// Reads the value through `f` without tracking.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self {
            Self::Const(value) => f(value),
            Self::Signal(signal) => signal.with_untracked(f),
            Self::Memo(memo) => memo.with_untracked(f),
        }
    }

    /// Clones the value, tracking the read when it is reactive.
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

    /// Whether the value can change: a signal or a memo.
    #[must_use]
    pub fn is_reactive(&self) -> bool {
        !matches!(self, Self::Const(_))
    }
}

impl<T: Clone> Clone for Value<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Const(value) => Self::Const(value.clone()),
            Self::Signal(signal) => Self::Signal(signal.clone()),
            Self::Memo(memo) => Self::Memo(memo.clone()),
        }
    }
}

/// Constants compare by value; signals and memos by identity.
impl<T: PartialEq> PartialEq for Value<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Const(a), Self::Const(b)) => a == b,
            (Self::Signal(a), Self::Signal(b)) => a == b,
            (Self::Memo(a), Self::Memo(b)) => a == b,
            _ => false,
        }
    }
}

impl<T: Default> Default for Value<T> {
    fn default() -> Self {
        Self::Const(T::default())
    }
}

impl<T> From<T> for Value<T> {
    fn from(value: T) -> Self {
        Self::Const(value)
    }
}

impl<T> From<Signal<T>> for Value<T> {
    fn from(signal: Signal<T>) -> Self {
        Self::Signal(signal)
    }
}

impl<T> From<Memo<T>> for Value<T> {
    fn from(memo: Memo<T>) -> Self {
        Self::Memo(memo)
    }
}

impl From<&str> for Value<String> {
    fn from(value: &str) -> Self {
        Self::Const(value.to_owned())
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
    effect_in(Phase::Main, f)
}

/// Like [`effect`], but runs after UI layout and transform propagation, in
/// [`ReactiveSet::PostLayout`], like Vue's `flush: 'post'` watchers or
/// React's `useLayoutEffect`.
///
/// Use it to read what layout computed this frame, such as `ComputedNode`
/// or `GlobalTransform`. It runs when spawned (in the first post-layout
/// phase after its entity spawns) and again whenever something it read
/// changed since its last run. Writes it makes are seen by ordinary effects
/// and laid out on the next frame.
///
/// ```ignore
/// layout_effect(|r: &Reads, commands: &mut Commands| {
///     if let Some(node) = r.this::<ComputedNode>() {
///         commands.trigger(Measured { entity: r.entity(), size: node.size() });
///     }
/// })
/// ```
///
/// [`ReactiveSet::PostLayout`]: crate::reactivity::ReactiveSet::PostLayout
pub fn layout_effect<F, C>(f: F) -> impl Template<Output = EmptySceneEffect>
where
    F: Fn(&Reads, &mut Commands) -> C + Clone + Send + Sync + 'static,
    C: IntoCleanup,
{
    effect_in(Phase::PostLayout, f)
}

fn effect_in<F, C>(phase: Phase, f: F) -> impl Template<Output = EmptySceneEffect>
where
    F: Fn(&Reads, &mut Commands) -> C + Clone + Send + Sync + 'static,
    C: IntoCleanup,
{
    effect_template_in(phase, (), move |()| -> RunFn {
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

/// Spawns `list`, as children of `parent` if given. Keyed lists pass `None`
/// and parent their rows once, in order, with `replace_children`.
fn spawn_list(world: &mut World, parent: Option<Entity>, list: BoxedList) -> Vec<Entity> {
    match world.spawn_scene_list(list) {
        Ok(entities) => {
            if let Some(parent) = parent {
                for &entity in &entities {
                    world.entity_mut(entity).insert(ChildOf(parent));
                }
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
                    .map(|list| spawn_list(world, Some(target), list))
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
    keyed_rows(
        items,
        key,
        |item| item,
        // A changed value respawns the row.
        |old: &V, item| if *old == item { None } else { Some(item) },
        render,
    )
}

/// Like [`for_keyed`], but each row keeps its entities for as long as its
/// key is present and receives its item as a signal, updated in place
/// (`set_if_neq`) when the item changes. Generated by
/// `#[key(..)] for $item in ..` in `coat!`.
#[doc(hidden)]
pub fn for_keyed_signal<V, K, IF, KF, R>(
    items: IF,
    key: KF,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
    IF: Fn() -> Vec<V> + Clone + Send + Sync + 'static,
    KF: Fn(&V) -> K + Clone + Send + Sync + 'static,
    R: Fn(&Signal<V>) -> BoxedList + Clone + Send + Sync + 'static,
{
    keyed_rows(
        items,
        key,
        Signal::new,
        |signal: &Signal<V>, item| {
            signal.set_if_neq(item);
            None
        },
        render,
    )
}

/// One set of children per key. `update` receives a kept row's state and
/// its new item, and returns the item back to respawn the row.
fn keyed_rows<V, K, S, IF, KF, R>(
    items: IF,
    key: KF,
    make: fn(V) -> S,
    update: fn(&S, V) -> Option<V>,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    V: Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
    S: Send + Sync + 'static,
    IF: Fn() -> Vec<V> + Clone + Send + Sync + 'static,
    KF: Fn(&V) -> K + Clone + Send + Sync + 'static,
    R: Fn(&S) -> BoxedList + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let (items, key, render) = (items.clone(), key.clone(), render.clone());
        let mut rows: HashMap<K, (S, Vec<Entity>)> = HashMap::default();
        let mut order: Vec<Entity> = Vec::new();
        Box::new(move |world: &mut World, target| {
            let (next, deps) = tracked(world, target, |_| items());
            let mut next_rows = HashMap::default();
            let mut next_order = Vec::new();
            for item in next {
                let item_key = untracked(|| key(&item));
                // Kept rows update in place or hand the item back to respawn.
                let respawn = match rows.remove(&item_key) {
                    Some((state, children)) => match update(&state, item) {
                        None => Err((state, children)),
                        Some(item) => {
                            despawn_all(world, &children);
                            Ok(item)
                        }
                    },
                    None => Ok(item),
                };
                let (state, children) = respawn.map_or_else(
                    |kept| kept,
                    |item| {
                        let state = make(item);
                        let list = untracked(|| render(&state));
                        let children = spawn_list(world, None, list);
                        (state, children)
                    },
                );
                next_order.extend(children.iter().copied());
                if let Some((_, duplicate)) = next_rows.insert(item_key, (state, children)) {
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

/// One set of children per position, each receiving the item at its
/// position as a signal updated in place, like Solid's `<Index>`. Rows are
/// added and removed only at the end. Generated by `for $item in ..`
/// without `#[key]` in `coat!`.
#[doc(hidden)]
pub fn for_indexed_signal<V, IF, R>(
    items: IF,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    V: PartialEq + Send + Sync + 'static,
    IF: Fn() -> Vec<V> + Clone + Send + Sync + 'static,
    R: Fn(&Signal<V>) -> BoxedList + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let (items, render) = (items.clone(), render.clone());
        let mut rows: Vec<(Signal<V>, Vec<Entity>)> = Vec::new();
        Box::new(move |world: &mut World, target| {
            let (next, deps) = tracked(world, target, |_| items());
            let len = next.len();
            for (index, item) in next.into_iter().enumerate() {
                if let Some((signal, _)) = rows.get(index) {
                    signal.set_if_neq(item);
                } else {
                    let signal = Signal::new(item);
                    let list = untracked(|| render(&signal));
                    let children = spawn_list(world, Some(target), list);
                    rows.push((signal, children));
                }
            }
            for (_, children) in rows.drain(len.min(rows.len())..) {
                despawn_all(world, &children);
            }
            deps.into()
        })
    })
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
        assign as __coat_assign, for_indexed as __coat_for_indexed,
        for_indexed_signal as __coat_for_indexed_signal, for_keyed as __coat_for,
        for_keyed_signal as __coat_for_signal, patch as __coat_patch, region as __coat_region,
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
