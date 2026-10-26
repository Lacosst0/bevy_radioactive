//! React-style function components with hooks.
//!
//! Mark a function with [`#[component]`](macro@component) and use it in
//! `bsn!` like a scene function: `@counter(5)`. The optional `cx` argument is
//! supplied by the runtime; the other arguments are props. The component
//! re-renders when its state, or ECS data it read through `cx`, changes:
//!
//! ```ignore
//! use bevy_radioactive::hooks::*;
//!
//! app.add_plugins(HooksPlugin);
//!
//! #[component]
//! fn counter(cx: &Cx, start: i32) -> impl Scene {
//!     let (count, set_count) = cx.use_state(|| start);
//!     bsn! {
//!         Button
//!         on(move |_: On<Pointer<Click>>| set_count(count + 1))
//!         Children [ Text({count.to_string()}) ]
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
//!     let rows: Vec<_> = cx.use_resource::<Todos>().unwrap_or_default().0
//!         .into_iter()
//!         .map(|todo| todo_row(todo.title).key(todo.id))
//!         .collect();
//!     bsn! {
//!         Node
//!         Children [
//!             #Player Health(100)
//!             --
//!             @counter(5)               // <Counter start={5} />
//!             --
//!             @health_bar(#Player)      // `Entity` props accept `#Name`
//!             --
//!             Node Children [ {rows} ]  // {todos.map(t => <TodoRow key={t.id} />)}
//!         ]
//!     }
//! }
//! ```
//!
//! | React | `hooks` |
//! | --- | --- |
//! | `<Counter start={5} />` | `@counter(5)` |
//! | `<Row key={id} />` | `row(..).key(id)` or `@keyed(id, row(..))` |
//! | inline component | `component(\|cx\| bsn! { .. })` |
//! | `useState` / `useReducer` | [`Cx::use_state`] / [`Cx::use_reducer`] |
//! | `useMemo` / `useEffect` / `useRef` | [`Cx::use_memo`] / [`Cx::use_effect`] / [`Cx::use_ref`] |
//! | `createContext` / `useContext` | [`Cx::provide_context`] / [`Cx::use_context`] |
//! | store/selector hooks | [`Cx::use_resource`] / [`Cx::use_component`] |
//! | `use(promise)` with Suspense | [`Cx::use_task`] |
//!
//! # Rendering
//!
//! - The rendered scene is a child of the component's entity. An entity
//!   without a UI `Node` becomes a `GhostNode` (with the `ui` feature), so a
//!   component can be a list item without affecting layout.
//! - A render replaces the rendered subtree. Child components keep their
//!   state, matched by key, or else by the `@name(..)` call that created them.
//! - Call hooks in the same order on every render.
//! - Props other than `Entity` must be `Clone + Send + Sync + 'static`.
//! - Renders run in [`HooksSystems`] in `PostUpdate`, before UI layout.

use std::{
    any::{Any, TypeId, type_name},
    cell::{Cell, RefCell},
    hash::{BuildHasher, Hash},
    panic::Location,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use bevy::{
    ecs::{
        change_detection::Tick,
        component::ComponentId,
        entity::EntityHashSet,
        entity_disabling::Disabled,
        lifecycle::HookContext,
        template::{EntityTemplate, Template, TemplateContext},
        world::{CommandQueue, DeferredWorld},
    },
    platform::{collections::HashMap, hash::FixedState},
    prelude::*,
    scene::{
        EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene, WorldSceneExt,
    },
    tasks::{AsyncComputeTaskPool, Task, TaskPool, futures::check_ready},
};

pub use crate::macros::component;

/// Rounds of rendering per frame, so state set during a render settles in
/// one frame.
const MAX_ROUNDS: usize = 8;

/// Renders components each frame. Required by `#[component]` and
/// [`component()`].
#[derive(Default)]
pub struct HooksPlugin;

impl Plugin for HooksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hosts>()
            .add_systems(PostUpdate, render_all.in_set(HooksSystems));
        #[cfg(feature = "ui")]
        app.configure_sets(
            PostUpdate,
            HooksSystems.before(bevy::ui::UiSystems::Prepare),
        );
    }
}

