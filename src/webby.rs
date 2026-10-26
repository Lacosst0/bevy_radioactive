//! React-style function components and hooks for BSN scenes.
//!
//! Mark a function with [`#[component]`](macro@component) and use it in
//! `bsn!` like a scene function: `@counter(5)`. The `cx` argument is
//! supplied by the runtime; the remaining arguments are props. The component
//! re-renders whenever state it read changes, and its output is ordinary BSN
//! built from ordinary Rust values:
//!
//! ```ignore
//! use bevy_radioactive::webby::*;
//!
//! #[component]
//! fn counter(cx: &Cx, start: i32) -> impl Scene {
//!     let (count, set_count) = cx.use_state(|| start);
//!     let doubled = cx.use_memo(count, |count| count * 2);
//!     cx.use_effect(count, |count, _commands| info!("count is now {count}"));
//!
//!     bsn! {
//!         Button
//!         on(move |_: On<Pointer<Click>>| set_count(count + 1))
//!         Children [ Text({format!("{count} (x2 = {doubled})")}) ]
//!     }
//! }
//!
//! #[component]
//! fn health_bar(cx: &Cx, player: Entity) -> impl Scene {
//!     let hp = cx.use_component::<Health>(player).map_or(0, |hp| hp.0);
//!     bsn! { Text({format!("HP {hp}")}) }
//! }
//!
//! #[component]
//! fn app(cx: &Cx) -> impl Scene {
//!     let todos = cx.use_resource::<Todos>().unwrap_or_default().0;
//!     let rows = todos.into_iter().map(|todo| todo_row(todo).key(todo.id)).collect::<Vec<_>>();
//!     bsn! {
//!         Node
//!         Children [
//!             #Player Health(100)
//!             --
//!             @counter(5)                 // <Counter start={5} />
//!             --
//!             @health_bar(#Player)        // `Entity` props accept `#Name`
//!             --
//!             Node Children [ {rows} ]    // {todos.map(t => <TodoRow key={t.id} />)}
//!         ]
//!     }
//! }
//!
//! // mount: commands.spawn_scene(bsn! { @app() })
//! ```
//!
//! | React                                  | `webby`                                         |
//! | -------------------------------------- | ----------------------------------------------- |
//! | `function Counter({ start })`          | `#[component] fn counter(cx: &Cx, start: i32) -> impl Scene` |
//! | `<Counter start={5} />`                | `@counter(5)`                                   |
//! | `<Row key={id} />`                     | `row(..).key(id)` or `@keyed(id, row(..))`      |
//! | inline component                       | `component(\|cx\| bsn! { .. })`                  |
//! | JSX, `{expr}`                          | `bsn! { ... }`, `{expr}`                        |
//! | `useState`                             | [`Cx::use_state`]                               |
//! | `useReducer`                           | [`Cx::use_reducer`]                             |
//! | signals (Preact/Solid)                 | [`Cx::use_signal`]                              |
//! | `useMemo`                              | [`Cx::use_memo`]                                |
//! | `useEffect`                            | [`Cx::use_effect`]                              |
//! | `useRef`                               | [`Cx::use_ref`]                                 |
//! | `use` / Suspense-style data loading    | [`Cx::use_task`]                                |
//! | `createContext` / `useContext`         | [`Cx::provide_context`] / [`Cx::use_context`]   |
//! | store/selector hooks                   | [`Cx::use_resource`] / [`Cx::use_component`]    |
//! | `onClick={...}`                        | BSN `on(move \|_: On<Pointer<Click>>\| ...)`    |
//!
//! # Props
//!
//! - The first argument may be `cx: &Cx`; it is optional.
//! - `Entity` props accept `#Name` references from the surrounding scene, or
//!   an `Entity` value. Mark other entity-valued props `#[entity]`, such as
//!   `#[entity] leader: Option<Entity>` or `#[entity] members: Vec<Entity>`
//!   (any type that implements [`Deps`]).
//! - Other props must be `Clone + Send + Sync + 'static`; they are cloned
//!   into each render. When every prop is also `PartialEq`, a parent
//!   re-render with equal props does not re-render the component.
//! - `-> impl Scene` never borrows from `cx`; the macro adds `use<..>`.
//! - A component may return `Result<impl Scene>`. An error goes to Bevy's
//!   fallback error handler, named after the component, and the component
//!   renders nothing until its next render.
//! - To render scenes of different types from two branches, use
//!   [`either`].
//!
//! # Rendering model
//!
//! - A component renders once when spawned and again whenever something it
//!   read changes: its state, any [`Signal`] it called `get` on, or ECS data
//!   read through [`Cx::use_resource`], [`Cx::use_component`], or the
//!   tracked [`Reads`] methods available on `cx`.
//! - The rendered scene is a child of the component's entity. An entity
//!   without a UI `Node` becomes a `GhostNode` (with the `ui` feature), so
//!   `@counter(5)` can be a list item without affecting layout.
//! - A render reconciles child components with the previous render, like
//!   React keeps component instances. A child is matched by its key, or
//!   else by the source location of its `@name(..)` call and how many
//!   components that location created earlier in the render, so a
//!   conditional sibling does not shift anyone's state. Key components
//!   created in loops.
//! - A matched child keeps its host entity, its hook state and its rendered
//!   subtree: the host is moved into the parent's new scene in place of the
//!   entity spawned for it. It renders again only if its props changed
//!   (`React.memo`): a `#[component]` whose props are all `PartialEq`
//!   (including `Entity` props, and [`Signal`]s, which compare by identity)
//!   is skipped when they are equal; one with a prop that is not
//!   `PartialEq`, and closures mounted with [`component`](fn@component),
//!   render again with every parent render. Children that the new render no
//!   longer includes are despawned and unmount.
//! - Plain entities in the parent's subtree are recreated. Components,
//!   effects, observers and children declared beside a kept child (as in
//!   `@row(id) Node { .. }`) are moved onto its host from the new scene,
//!   replacing those of the previous render; a component type the new scene
//!   no longer declares there stays on the host. A child that skips its
//!   render does not see a changed non-signal context value; provide a
//!   [`Signal`] for context that changes.
//! - Hooks must be called in the same order on every render.
//! - Renders run in [`ReactiveSet::Publish`], before UI layout, so state set
//!   by an observer or system earlier in the frame is shown the same frame.
//!
//! [`ReactiveSet::Publish`]: crate::reactivity::ReactiveSet::Publish

