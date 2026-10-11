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
//! | `createContext` / `useContext`         | [`Cx::provide_context`] / [`Cx::use_context`]   |
//! | store/selector hooks                   | [`Cx::use_resource`] / [`Cx::use_component`]    |
//! | `onClick={...}`                        | BSN `on(move \|_: On<Pointer<Click>>\| ...)`    |
//!
//! # Props
//!
//! - The first argument may be `cx: &Cx`; it is optional.
//! - `Entity` props accept `#Name` references from the surrounding scene, or
//!   an `Entity` value.
//! - Other props must be `Clone + Send + Sync + 'static`; they are cloned
//!   into each render.
//! - `-> impl Scene` never borrows from `cx`; the macro adds `use<..>`.
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
//! - A render replaces the rendered subtree. Child components keep their
//!   hook state across parent renders, matched by position or key, like
//!   React keeps component instances. Plain entities in the subtree are
//!   recreated.
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
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use bevy::{
    ecs::{
        template::{Template, TemplateContext},
        world::CommandQueue,
    },
    platform::{collections::HashMap, hash::FixedState},
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene},
};

use crate::runtime::{RunFn, attach, ensure_runtime, spawn_child, tracked};
pub use crate::{
    coat::{Signal, cloned},
    macros::component,
    runtime::{Deps, Effects, Reads},
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// Component instances
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ChildKey {
    Position(usize),
    Key(u64),
}

#[derive(Default)]
struct ChildSlots {
    previous: HashMap<ChildKey, Arc<Instance>>,
    current: HashMap<ChildKey, Arc<Instance>>,
    position: usize,
}

type PendingEffect = Box<dyn FnOnce(&mut Commands) + Send + Sync>;

/// State of one mounted component: hooks, child instances and contexts.
struct Instance {
    parent: Option<Arc<Instance>>,
    hooks: Mutex<Vec<Box<dyn Any + Send + Sync>>>,
    children: Mutex<ChildSlots>,
    contexts: Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

impl Instance {
    fn new(parent: Option<Arc<Instance>>) -> Arc<Self> {
        Arc::new(Self {
            parent,
            hooks: Mutex::default(),
            children: Mutex::default(),
            contexts: Mutex::default(),
        })
    }

    /// Returns the instance for the next child mounted during a render,
    /// reusing the one from the previous render with the same key.
    fn child(self: &Arc<Self>, key: Option<u64>) -> Arc<Self> {
        let mut slots = lock(&self.children);
        let key = if let Some(key) = key {
            ChildKey::Key(key)
        } else {
            slots.position += 1;
            ChildKey::Position(slots.position - 1)
        };
        let instance = slots
            .previous
            .remove(&key)
            .unwrap_or_else(|| Self::new(Some(Arc::clone(self))));
        slots.current.insert(key, Arc::clone(&instance));
        instance
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

fn instance_for(key: Option<u64>) -> Arc<Instance> {
    RENDERING
        .with(|stack| stack.borrow().last().cloned())
        .map_or_else(|| Instance::new(None), |parent| parent.child(key))
}

// ---------------------------------------------------------------------------
// Cx and hooks
// ---------------------------------------------------------------------------

/// Render context passed to components. Provides hooks and tracked reads.
///
/// Dereferences to [`Reads`] for tracked ECS access (`get`, `this`,
/// `resource`, `untracked`).
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
    /// Entity hosting this component; the rendered scene is its child.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.reads.target()
    }

    fn hook<H, R>(&self, init: impl FnOnce() -> H, use_hook: impl FnOnce(&mut H) -> R) -> R
    where
        H: Any + Send + Sync,
    {
        let index = self.hook.get();
        self.hook.set(index + 1);
        let mut slot = {
            let mut hooks = lock(&self.instance.hooks);
            if index == hooks.len() {
                hooks.push(Box::new(init()));
            }
            std::mem::replace(&mut hooks[index], Box::new(()))
        };
        let hook = slot.downcast_mut::<H>().unwrap_or_else(|| {
            panic!(
                "hook #{index} changed type to `{}`; call hooks in the same order on every render",
                type_name::<H>(),
            )
        });
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
    /// let (count, dispatch) = cx.use_reducer(|| 0, |count, delta: i32| count + delta);
    /// on(move |_: On<Pointer<Click>>| dispatch(1))
    /// ```
    pub fn use_reducer<T, A>(
        &self,
        init: impl FnOnce() -> T,
        reducer: impl Fn(&T, A) -> T + Send + Sync + 'static,
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

    /// Like React's `useEffect`: runs `effect` after the render is spawned,
    /// on mount and whenever `deps` change. Pass `()` to run once on mount.
    pub fn use_effect<D>(
        &self,
        deps: D,
        effect: impl FnOnce(&D, &mut Commands) + Send + Sync + 'static,
    ) where
        D: PartialEq + Clone + Send + Sync + 'static,
    {
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
            self.effects
                .borrow_mut()
                .push(Box::new(move |commands| effect(&deps, commands)));
        }
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
    #[must_use]
    pub fn use_resource<R: Resource + Clone>(&self) -> Option<R> {
        self.reads.resource::<R>().cloned()
    }

    /// Clones component `C` on `entity`; the component re-renders when it
    /// changes.
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
        let mut ancestor = self.instance.parent.clone();
        while let Some(instance) = ancestor {
            if let Some(value) = lock(&instance.contexts)
                .get(&TypeId::of::<T>())
                .and_then(|value| value.downcast_ref::<T>())
            {
                return Some(value.clone());
            }
            #[expect(
                clippy::assigning_clones,
                reason = "`instance` is moved out of `ancestor`"
            )]
            {
                ancestor = instance.parent.clone();
            }
        }
        None
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
/// current entity. Returned by `#[component]` functions and [`component`].
///
/// The rendered scene is spawned as the entity's child. An entity without a
/// UI `Node` becomes a `GhostNode` (with the `ui` feature), so components can
/// be used directly as list items without affecting layout.
pub struct Mount<D, F> {
    deps: D,
    key: Option<u64>,
    render: F,
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
        let run = render_loop(instance_for(self.key), self.render.clone(), deps);
        attach(context, run);
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            deps: self.deps.clone_deps(),
            key: self.key,
            render: self.render.clone(),
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

fn render_loop<O, F>(instance: Arc<Instance>, render: F, deps: O) -> RunFn
where
    O: Copy + Send + Sync + 'static,
    F: Render<O>,
{
    let mut child: Option<Entity> = None;
    let mut mounted = false;
    Box::new(move |world: &mut World, target| {
        if !mounted {
            mounted = true;
            make_layout_transparent(world, target);
        }
        let ((scene, effects), read_deps) = tracked(world, target, |reads| {
            let cx = Cx {
                reads,
                instance: &instance,
                hook: Cell::new(0),
                effects: RefCell::default(),
            };
            let scene = render.render(&cx, deps);
            (scene, cx.effects.into_inner())
        });

        if let Some(old) = child.take() {
            world.despawn(old);
        }
        {
            let mut slots = lock(&instance.children);
            slots.previous = std::mem::take(&mut slots.current);
            slots.position = 0;
        }
        RENDERING.with(|stack| stack.borrow_mut().push(Arc::clone(&instance)));
        let guard = RenderingGuard;
        child = spawn_child(world, target, scene);
        drop(guard);
        // Children that were not rendered again unmount with their state.
        lock(&instance.children).previous.clear();

        if !effects.is_empty() {
            let mut queue = CommandQueue::default();
            let mut commands = Commands::new(&mut queue, world);
            for effect in effects {
                effect(&mut commands);
            }
            queue.apply(world);
        }
        read_deps
    })
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
pub fn component<F, S>(render: F) -> Mount<(), impl Render<()>>
where
    F: Fn(&Cx) -> S + Clone + Send + Sync + 'static,
    S: Scene,
{
    component_with((), move |cx, ()| render(cx))
}

/// Mounts a closure that receives entities resolved from the scene, such as
/// `#Player` or `[#A, #B]`: `component_with(#Player, |cx, player| ..)`.
pub fn component_with<D, F, S>(entities: D, render: F) -> Mount<D, F>
where
    D: Deps,
    F: Fn(&Cx, D::Output) -> S + Clone + Send + Sync + 'static,
    S: Scene,
{
    Mount {
        deps: entities,
        key: None,
        render,
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