/// System set in `PostUpdate` that renders components.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HooksSystems;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// Component instances
// ---------------------------------------------------------------------------

/// Identity of a child component within its parent's render.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ChildKey {
    /// Unkeyed: the source location of the `@name(..)` call, and how many
    /// components that location created earlier in the same render.
    Site {
        site: &'static Location<'static>,
        nth: u32,
    },
    /// Keyed with [`Element::key`].
    Key(u64),
}

#[derive(Default)]
struct ChildSlots {
    previous: HashMap<ChildKey, Arc<Instance>>,
    current: HashMap<ChildKey, Arc<Instance>>,
    sites: HashMap<&'static Location<'static>, u32>,
}

type PendingEffect = Box<dyn FnOnce(&mut Commands) + Send + Sync>;

/// One mounted component: hook state, child instances and contexts.
///
/// A parent owns its child instances; the link back is weak, so despawning
/// a host frees its whole instance tree.
struct Instance {
    parent: Option<Weak<Instance>>,
    /// Bumped by state setters; a change re-renders the component.
    version: AtomicU64,
    hooks: Mutex<Vec<Box<dyn Any + Send + Sync>>>,
    children: Mutex<ChildSlots>,
    contexts: Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

impl Instance {
    fn new(parent: Option<Weak<Instance>>) -> Arc<Self> {
        Arc::new(Self {
            parent,
            version: AtomicU64::new(0),
            hooks: Mutex::default(),
            children: Mutex::default(),
            contexts: Mutex::default(),
        })
    }

    /// The instance for the next child mounted during a render, reusing the
    /// one from the previous render with the same key, or the same call site
    /// and occurrence.
    fn child(self: &Arc<Self>, key: Option<u64>, site: &'static Location<'static>) -> Arc<Self> {
        let mut slots = lock(&self.children);
        let key = if let Some(key) = key {
            ChildKey::Key(key)
        } else {
            let nth = slots.sites.entry(site).or_default();
            *nth += 1;
            ChildKey::Site {
                site,
                nth: *nth - 1,
            }
        };
        let instance = slots
            .previous
            .remove(&key)
            .unwrap_or_else(|| Self::new(Some(Arc::downgrade(self))));
        slots.current.insert(key, Arc::clone(&instance));
        instance
    }

    fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    fn invalidate(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
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

fn instance_for(key: Option<u64>, site: &'static Location<'static>) -> Arc<Instance> {
    RENDERING
        .with(|stack| stack.borrow().last().cloned())
        .map_or_else(|| Instance::new(None), |parent| parent.child(key, site))
}

// ---------------------------------------------------------------------------
// Cx and hooks
// ---------------------------------------------------------------------------

/// Render context passed to components.
pub struct Cx<'a> {
    world: &'a World,
    entity: Entity,
    instance: &'a Arc<Instance>,
    hook: Cell<usize>,
    reads: RefCell<Vec<Read>>,
    effects: RefCell<Vec<PendingEffect>>,
}

impl Cx<'_> {
    /// Entity hosting this component; the rendered scene is its child.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// Read-only world access. Reads through it do not re-render.
    #[must_use]
    pub fn world(&self) -> &World {
        self.world
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
                hooks.push(Box::new(()));
            }
            std::mem::replace(&mut hooks[index], Box::new(()))
        };
        if !slot.is::<H>() {
            // A new slot holds `()`. Anything else means hooks were called
            // in a different order than on the previous render.
            if !slot.is::<()>() {
                warn!(
                    "hooks: hook #{index} changed type to `{}`, so its state was reset; call \
                     hooks in the same order on every render, or give conditional \
                     components a `.key(..)`",
                    type_name::<H>(),
                );
            }
            slot = Box::new(init());
        }
        let hook = slot.downcast_mut::<H>().expect("slot was just checked");
        let result = use_hook(hook);
        lock(&self.instance.hooks)[index] = slot;
        result
    }

    /// Like React's `useState`: the current value and a setter.
    ///
    /// ```ignore
    /// let (count, set_count) = cx.use_state(|| 0);
    /// on(move |_: On<Pointer<Click>>| set_count(count + 1))
    /// ```
    pub fn use_state<T>(&self, init: impl FnOnce() -> T) -> (T, SetState<T>)
    where
        T: Clone + Send + Sync + 'static,
    {
        let cell = self.hook(|| Arc::new(Mutex::new(init())), |cell| Arc::clone(cell));
        let value = lock(&cell).clone();
        let instance = Arc::clone(self.instance);
        let set = move |value| {
            *lock(&cell) = value;
            instance.invalidate();
        };
        (value, Arc::new(set))
    }

    /// Like React's `useReducer`: state updated by dispatching actions to
    /// `reducer`, which always sees the latest state.
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
        let cell = self.hook(|| Arc::new(Mutex::new(init())), |cell| Arc::clone(cell));
        let value = lock(&cell).clone();
        let instance = Arc::clone(self.instance);
        let dispatch = move |action| {
            let mut state = lock(&cell);
            *state = reducer(&state, action);
            drop(state);
            instance.invalidate();
        };
        (value, Arc::new(dispatch))
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
    /// on mount and whenever `deps` change. Pass `()` to run once.
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

    /// Like React's `useRef`: a value that persists across renders without
    /// triggering them.
    pub fn use_ref<T>(&self, init: impl FnOnce() -> T) -> Arc<Mutex<T>>
    where
        T: Send + Sync + 'static,
    {
        self.hook(|| Arc::new(Mutex::new(init())), |cell| Arc::clone(cell))
    }

    /// Clones resource `R`; the component re-renders when it changes.
    #[must_use]
    pub fn use_resource<R: Resource + Clone>(&self) -> Option<R> {
        let value = self.world.get_resource::<R>();
        self.track(
            self.world.component_id::<R>().map(ReadKey::Resource),
            TypeId::of::<R>(),
            value.is_some(),
        );
        value.cloned()
    }

    /// Clones component `C` of `entity`; the component re-renders when it
    /// changes.
    #[must_use]
    pub fn use_component<C: Component + Clone>(&self, entity: Entity) -> Option<C> {
        let value = self
            .world
            .get_entity(entity)
            .ok()
            .and_then(|entity| entity.get::<C>());
        self.track(
            self.world
                .component_id::<C>()
                .map(|id| ReadKey::Component(entity, id)),
            TypeId::of::<C>(),
            value.is_some(),
        );
        value.cloned()
    }

    fn track(&self, key: Option<ReadKey>, type_id: TypeId, present: bool) {
        let key = key.unwrap_or(ReadKey::Unregistered(type_id));
        self.reads.borrow_mut().push(Read { key, present });
    }

    /// Like a React context provider: makes `value` available to
    /// [`use_context`](Self::use_context) in descendant components.
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

    /// Runs an async task for `deps` and re-renders when it finishes, like
    /// React's `use` with Suspense.
    ///
    /// The first render with new `deps` starts `start(deps)` on Bevy's
    /// [`AsyncComputeTaskPool`] and returns [`TaskState::Pending`]. Once the
    /// future completes, the component re-renders and gets
    /// [`TaskState::Ready`] with a clone of the output until `deps` change.
    /// Changing `deps` drops the running task, which cancels it, and so does
    /// despawning the component.
    ///
    /// ```ignore
    /// let name = match cx.use_task(user_id, |id| async move { load_user(id).await }) {
    ///     TaskState::Pending => "Loading...".to_string(),
    ///     TaskState::Ready(user) => user.name,
    /// };
    /// ```
    pub fn use_task<D, T, F, Fut>(&self, deps: D, start: F) -> TaskState<T>
    where
        D: PartialEq + Send + Sync + 'static,
        T: Clone + Send + Sync + 'static,
        F: FnOnce(&D) -> Fut,
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
                    task: Mutex::new(Some(pool.spawn(start(&deps)))),
                    output: Mutex::new(None),
                });
                // Replacing the previous slot drops its task, cancelling it.
                *hook = Some((deps, Arc::clone(&slot)));
                slot
            },
        );
        let output = lock(&slot.output).clone();
        if output.is_none() {
            self.reads.borrow_mut().push(Read {
                key: ReadKey::Task(slot),
                present: true,
            });
        }
        output.map_or(TaskState::Pending, TaskState::Ready)
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