use std::{
    any::{Any, TypeId, type_name},
    cell::{Cell, RefCell},
    hash::{BuildHasher, Hash},
    ops::Deref,
    panic::Location,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
};

use bevy::{
    ecs::{
        template::{Template, TemplateContext},
        world::CommandQueue,
    },
    platform::{collections::HashMap, hash::FixedState},
    prelude::*,
    scene::{
        EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene,
        SceneDependencies,
    },
};

use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool, futures::check_ready};

use bevy::ecs::observer::ObservedBy;

use crate::runtime::{
    PendingTasks, PollTask, PostEffects, RunFn, Unmounted, adopt_effects, attach_component,
    ensure_runtime, spawn_child, tracked,
};
pub use crate::{
    coat::{Signal, cloned},
    macros::component,
    runtime::{Cleanup, Deps, Effects, IntoCleanup, Reads},
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// Component instances
// ---------------------------------------------------------------------------

/// Identity of a child component within its parent's render.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ChildKey {
    /// Unkeyed: the source location that created the component, and how
    /// many components that location created earlier in the same render.
    Site {
        loc: &'static Location<'static>,
        nth: u32,
    },
    /// Keyed with [`Mount::key`].
    Key(u64),
}

#[derive(Default)]
struct ChildSlots {
    previous: HashMap<ChildKey, Arc<Instance>>,
    current: HashMap<ChildKey, Arc<Instance>>,
    sites: HashMap<&'static Location<'static>, u32>,
}

type PendingEffect = Box<dyn FnOnce(&mut Commands) + Send + Sync>;

/// A cleanup slot shared between a hook and its instance, run on unmount.
type CleanupCell = Arc<Mutex<Option<Cleanup>>>;

/// State of one mounted component: hooks, child instances and contexts.
///
/// A host's effect slot owns its instance, and an instance owns its child
/// instances; the parent link is weak, so unmounting a host frees its whole
/// instance tree.
struct Instance {
    parent: Option<Weak<Instance>>,
    hooks: Mutex<Vec<Box<dyn Any + Send + Sync>>>,
    children: Mutex<ChildSlots>,
    contexts: Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
    /// Effect cleanups and `on_unmount` callbacks, run when dropped.
    cleanups: Mutex<Vec<CleanupCell>>,
    unmounted: Unmounted,
    /// The entity hosting the component, once its render loop first ran.
    host: Mutex<Option<Entity>>,
    /// The root entity of the latest render's scene.
    rendered: Mutex<Option<Entity>>,
    /// Observers the parent's scene declared on the host, replaced when a
    /// parent re-render keeps the host.
    observers: Mutex<Vec<Entity>>,
    /// The latest [`Props`], type-erased.
    props: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    /// Notified when a parent render replaces `props`; read by every render.
    props_changed: Signal<()>,
    /// Child instances kept by the render in progress, with the entity the
    /// new scene spawned for each; their hosts take those entities' places.
    adopt: Mutex<Vec<(Entity, Arc<Instance>)>>,
}

/// What a mounted component renders from: its render function, its
/// resolved entity props, and the props compared to skip a re-render.
struct Props<F, O> {
    render: F,
    deps: O,
    memo: Option<PropsMemo>,
}

type DynEq = Arc<dyn Fn(&dyn Any, &dyn Any) -> bool + Send + Sync>;

/// Captured props of a `#[component]` call, compared by `PartialEq` when a
/// parent re-render keeps the component.
#[derive(Clone)]
struct PropsMemo {
    props: Arc<dyn Any + Send + Sync>,
    props_eq: DynEq,
    deps_eq: DynEq,
}

impl PropsMemo {
    /// Whether `self` and `deps` render the same as `old` and `old_deps`.
    fn same(&self, old: &Self, deps: &dyn Any, old_deps: &dyn Any) -> bool {
        (self.props_eq)(&*old.props, &*self.props) && (self.deps_eq)(old_deps, deps)
    }
}

/// Erases `eq`; `None` (props that are not `PartialEq`) counts as changed.
fn erase_eq<T: 'static>(eq: fn(&T, &T) -> Option<bool>) -> DynEq {
    Arc::new(
        move |old, new| match (old.downcast_ref::<T>(), new.downcast_ref::<T>()) {
            (Some(old), Some(new)) => eq(old, new) == Some(true),
            _ => false,
        },
    )
}

/// The child instance a `Mount` resolved to.
struct Mounted {
    instance: Arc<Instance>,
    /// The component rendering it, if any.
    parent: Option<Arc<Instance>>,
    /// Whether the parent's previous render created it.
    reused: bool,
}

impl Instance {
    fn new(parent: Option<Weak<Instance>>, unmounted: Unmounted) -> Arc<Self> {
        Arc::new(Self {
            parent,
            hooks: Mutex::default(),
            children: Mutex::default(),
            contexts: Mutex::default(),
            cleanups: Mutex::default(),
            unmounted,
            host: Mutex::default(),
            rendered: Mutex::default(),
            observers: Mutex::default(),
            props: Mutex::default(),
            props_changed: Signal::new(()),
            adopt: Mutex::default(),
        })
    }

    fn host(&self) -> Option<Entity> {
        *lock(&self.host)
    }

    /// The latest props, if they are of type `Props<F, O>`.
    fn props<F, O>(&self) -> Option<Arc<Props<F, O>>>
    where
        F: Send + Sync + 'static,
        O: Send + Sync + 'static,
    {
        lock(&self.props).clone()?.downcast().ok()
    }

    /// Returns the instance for the next child mounted during a render,
    /// reusing the one from the previous render with the same key, and
    /// whether it was reused.
    fn child(
        self: &Arc<Self>,
        key: Option<u64>,
        site: &'static Location<'static>,
    ) -> (Arc<Self>, bool) {
        let mut slots = lock(&self.children);
        let key = if let Some(key) = key {
            ChildKey::Key(key)
        } else {
            let nth = slots.sites.entry(site).or_default();
            *nth += 1;
            ChildKey::Site {
                loc: site,
                nth: *nth - 1,
            }
        };
        let previous = slots.previous.remove(&key);
        let reused = previous.is_some();
        let instance = previous
            .unwrap_or_else(|| Self::new(Some(Arc::downgrade(self)), self.unmounted.clone()));
        slots.current.insert(key, Arc::clone(&instance));
        (instance, reused)
    }
}

impl Drop for Instance {
    /// The component unmounted. Its cleanups need `Commands`, so they wait
    /// for the runtime's next pass.
    fn drop(&mut self) {
        let cleanups: Vec<Cleanup> = lock(&self.cleanups)
            .drain(..)
            .filter_map(|cell| lock(&cell).take())
            .collect();
        if !cleanups.is_empty() {
            lock(&self.unmounted.0).extend(cleanups);
        }
    }
}

thread_local! {
    /// Components currently spawning their rendered subtree.
    static RENDERING: RefCell<Vec<Arc<Instance>>> = const { RefCell::new(Vec::new()) };
}

struct RenderingGuard;

impl Drop for RenderingGuard {
    fn drop(&mut self) {
        RENDERING.with(|stack| stack.borrow_mut().pop());
    }
}

fn instance_for(
    key: Option<u64>,
    site: &'static Location<'static>,
    unmounted: Unmounted,
) -> Mounted {
    let parent = RENDERING.with(|stack| stack.borrow().last().cloned());
    match parent {
        Some(parent) => {
            let (instance, reused) = parent.child(key, site);
            Mounted {
                instance,
                parent: Some(parent),
                reused,
            }
        }
        None => Mounted {
            instance: Instance::new(None, unmounted),
            parent: None,
            reused: false,
        },
    }
}

// ---------------------------------------------------------------------------
// Cx and hooks
// ---------------------------------------------------------------------------

/// Render context passed to components. Provides hooks and tracked reads.
///
/// Dereferences to [`Reads`] for tracked ECS access (`get`, `this`,
/// `resource`, `untracked`), and for [`Reads::entity`], the entity hosting
/// this component (the rendered scene is its child).
pub struct Cx<'a, 'w> {
    reads: &'a Reads<'w>,
    instance: &'a Arc<Instance>,
    hook: Cell<usize>,
    effects: RefCell<Vec<PendingEffect>>,
}

impl<'w> Deref for Cx<'_, 'w> {
    type Target = Reads<'w>;

    fn deref(&self) -> &Reads<'w> {
        self.reads
    }
}

impl Cx<'_, '_> {
    fn hook<H, R>(&self, init: impl FnOnce() -> H, use_hook: impl FnOnce(&mut H) -> R) -> R
    where
        H: Any + Send + Sync,
    {
        let index = self.hook.get();
        self.hook.set(index + 1);
        let mut init = Some(init);
        let mut slot = {
            let mut hooks = lock(&self.instance.hooks);
            if index == hooks.len()
                && let Some(init) = init.take()
            {
                hooks.push(Box::new(init()));
            }
            std::mem::replace(&mut hooks[index], Box::new(()))
        };
        if !slot.is::<H>() {
            // Usually an unkeyed component inherited another component's
            // state. Start over from this hook rather than crashing the app.
            warn!(
                "webby: hook #{index} changed type to `{}`; resetting the component's state from                  this hook on. Call hooks in the same order on every render, and give                  conditionally rendered sibling components a `.key(..)`.",
                type_name::<H>(),
            );
            lock(&self.instance.hooks).truncate(index + 1);
            let init = init
                .take()
                .unwrap_or_else(|| unreachable!("a new hook has the right type"));
            slot = Box::new(init());
        }
        let hook = slot
            .downcast_mut::<H>()
            .unwrap_or_else(|| unreachable!("hook #{index} was just initialised"));
        let result = use_hook(hook);
        lock(&self.instance.hooks)[index] = slot;
        result
    }