/// A [`Cx::use_task`] task and, once it finished, its output.
struct TaskSlot<T> {
    task: Mutex<Option<Task<T>>>,
    output: Mutex<Option<T>>,
}

/// A running task, polled by the render loop like any other read.
trait PollTask: Send + Sync {
    /// Moves the output in if the task finished; returns whether it did.
    fn poll_finished(&self) -> bool;
}

impl<T: Send + Sync + 'static> PollTask for TaskSlot<T> {
    fn poll_finished(&self) -> bool {
        let mut task = lock(&self.task);
        let Some(output) = task.as_mut().and_then(check_ready) else {
            return task.is_none();
        };
        *task = None;
        drop(task);
        *lock(&self.output) = Some(output);
        true
    }
}

/// Setter returned by [`Cx::use_state`]: `set_count(count + 1)`.
pub type SetState<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Dispatcher returned by [`Cx::use_reducer`]: `dispatch(Action::Add)`.
pub type Dispatch<A> = Arc<dyn Fn(A) + Send + Sync>;

// ---------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------

/// A component with its props, ready to be placed in a scene. Returned by
/// `#[component]` functions and [`component()`].
///
/// An unkeyed component is matched across its parent's renders by the call
/// site that created it (the `@name(..)` call) and how many components that
/// site created before it, so adding or removing a conditional sibling does
/// not move another component's state. Components created in a loop share a
/// call site; key them.
pub struct Element<P, F> {
    props: P,
    key: Option<u64>,
    site: &'static Location<'static>,
    render: F,
}

impl<P, F> Element<P, F> {
    /// Gives the component a key, like `<Row key={id} />`. Keyed components
    /// keep their state when siblings are inserted, removed or reordered.
    #[must_use]
    pub fn key(mut self, key: impl Hash) -> Self {
        self.key = Some(FixedState::default().hash_one(key));
        self
    }
}

/// Keys a component inside `bsn!`: `@keyed(todo.id, todo_row(todo))`.
pub fn keyed<P, F>(key: impl Hash, element: Element<P, F>) -> Element<P, F> {
    element.key(key)
}

/// Mounts a closure as a component: `component(|cx| bsn! { .. })`.
#[track_caller]
pub fn component<F, S>(render: F) -> Element<(), impl Render<()>>
where
    F: Fn(&Cx) -> S + Clone + Send + Sync + 'static,
    S: Scene,
{
    element((), move |cx: &Cx, ()| render(cx))
}

#[doc(hidden)]
#[track_caller]
pub fn element<P, F, S>(props: P, render: F) -> Element<P, F>
where
    P: EntityProps,
    F: Fn(&Cx, P::Output) -> S + Clone + Send + Sync + 'static,
    S: Scene,
{
    Element {
        props,
        key: None,
        site: Location::caller(),
        render,
    }
}

/// A component's render function, taking its resolved entity props.
#[doc(hidden)]
pub trait Render<P>: Clone + Send + Sync + 'static {
    type Output: Scene;

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

/// `Entity` props, which accept `#Name` references from the scene.
#[doc(hidden)]
pub trait EntityProps: Send + Sync + 'static {
    type Output: Copy + Send + Sync + 'static;

    fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output>;

    #[must_use]
    fn clone_props(&self) -> Self;
}

impl EntityProps for () {
    type Output = ();

    fn resolve(&self, _: &mut TemplateContext) -> Result<()> {
        Ok(())
    }

    fn clone_props(&self) -> Self {}
}

impl EntityProps for EntityTemplate {
    type Output = Entity;

    fn resolve(&self, context: &mut TemplateContext) -> Result<Entity> {
        self.build_template(context)
    }

    fn clone_props(&self) -> Self {
        self.clone_template()
    }
}