    /// Like React's `useState`: the current value and a setter function.
    ///
    /// ```ignore
    /// let (count, set_count) = cx.use_state(|| 0);
    /// on(move |_: On<Pointer<Click>>| set_count(count + 1))
    /// ```
    ///
    /// Calling the setter re-renders the component. For updates that depend
    /// on the latest value rather than the rendered one, use
    /// [`use_reducer`](Self::use_reducer) or [`use_signal`](Self::use_signal).
    pub fn use_state<T>(&self, init: impl FnOnce() -> T) -> (T, SetState<T>)
    where
        T: Clone + Send + Sync + 'static,
    {
        let signal = self.use_signal(init);
        let value = signal.get();
        (value, Arc::new(move |value| signal.set(value)))
    }

    /// Like React's `useReducer`: state updated by dispatching actions.
    ///
    /// ```ignore
    /// let (count, dispatch) = cx.use_reducer(|count, delta: i32| count + delta, || 0);
    /// on(move |_: On<Pointer<Click>>| dispatch(1))
    /// ```
    ///
    /// The reducer comes first, as in React's `useReducer(reducer, init)`.
    pub fn use_reducer<T, A>(
        &self,
        reducer: impl Fn(&T, A) -> T + Send + Sync + 'static,
        init: impl FnOnce() -> T,
    ) -> (T, Dispatch<A>)
    where
        T: Clone + Send + Sync + 'static,
    {
        let signal = self.use_signal(init);
        let value = signal.get();
        let dispatch = move |action| signal.update(|state| *state = reducer(state, action));
        (value, Arc::new(dispatch))
    }

    /// A [`Signal`] that persists across renders.
    ///
    /// Reading it with `get` during render re-renders on change. Pass it to
    /// children or event handlers to share state without prop drilling.
    pub fn use_signal<T>(&self, init: impl FnOnce() -> T) -> Signal<T>
    where
        T: Send + Sync + 'static,
    {
        self.hook(
            || Signal::new(init()),
            |signal: &mut Signal<T>| signal.clone(),
        )
    }

    /// Like React's `useMemo`: recomputes only when `deps` change.
    pub fn use_memo<D, T>(&self, deps: D, compute: impl FnOnce(&D) -> T) -> T
    where
        D: PartialEq + Send + Sync + 'static,
        T: Clone + Send + Sync + 'static,
    {
        self.hook(
            || None::<(D, T)>,
            |memo| {
                if let Some((previous, value)) = memo
                    && *previous == deps
                {
                    return value.clone();
                }
                let value = compute(&deps);
                *memo = Some((deps, value.clone()));
                value
            },
        )
    }

    /// Like React's `useEffect`: runs `effect` on mount and whenever `deps`
    /// change. Pass `()` to run once on mount.
    ///
    /// Effects run at the end of the reactive pass, after this component's
    /// subtree, including child components, has rendered; child effects run
    /// before parent effects. `effect` may return a cleanup closure, which
    /// runs before the effect runs again and when the component unmounts:
    ///
    /// ```ignore
    /// cx.use_effect(id, |id, commands| {
    ///     let watcher = commands.spawn(Watching(*id)).id();
    ///     move |commands: &mut Commands| commands.entity(watcher).despawn()
    /// });
    /// ```
    pub fn use_effect<D, C>(
        &self,
        deps: D,
        effect: impl FnOnce(&D, &mut Commands) -> C + Send + Sync + 'static,
    ) where
        D: PartialEq + Clone + Send + Sync + 'static,
        C: IntoCleanup,
    {
        let cell = self.cleanup_cell();
        let changed = self.hook(
            || None::<D>,
            |previous| {
                if previous.as_ref() == Some(&deps) {
                    return false;
                }
                *previous = Some(deps.clone());
                true
            },
        );
        if changed {
            self.effects.borrow_mut().push(Box::new(move |commands| {
                let previous = lock(&cell).take();
                if let Some(cleanup) = previous {
                    cleanup(commands);
                }
                let cleanup = effect(&deps, commands).into_cleanup();
                *lock(&cell) = cleanup;
            }));
        }
    }

    /// Runs `f` when the component unmounts: when its host is despawned, or
    /// when a parent render no longer includes it. The latest `f` passed is
    /// the one that runs.
    pub fn on_unmount(&self, f: impl FnOnce(&mut Commands) + Send + Sync + 'static) {
        let cell = self.cleanup_cell();
        *lock(&cell) = Some(Box::new(f));
    }