impl EntityProps for Entity {
    type Output = Entity;

    fn resolve(&self, _: &mut TemplateContext) -> Result<Entity> {
        Ok(*self)
    }

    fn clone_props(&self) -> Self {
        *self
    }
}

macro_rules! tuple_props {
    ($($name:ident),+) => {
        impl<$($name: EntityProps),+> EntityProps for ($($name,)+) {
            type Output = ($($name::Output,)+);

            #[allow(non_snake_case)]
            fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output> {
                let ($($name,)+) = self;
                Ok(($($name.resolve(context)?,)+))
            }

            #[allow(non_snake_case)]
            fn clone_props(&self) -> Self {
                let ($($name,)+) = self;
                ($($name.clone_props(),)+)
            }
        }
    };
}

tuple_props!(A);
tuple_props!(A, B);
tuple_props!(A, B, C);
tuple_props!(A, B, C, D);

impl<P, F> Template for Element<P, F>
where
    P: EntityProps,
    F: Render<P::Output>,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let installed = context
            .entity
            .world_scope(|world| world.contains_resource::<Hosts>());
        if !installed {
            return Err("components need `HooksPlugin`; add it to the app".into());
        }
        let props = self.props.resolve(context)?;
        let renderer = Renderer {
            render: render_loop(
                instance_for(self.key, self.site),
                self.render.clone(),
                props,
            ),
            reads: Vec::new(),
            last_run: None,
        };
        if let Some(mut renderers) = context.entity.get_mut::<Renderers>() {
            renderers.0.push(renderer);
        } else {
            context.entity.insert(Renderers(vec![renderer]));
        }
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            props: self.props.clone_props(),
            key: self.key,
            site: self.site,
            render: self.render.clone(),
        }
    }
}

impl<P, F> Scene for Element<P, F>
where
    P: EntityProps,
    F: Render<P::Output>,
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

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum ReadKey {
    Component(Entity, ComponentId),
    Resource(ComponentId),
    Unregistered(TypeId),
    State(Arc<Instance>, u64),
    /// A running [`Cx::use_task`] task; changed once it finished.
    Task(Arc<dyn PollTask>),
}

#[derive(Clone)]
struct Read {
    key: ReadKey,
    present: bool,
}

impl Read {
    fn changed(&self, world: &World, last_run: Tick, this_run: Tick) -> bool {
        let ticks = match &self.key {
            &ReadKey::Component(entity, id) => world
                .get_entity(entity)
                .ok()
                .and_then(|entity| entity.get_change_ticks_by_id(id)),
            &ReadKey::Resource(id) => world
                .resource_entities()
                .get(id)
                .and_then(|entity| world.get_entity(entity).ok())
                .and_then(|entity| entity.get_change_ticks_by_id(id)),
            ReadKey::State(instance, version) => return instance.version() != *version,
            ReadKey::Task(task) => return task.poll_finished(),
            &ReadKey::Unregistered(type_id) => {
                return world.components().get_id(type_id).is_some();
            }
        };
        ticks.is_some() != self.present
            || ticks.is_some_and(|ticks| ticks.is_changed(last_run, this_run))
    }
}

type RenderFn = Box<dyn FnMut(&mut World, Entity) -> Vec<Read> + Send + Sync>;

struct Renderer {
    render: RenderFn,
    reads: Vec<Read>,
    last_run: Option<Tick>,
}

/// Components mounted on an entity.
#[derive(Component)]
#[component(on_add = add_host, on_remove = remove_host)]
struct Renderers(Vec<Renderer>);

#[derive(Resource, Default)]
struct Hosts(EntityHashSet);

fn add_host(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<Hosts>() {
        hosts.0.insert(context.entity);
    }
}

fn remove_host(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<Hosts>() {
        hosts.0.remove(&context.entity);
    }
}

fn render_all(world: &mut World) {
    for _ in 0..MAX_ROUNDS {
        if render_dirty(world) == 0 {
            return;
        }
    }
    warn!(
        "hooks: components still re-rendering after {MAX_ROUNDS} rounds; the rest render next \
         frame. A render may be setting its own state."
    );
}