    /// A cleanup slot that persists across renders and runs on unmount.
    fn cleanup_cell(&self) -> CleanupCell {
        self.hook(
            || {
                let cell = CleanupCell::default();
                lock(&self.instance.cleanups).push(Arc::clone(&cell));
                cell
            },
            |cell: &mut CleanupCell| Arc::clone(cell),
        )
    }

    /// Runs an async task for `deps` and re-renders when it finishes.
    ///
    /// The first render with new `deps` starts `start(deps)` on Bevy's
    /// [`AsyncComputeTaskPool`] and returns [`TaskState::Pending`]; once the
    /// future completes, the component re-renders and gets
    /// [`TaskState::Ready`] with a clone of its output until `deps` change.
    /// Changing `deps` drops the running task, which cancels it, and so does
    /// unmounting the component (despawning its host).
    ///
    /// ```ignore
    /// #[component]
    /// fn profile(cx: &Cx, user_id: u32) -> impl Scene {
    ///     let name = match cx.use_task(user_id, |id| async move { load_user(id).await }) {
    ///         TaskState::Pending => "Loading...".to_string(),
    ///         TaskState::Ready(user) => user.name,
    ///     };
    ///     bsn! { Text({name}) }
    /// }
    /// ```
    ///
    /// [`AsyncComputeTaskPool`]: bevy::tasks::AsyncComputeTaskPool
    pub fn use_task<D, T, F, Fut>(&self, deps: D, start: F) -> TaskState<T>
    where
        D: PartialEq + Clone + Send + Sync + 'static,
        T: Clone + Send + Sync + 'static,
        F: FnOnce(D) -> Fut,
        Fut: Future<Output = T> + Send + 'static,
    {
        let slot = self.hook(
            || None::<(D, Arc<TaskSlot<T>>)>,
            |hook| {
                if let Some((previous, slot)) = hook
                    && *previous == deps
                {
                    return Arc::clone(slot);
                }
                let pool = AsyncComputeTaskPool::get_or_init(TaskPool::default);
                let slot = Arc::new(TaskSlot {
                    task: Mutex::new(Some(pool.spawn(start(deps.clone())))),
                    output: Mutex::new(None),
                    done: Signal::new(()),
                });
                if let Some(pending) = self.reads.untracked().get_resource::<PendingTasks>() {
                    let weak: Weak<dyn PollTask> = Arc::downgrade(&slot) as Weak<dyn PollTask>;
                    lock(&pending.0).push(weak);
                }
                // Replacing the previous slot drops its task, cancelling it.
                *hook = Some((deps, Arc::clone(&slot)));
                slot
            },
        );
        slot.done.with(|()| {});
        lock(&slot.output)
            .clone()
            .map_or(TaskState::Pending, TaskState::Ready)
    }

    /// Like React's `useRef`: a mutable box that persists across renders
    /// without triggering them.
    pub fn use_ref<T>(&self, init: impl FnOnce() -> T) -> Ref<T>
    where
        T: Send + Sync + 'static,
    {
        self.hook(
            || Ref(Arc::new(Mutex::new(init()))),
            |reference: &mut Ref<T>| reference.clone(),
        )
    }

    /// Clones resource `R`; the component re-renders when it changes.
    ///
    /// Shorthand for `cx.resource::<R>().cloned()` (see [`Reads::resource`]).
    #[must_use]
    pub fn use_resource<R: Resource + Clone>(&self) -> Option<R> {
        self.reads.resource::<R>().cloned()
    }

    /// Clones component `C` on `entity`; the component re-renders when it
    /// changes.
    ///
    /// Shorthand for `cx.get::<C>(entity).cloned()` (see [`Reads::get`]).
    #[must_use]
    pub fn use_component<C: Component + Clone>(&self, entity: Entity) -> Option<C> {
        self.reads.get::<C>(entity).cloned()
    }

    /// Like a React context provider: makes `value` available to
    /// [`use_context`](Self::use_context) in descendant components.
    ///
    /// Provide a [`Signal`] for values that change, so consumers that read
    /// it re-render.
    pub fn provide_context<T: Clone + Send + Sync + 'static>(&self, value: T) {
        lock(&self.instance.contexts).insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Like React's `useContext`: the nearest ancestor's provided `T`.
    #[must_use]
    pub fn use_context<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        let mut ancestor = self.instance.parent.as_ref().and_then(Weak::upgrade);
        while let Some(instance) = ancestor {
            if let Some(value) = lock(&instance.contexts)
                .get(&TypeId::of::<T>())
                .and_then(|value| value.downcast_ref::<T>())
            {
                return Some(value.clone());
            }
            ancestor = instance.parent.as_ref().and_then(Weak::upgrade);
        }
        None
    }
}

/// Progress of a [`Cx::use_task`] task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskState<T> {
    /// The task is running.
    Pending,
    /// The task finished with this output.
    Ready(T),
}

impl<T> TaskState<T> {
    /// The output, if the task finished.
    #[must_use]
    pub fn ready(self) -> Option<T> {
        match self {
            Self::Pending => None,
            Self::Ready(value) => Some(value),
        }
    }

    /// Whether the task is still running.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// A `use_task` hook's running task and its output.
struct TaskSlot<T> {
    task: Mutex<Option<Task<T>>>,
    output: Mutex<Option<T>>,
    /// Notified when the output arrives, re-rendering the component.
    done: Signal<()>,
}

impl<T: Send + Sync + 'static> PollTask for TaskSlot<T> {
    fn poll_task(&self) -> bool {
        let mut task = lock(&self.task);
        let Some(output) = task.as_mut().and_then(check_ready) else {
            return task.is_none();
        };
        *task = None;
        drop(task);
        *lock(&self.output) = Some(output);
        self.done.notify();
        true
    }
}

/// Setter returned by [`Cx::use_state`]. Call it like React's `setCount`:
/// `set_count(count + 1)`. Cheap to clone into event handlers.
pub type SetState<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Dispatcher returned by [`Cx::use_reducer`]: `dispatch(Action::Increment)`.
pub type Dispatch<A> = Arc<dyn Fn(A) + Send + Sync>;

/// Mutable box returned by [`Cx::use_ref`].
pub struct Ref<T>(Arc<Mutex<T>>);

impl<T> Ref<T> {
    /// Locks the value for reading or writing.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        lock(&self.0)
    }

    /// Clones the value.
    #[must_use]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.lock().clone()
    }

    /// Replaces the value.
    pub fn set(&self, value: T) {
        *self.lock() = value;
    }
}

impl<T> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

// ---------------------------------------------------------------------------
// Mounting
// ---------------------------------------------------------------------------

/// A mounted component: a BSN template that renders a component on the
/// current entity. Returned by `#[component]` functions and [`component`](fn@component).
///
/// The rendered scene is spawned as the entity's child. An entity without a
/// UI `Node` becomes a `GhostNode` (with the `ui` feature), so components can
/// be used directly as list items without affecting layout.
///
/// An unkeyed component is identified by the source location that created
/// it (the `@name(..)` call, through `#[track_caller]`) and how many
/// components that location created before it in the same parent render.
/// Adding or removing a conditional sibling therefore does not move another
/// component's state; components created in a loop still match by position
/// and should be keyed.
///
/// When a parent re-renders, a matched component keeps its host entity and
/// renders again only if its props changed; see the
/// [module docs](self#rendering-model).
pub struct Mount<D, F> {
    deps: D,
    key: Option<u64>,
    site: &'static Location<'static>,
    render: F,
    memo: Option<PropsMemo>,
}

impl<D, F> Mount<D, F> {
    /// Gives the component a key, like `<Row key={id} />`. Keyed components
    /// keep their state when siblings are inserted, removed, or reordered.
    #[must_use]
    pub fn key(mut self, key: impl Hash) -> Self {
        self.key = Some(FixedState::default().hash_one(key));
        self
    }
}

impl<D: Deps, F> Mount<D, F> {
    /// Lets a parent re-render skip re-rendering this component when
    /// `props_eq` and `deps_eq` find its props unchanged. Generated by
    /// `#[component]`; `None` from either means "changed".
    #[doc(hidden)]
    #[must_use]
    pub fn __memo<P: Send + Sync + 'static>(
        mut self,
        props: Arc<P>,
        props_eq: fn(&P, &P) -> Option<bool>,
        deps_eq: fn(&D::Output, &D::Output) -> Option<bool>,
    ) -> Self {
        self.memo = Some(PropsMemo {
            props,
            props_eq: erase_eq(props_eq),
            deps_eq: erase_eq(deps_eq),
        });
        self
    }
}

/// Adds a key to a mounted component inside `bsn!`:
/// `@keyed(todo.id, todo_row(todo))`.
pub fn keyed<D, F>(key: impl Hash, mount: Mount<D, F>) -> Mount<D, F> {
    mount.key(key)
}

impl<D, F> Template for Mount<D, F>
where
    D: Deps,
    F: Render<D::Output>,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        ensure_runtime(context)?;
        let deps = self.deps.resolve(context)?;
        let unmounted = context
            .entity
            .world_scope(|world| world.resource::<Unmounted>().clone());
        let Mounted {
            instance,
            parent,
            reused,
        } = instance_for(self.key, self.site, unmounted);
        let props = Arc::new(Props {
            render: self.render.clone(),
            deps,
            memo: self.memo.clone(),
        });
        // A component the parent's previous render created keeps its host:
        // the parent moves the host into this entity's place once its scene
        // has spawned, and the host re-renders only if its props changed.
        if let Some(parent) = parent
            && reused
            && let Some(host) = instance.host()
            && context.entity.world_scope(|world| {
                world
                    .get_entity(host)
                    .is_ok_and(|host| host.contains::<Effects>())
            })
            && let Some(old) = instance.props::<F, D::Output>()
        {
            let same = match (&props.memo, &old.memo) {
                (Some(new), Some(old_memo)) => new.same(old_memo, &props.deps, &old.deps),
                _ => false,
            };
            if !same {
                *lock(&instance.props) = Some(props);
                instance.props_changed.notify();
            }
            lock(&parent.adopt).push((context.entity.id(), instance));
            return Ok(EmptySceneEffect);
        }
        *lock(&instance.props) = Some(props);
        attach_component(context, render_loop::<D::Output, F>(instance));
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            deps: self.deps.clone_deps(),
            key: self.key,
            site: self.site,
            render: self.render.clone(),
            memo: self.memo.clone(),
        }
    }
}