/// Re-renders every component whose reads changed. Returns how many ran.
fn render_dirty(world: &mut World) -> usize {
    let hosts: Vec<Entity> = world.resource::<Hosts>().0.iter().copied().collect();
    let mut rendered = 0;
    for host in hosts {
        let Ok(mut entity) = world.get_entity_mut(host) else {
            continue;
        };
        if entity.contains::<Disabled>() {
            continue;
        }
        let Some(mut renderers) = entity.get_mut::<Renderers>() else {
            continue;
        };
        let mut renderers = std::mem::take(&mut renderers.bypass_change_detection().0);
        for renderer in &mut renderers {
            let this_run = world.read_change_tick();
            let dirty = renderer.last_run.is_none_or(|last_run| {
                renderer
                    .reads
                    .iter()
                    .any(|read| read.changed(world, last_run, this_run))
            });
            if !dirty {
                continue;
            }
            // Writes made by this render get a tick no newer than its own
            // `last_run`, so a render never re-triggers itself.
            world.increment_change_tick();
            let tick = world.read_change_tick();
            renderer.reads = (renderer.render)(world, host);
            renderer.last_run = Some(tick);
            rendered += 1;
            if world.get_entity(host).is_err() {
                break;
            }
        }
        if let Ok(mut entity) = world.get_entity_mut(host)
            && let Some(mut current) = entity.get_mut::<Renderers>()
        {
            let current = current.bypass_change_detection();
            renderers.append(&mut current.0);
            current.0 = renderers;
        }
    }
    rendered
}

fn render_loop<P, F>(instance: Arc<Instance>, render: F, props: P) -> RenderFn
where
    P: Copy + Send + Sync + 'static,
    F: Render<P>,
{
    let mut child: Option<Entity> = None;
    let mut mounted = false;
    Box::new(move |world: &mut World, host| {
        if !mounted {
            mounted = true;
            make_layout_transparent(world, host);
        }
        let version = instance.version();
        let cx = Cx {
            world,
            entity: host,
            instance: &instance,
            hook: Cell::new(0),
            reads: RefCell::default(),
            effects: RefCell::default(),
        };
        let scene = render.render(&cx, props);
        let mut reads = cx.reads.into_inner();
        let effects = cx.effects.into_inner();
        reads.push(Read {
            key: ReadKey::State(Arc::clone(&instance), version),
            present: true,
        });

        if let Some(old) = child.take() {
            world.despawn(old);
        }
        {
            let mut slots = lock(&instance.children);
            slots.previous = std::mem::take(&mut slots.current);
            slots.sites.clear();
        }
        RENDERING.with(|stack| stack.borrow_mut().push(Arc::clone(&instance)));
        let guard = RenderingGuard;
        child = spawn_child(world, host, scene);
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
        reads
    })
}

fn spawn_child<S: Scene>(world: &mut World, parent: Entity, scene: S) -> Option<Entity> {
    match world.spawn_scene(scene) {
        Ok(mut child) => Some(child.insert(ChildOf(parent)).id()),
        Err(error) => {
            let handler = world.fallback_error_handler();
            handler(
                BevyError::from(error),
                bevy::ecs::error::ErrorContext::Command {
                    name: "bevy_radioactive::hooks::render".into(),
                },
            );
            None
        }
    }
}

/// Makes a host without a UI `Node` a `GhostNode`. `GhostNode` requires
/// `Transform`, which UI layout doesn't use and Bevy warns about under a UI
/// parent, so it is removed.
#[cfg(feature = "ui")]
fn make_layout_transparent(world: &mut World, host: Entity) {
    use bevy::ui::{Node, experimental::GhostNode};
    if let Ok(mut entity) = world.get_entity_mut(host)
        && !entity.contains::<Node>()
        && !entity.contains::<GhostNode>()
    {
        entity.insert(GhostNode);
        entity.remove::<(Transform, GlobalTransform)>();
    }
}

#[cfg(not(feature = "ui"))]
fn make_layout_transparent(_: &mut World, _: Entity) {}