impl<D, F> Scene for Mount<D, F>
where
    D: Deps,
    F: Render<D::Output>,
{
    fn resolve(
        self,
        _context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        scene.push_template(self);
        Ok(())
    }
}

fn render_loop<O, F>(instance: Arc<Instance>) -> RunFn
where
    O: Clone + Send + Sync + 'static,
    F: Render<O>,
{
    let mut mounted = false;
    Box::new(move |world: &mut World, target| {
        if !mounted {
            mounted = true;
            *lock(&instance.host) = Some(target);
            // Effects have not run yet, so these were declared by the scene.
            *lock(&instance.observers) = world
                .get::<ObservedBy>(target)
                .map(|observers| observers.get().to_vec())
                .unwrap_or_default();
            make_layout_transparent(world, target);
        }
        let Some(props) = instance.props::<F, O>() else {
            return Vec::new().into();
        };
        let ((scene, effects), read_deps) = tracked(world, target, |reads| {
            let cx = Cx {
                reads,
                instance: &instance,
                hook: Cell::new(0),
                effects: RefCell::default(),
            };
            instance.props_changed.with(|()| {});
            let scene = props.render.render(&cx, props.deps.clone());
            (scene, cx.effects.into_inner())
        });

        {
            let mut slots = lock(&instance.children);
            slots.previous = std::mem::take(&mut slots.current);
            slots.sites.clear();
        }
        RENDERING.with(|stack| stack.borrow_mut().push(Arc::clone(&instance)));
        let guard = RenderingGuard;
        let mut child = spawn_child(world, target, scene);
        drop(guard);

        // Kept children move into the new scene. Nested ones were mounted
        // after their ancestors, so they move first and travel with them.
        let adopted = std::mem::take(&mut *lock(&instance.adopt));
        let mut kept = Vec::with_capacity(adopted.len());
        for (spawned, kid) in adopted.into_iter().rev() {
            if let Some(host) = adopt(world, spawned, &kid) {
                if child == Some(spawned) {
                    child = Some(host);
                }
                kept.push(host);
            }
        }
        let old = std::mem::replace(&mut *lock(&instance.rendered), child);
        if let Some(old) = old
            && Some(old) != child
            && !kept.contains(&old)
            && world.get_entity(old).is_ok()
        {
            world.despawn(old);
        }
        // Children that were not rendered again unmount with their state.
        lock(&instance.children).previous.clear();

        if !effects.is_empty() {
            world
                .resource_mut::<PostEffects>()
                .0
                .push(Box::new(move |world: &mut World| {
                    let mut queue = CommandQueue::default();
                    let mut commands = Commands::new(&mut queue, world);
                    for effect in effects {
                        effect(&mut commands);
                    }
                    queue.apply(world);
                }));
        }
        read_deps.into()
    })
}

/// Moves the host of the kept child instance `kid` into the place of
/// `spawned`, the entity the parent's new scene spawned for it, and
/// despawns `spawned`. Returns the host.
///
/// The host keeps its entity, rendered subtree and hook state. What the
/// parent's new scene declared beside the component replaces what its
/// previous scene declared: components are moved over (overwriting those
/// of the same type), and effects, observers and children are swapped.
/// Components the previous scene declared and the new one does not are
/// left in place.
fn adopt(world: &mut World, spawned: Entity, kid: &Instance) -> Option<Entity> {
    let host = kid.host()?;
    if world.get_entity(spawned).is_err() || world.get_entity(host).is_err() {
        return None;
    }
    let rendered = *lock(&kid.rendered);
    adopt_effects(world, spawned, host);

    // Children declared beside the component.
    let stale: Vec<Entity> = world
        .get::<Children>(host)
        .map(|children| {
            children
                .iter()
                .filter(|&child| Some(child) != rendered)
                .collect()
        })
        .unwrap_or_default();
    for child in stale {
        world.despawn(child);
    }
    let declared: Vec<Entity> = world
        .get::<Children>(spawned)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    if !declared.is_empty() {
        world.entity_mut(host).insert_children(0, &declared);
    }

    // Observers declared beside the component.
    let observers = world
        .get::<ObservedBy>(spawned)
        .map(|observers| observers.get().to_vec())
        .unwrap_or_default();
    let add_observers = !observers.is_empty();
    for observer in std::mem::replace(&mut *lock(&kid.observers), observers) {
        if world.get_entity(observer).is_ok() {
            world.despawn(observer);
        }
    }

    world
        .entity_mut(spawned)
        .clone_with_opt_out(host, move |builder| {
            builder
                .deny::<(ChildOf, Children, Effects)>()
                .move_components(true)
                .add_observers(add_observers);
        });

    match world.get::<ChildOf>(spawned).map(ChildOf::parent) {
        Some(parent) => {
            let index = world
                .get::<Children>(parent)
                .and_then(|children| children.iter().position(|child| child == spawned));
            let mut parent = world.entity_mut(parent);
            match index {
                Some(index) => parent.insert_children(index, &[host]),
                None => parent.add_child(host),
            };
        }
        None => {
            world.entity_mut(host).remove::<ChildOf>();
        }
    }
    world.despawn(spawned);
    Some(host)
}

#[cfg(feature = "ui")]
fn make_layout_transparent(world: &mut World, host: Entity) {
    use bevy::ui::{Node, experimental::GhostNode};
    if let Ok(mut entity) = world.get_entity_mut(host)
        && !entity.contains::<Node>()
        && !entity.contains::<GhostNode>()
    {
        entity.insert(GhostNode);
        crate::runtime::strip_ghost_transform(world, host);
    }
}

#[cfg(not(feature = "ui"))]
fn make_layout_transparent(_: &mut World, _: Entity) {}

/// Mounts a closure as a component: `component(|cx| counter(cx, 5))`.
///
/// Prefer `#[component]` functions, which are used as `@counter(5)`.
#[track_caller]
pub fn component<F, S>(render: F) -> Mount<(), impl Render<()>>
where
    F: Fn(&Cx) -> S + Clone + Send + Sync + 'static,
    S: Scene,
{
    component_with((), move |cx, ()| render(cx))
}

/// Mounts a closure that receives entities resolved from the scene, such as
/// `#Player` or `[#A, #B]`: `component_with(#Player, |cx, player| ..)`.
#[track_caller]
pub fn component_with<D, F, S>(entities: D, render: F) -> Mount<D, F>
where
    D: Deps,
    F: Fn(&Cx, D::Output) -> S + Clone + Send + Sync + 'static,
    S: Scene,
{
    Mount {
        deps: entities,
        key: None,
        site: Location::caller(),
        render,
        memo: None,
    }
}

/// Compares two props values with `PartialEq` when their type has it, through
/// autoref specialisation: `(&__Probe(a, b)).__props_eq()` is `Some(a == b)`
/// for `PartialEq` types and `None` otherwise. Used by `#[component]`.
#[doc(hidden)]
pub struct __Probe<'a, T>(pub &'a T, pub &'a T);

#[doc(hidden)]
pub trait __ProbeEq {
    fn __props_eq(&self) -> Option<bool>;
}

impl<T: PartialEq> __ProbeEq for __Probe<'_, T> {
    fn __props_eq(&self) -> Option<bool> {
        Some(self.0 == self.1)
    }
}

#[doc(hidden)]
pub trait __ProbeOpaque {
    fn __props_eq(&self) -> Option<bool>;
}

impl<T> __ProbeOpaque for &__Probe<'_, T> {
    fn __props_eq(&self) -> Option<bool> {
        None
    }
}

/// Reports a failed render of a `-> Result<impl Scene>` component to Bevy's
/// [`FallbackErrorHandler`](bevy::ecs::error::FallbackErrorHandler) and
/// renders nothing.
#[doc(hidden)]
pub fn __render_result<S: Scene>(
    cx: &Cx,
    component: &'static str,
    result: Result<S, impl Into<BevyError>>,
) -> Option<S> {
    use bevy::ecs::error::{ErrorContext, ErrorHandler, FallbackErrorHandler};
    match result {
        Ok(scene) => Some(scene),
        Err(error) => {
            let handler = cx
                .untracked()
                .get_resource::<FallbackErrorHandler>()
                .map_or(bevy::ecs::error::error as ErrorHandler, |handler| handler.0);
            let name = format!("webby component `{component}` on {}", cx.entity());
            handler(error.into(), ErrorContext::Command { name: name.into() });
            None
        }
    }
}

/// One of two scenes of different types, returned by [`either`].
///
/// Lets the branches of an `if` render different `bsn!` scenes without
/// boxing them.
pub enum Either<A, B> {
    /// The scene of the first branch.
    Left(A),
    /// The scene of the second branch.
    Right(B),
}

impl<A: Scene, B: Scene> Scene for Either<A, B> {
    fn resolve(
        self,
        context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        match self {
            Self::Left(left) => left.resolve(context, scene),
            Self::Right(right) => right.resolve(context, scene),
        }
    }

    fn register_dependencies(&self, dependencies: &mut SceneDependencies) {
        match self {
            Self::Left(left) => left.register_dependencies(dependencies),
            Self::Right(right) => right.register_dependencies(dependencies),
        }
    }
}

/// Builds `left()` when `condition` holds and `right()` otherwise; the two
/// scenes may have different types.
///
/// ```ignore
/// either(done, || bsn! { Text("done") }, || bsn! { Button Children [ Text("finish") ] })
/// ```
pub fn either<A: Scene, B: Scene>(
    condition: bool,
    left: impl FnOnce() -> A,
    right: impl FnOnce() -> B,
) -> Either<A, B> {
    if condition {
        Either::Left(left())
    } else {
        Either::Right(right())
    }
}

/// A component's render function, taking its entity props `P`.
///
/// Implemented for every `Fn(&Cx, P) -> impl Scene` closure.
pub trait Render<P>: Clone + Send + Sync + 'static {
    /// Scene produced by one render.
    type Output: Scene;

    /// Renders the component.
    fn render(&self, cx: &Cx, props: P) -> Self::Output;
}

impl<P, S, F> Render<P> for F
where
    S: Scene,
    F: Fn(&Cx, P) -> S + Clone + Send + Sync + 'static,
{
    type Output = S;

    fn render(&self, cx: &Cx, props: P) -> S {
        self(cx, props)
    }
}
