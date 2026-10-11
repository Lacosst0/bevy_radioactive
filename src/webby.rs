//! Solid/React-flavoured UI reactivity for BSN scenes.
//!
//! `webby` effects are closures that *read* state through a [`Cx`] and
//! automatically re-run when anything they read changes — no app-time
//! registration, no handler boilerplate. State is ordinary ECS data: any
//! component on any entity, or any resource.
//!
//! ```ignore
//! use bevy_radioactive::webby::*;
//!
//! fn hud() -> impl Template<Output = EmptySceneEffect> {
//!     bsn! {
//!         Node
//!         Children [
//!             #Player Health { current: 100, max: 100 }
//!             --
//!             // like `{player.hp}` in JSX: re-renders when Health changes
//!             Text("")
//!             text(#Player, |cx, player| {
//!                 let hp = cx.get::<Health>(player).map_or(0, |hp| hp.current);
//!                 format!("HP: {hp}")
//!             })
//!             --
//!             // like Solid's <Show when={...}>
//!             Node
//!             when(#Player, |cx, player| cx.get::<Health>(player).is_none(), |_| bsn! {
//!                 Text("GAME OVER")
//!             })
//!         ]
//!     }
//! }
//! ```
//!
//! | JS frameworks                    | `webby`                                   |
//! | -------------------------------- | ----------------------------------------- |
//! | `createEffect` / `useEffect`     | [`effect`]                                |
//! | `createMemo` / JSX props         | [`prop`] (derived component, `PartialEq`) |
//! | `{value}` text interpolation     | [`text`]                                  |
//! | `style={{display}}`              | [`show`]                                  |
//! | `<Show when>`                    | [`when`]                                  |
//! | `<Switch>/<Match>`               | [`switch`]                                |
//! | `<For each>` / keyed `map`       | [`each`]                                  |
//! | `createSignal` / `useState`      | any component or resource                 |
//!
//! Every helper takes a dependency list first: `()`, a `#Name`, an
//! [`Entity`], an array like `[#A, #B]`, or a tuple. Entity references are
//! resolved once when the scene spawns and passed to the closure, because
//! `#Name` cannot be written inside closure bodies. Which *values* an effect
//! depends on is tracked automatically from what it reads through [`Cx`].
//!
//! Effects run once when spawned and again whenever a value they read was
//! inserted, mutated, or removed. They run inside [`ReactiveSet::Publish`],
//! before UI layout, and repeat until stable within a frame.
//!
//! [`ReactiveSet::Publish`]: crate::reactivity::ReactiveSet::Publish

use std::{any::TypeId, cell::RefCell, hash::Hash};

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
    platform::collections::HashMap,
    prelude::*,
    scene::{
        EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene, WorldSceneExt,
    },
};

use crate::reactivity::ReactiveSettings;

/// Tracking context passed to effects.
///
/// Every read through `Cx` becomes a dependency of the running effect.
pub struct Cx<'w> {
    world: &'w World,
    target: Entity,
    deps: RefCell<Vec<Dep>>,
}

impl<'w> Cx<'w> {
    fn new(world: &'w World, target: Entity) -> Self {
        Self {
            world,
            target,
            deps: RefCell::default(),
        }
    }

    /// Entity that owns the running effect.
    #[must_use]
    pub fn target(&self) -> Entity {
        self.target
    }

    /// Reads component `T` on `entity` and depends on it.
    ///
    /// Returns `None` when the entity or the component is missing; the
    /// effect re-runs when it appears.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&'w T> {
        let value = self
            .world
            .get_entity(entity)
            .ok()
            .and_then(|e| e.get::<T>());
        let key = match self.world.component_id::<T>() {
            Some(id) => DepKey::Component(entity, id),
            None => DepKey::Unregistered(TypeId::of::<T>()),
        };
        self.track(key, value.is_some());
        value
    }

    /// Reads component `T` on the effect's own entity and depends on it.
    pub fn this<T: Component>(&self) -> Option<&'w T> {
        self.get(self.target)
    }

    /// Reads resource `T` and depends on it.
    pub fn resource<T: Resource>(&self) -> Option<&'w T> {
        let value = self.world.get_resource::<T>();
        let key = match self.world.component_id::<T>() {
            Some(id) => DepKey::Resource(id),
            None => DepKey::Unregistered(TypeId::of::<T>()),
        };
        self.track(key, value.is_some());
        value
    }

    /// Untracked world access. Reads made through it do not re-run the effect.
    #[must_use]
    pub fn untracked(&self) -> &'w World {
        self.world
    }

    fn track(&self, key: DepKey, present: bool) {
        self.deps.borrow_mut().push(Dep { key, present });
    }
}

#[derive(Clone, Copy)]
enum DepKey {
    Component(Entity, ComponentId),
    Resource(ComponentId),
    Unregistered(TypeId),
}

#[derive(Clone, Copy)]
struct Dep {
    key: DepKey,
    present: bool,
}

impl Dep {
    fn is_dirty(&self, world: &World, last_run: Tick, this_run: Tick) -> bool {
        let ticks = match self.key {
            DepKey::Component(entity, id) => world
                .get_entity(entity)
                .ok()
                .and_then(|entity| entity.get_change_ticks_by_id(id)),
            DepKey::Resource(id) => world
                .resource_entities()
                .get(id)
                .and_then(|entity| world.get_entity(entity).ok())
                .and_then(|entity| entity.get_change_ticks_by_id(id)),
            DepKey::Unregistered(type_id) => {
                return world.components().get_id(type_id).is_some();
            }
        };
        ticks.is_some() != self.present
            || ticks.is_some_and(|ticks| ticks.is_changed(last_run, this_run))
    }
}

type RunFn = Box<dyn FnMut(&mut World, Entity) -> Vec<Dep> + Send + Sync>;

struct Slot {
    run: RunFn,
    deps: Vec<Dep>,
    last_run: Option<Tick>,
}

/// Effects owned by an entity. Inserted by `webby` helpers.
#[derive(Component, Default)]
#[component(on_add = register_host, on_remove = unregister_host)]
pub struct Effects(Vec<Slot>);

#[derive(Resource, Default)]
struct EffectHosts(EntityHashSet);

fn register_host(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<EffectHosts>() {
        hosts.0.insert(context.entity);
    }
}

fn unregister_host(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<EffectHosts>() {
        hosts.0.remove(&context.entity);
    }
}

/// Runs every dirty effect once. Returns the number of effects that ran.
pub(crate) fn run_effects(world: &mut World) -> usize {
    let Some(hosts) = world.get_resource::<EffectHosts>() else {
        return 0;
    };
    let hosts: Vec<Entity> = hosts.0.iter().copied().collect();
    let mut ran = 0;
    for host in hosts {
        let Ok(mut entity) = world.get_entity_mut(host) else {
            continue;
        };
        if entity.contains::<Disabled>() {
            continue;
        }
        let Some(mut effects) = entity.get_mut::<Effects>() else {
            continue;
        };
        let mut slots = std::mem::take(&mut effects.bypass_change_detection().0);
        for slot in &mut slots {
            let this_run = world.read_change_tick();
            let dirty = slot.last_run.is_none_or(|last_run| {
                slot.deps
                    .iter()
                    .any(|dep| dep.is_dirty(world, last_run, this_run))
            });
            if !dirty {
                continue;
            }
            // Writes made by this effect get a tick no newer than its own
            // `last_run`, so an effect never re-triggers itself, while later
            // effects still see those writes as changes.
            world.increment_change_tick();
            let tick = world.read_change_tick();
            slot.deps = (slot.run)(world, host);
            slot.last_run = Some(tick);
            ran += 1;
            if world.get_entity(host).is_err() {
                break;
            }
        }
        if let Ok(mut entity) = world.get_entity_mut(host)
            && let Some(mut effects) = entity.get_mut::<Effects>()
        {
            let effects = effects.bypass_change_detection();
            slots.append(&mut effects.0);
            effects.0 = slots;
        }
    }
    ran
}

pub(crate) fn init(world: &mut World) {
    world.init_resource::<EffectHosts>();
}

/// Entity dependencies resolved when a `webby` scene spawns.
pub trait Deps: Send + Sync + 'static {
    /// Resolved form passed to effect closures.
    type Output: Copy + Send + Sync + 'static;

    /// Resolves the dependencies.
    ///
    /// # Errors
    /// Returns an error when a reference cannot be resolved.
    fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output>;

    /// Clones the unresolved dependencies.
    #[must_use]
    fn clone_deps(&self) -> Self;
}

impl Deps for () {
    type Output = ();

    fn resolve(&self, _context: &mut TemplateContext) -> Result<()> {
        Ok(())
    }

    fn clone_deps(&self) -> Self {}
}

impl Deps for EntityTemplate {
    type Output = Entity;

    fn resolve(&self, context: &mut TemplateContext) -> Result<Entity> {
        self.build_template(context)
    }

    fn clone_deps(&self) -> Self {
        self.clone_template()
    }
}

impl Deps for Entity {
    type Output = Entity;

    fn resolve(&self, _context: &mut TemplateContext) -> Result<Entity> {
        Ok(*self)
    }

    fn clone_deps(&self) -> Self {
        *self
    }
}

impl<D: Deps, const N: usize> Deps for [D; N] {
    type Output = [D::Output; N];

    fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output> {
        let mut out = Vec::with_capacity(N);
        for dep in self {
            out.push(dep.resolve(context)?);
        }
        Ok(out
            .try_into()
            .unwrap_or_else(|_| unreachable!("array length is N")))
    }

    fn clone_deps(&self) -> Self {
        std::array::from_fn(|index| self[index].clone_deps())
    }
}

macro_rules! tuple_deps {
    ($($name:ident),+) => {
        impl<$($name: Deps),+> Deps for ($($name,)+) {
            type Output = ($($name::Output,)+);

            #[allow(non_snake_case)]
            fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output> {
                let ($($name,)+) = self;
                Ok(($($name.resolve(context)?,)+))
            }

            #[allow(non_snake_case)]
            fn clone_deps(&self) -> Self {
                let ($($name,)+) = self;
                ($($name.clone_deps(),)+)
            }
        }
    };
}

tuple_deps!(A);
tuple_deps!(A, B);
tuple_deps!(A, B, C);
tuple_deps!(A, B, C, D);

/// BSN template that attaches one effect to the current entity.
pub struct Effect<D, F> {
    deps: D,
    make: F,
}

impl<D, F> Template for Effect<D, F>
where
    D: Deps,
    F: Fn(D::Output) -> RunFn + Clone + Send + Sync + 'static,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        let installed = context
            .entity
            .world_scope(|world| world.contains_resource::<ReactiveSettings>());
        if !installed {
            return Err(
                "`webby` effects need the reactive runtime; add `RadioactivePlugin` to the app"
                    .into(),
            );
        }
        let deps = self.deps.resolve(context)?;
        let slot = Slot {
            run: (self.make)(deps),
            deps: Vec::new(),
            last_run: None,
        };
        if let Some(mut effects) = context.entity.get_mut::<Effects>() {
            effects.0.push(slot);
        } else {
            context.entity.insert(Effects(vec![slot]));
        }
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            deps: self.deps.clone_deps(),
            make: self.make.clone(),
        }
    }
}

impl<D, F> Scene for Effect<D, F>
where
    D: Deps,
    F: Fn(D::Output) -> RunFn + Clone + Send + Sync + 'static,
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

fn effect_template<D: Deps>(
    deps: D,
    make: impl Fn(D::Output) -> RunFn + Clone + Send + Sync + 'static,
) -> impl Template<Output = EmptySceneEffect> {
    Effect { deps, make }
}

fn tracked<R>(world: &World, target: Entity, f: impl FnOnce(&Cx) -> R) -> (R, Vec<Dep>) {
    let cx = Cx::new(world, target);
    let value = f(&cx);
    (value, cx.deps.into_inner())
}

/// Runs `f` now and again whenever anything it read through [`Cx`] changes.
///
/// Side effects go through `Commands`, applied right after the effect.
///
/// ```ignore
/// effect(#Player, |cx, player, commands| {
///     if cx.get::<Health>(player).is_some_and(|hp| hp.current == 0) {
///         commands.trigger(GameOver);
///     }
/// })
/// ```
pub fn effect<D, F>(deps: D, f: F) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    F: Fn(&Cx, D::Output, &mut Commands) + Clone + Send + Sync + 'static,
{
    effect_template(deps, move |deps| {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let mut queue = CommandQueue::default();
            let ((), tracked_deps) = tracked(world, target, |cx| {
                let mut commands = Commands::new(&mut queue, world);
                f(cx, deps, &mut commands);
            });
            queue.apply(world);
            tracked_deps
        })
    })
}

/// Derives component `C` on the current entity from tracked state.
///
/// `C` is inserted only when it differs from the current value, so other
/// effects can depend on it like a memo.
///
/// ```ignore
/// prop(#Player, |cx, player| {
///     let low = cx.get::<Health>(player).is_some_and(|hp| hp.current < 30);
///     BackgroundColor(if low { RED.into() } else { GREEN.into() })
/// })
/// ```
pub fn prop<D, C, F>(deps: D, f: F) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    C: Component + PartialEq,
    F: Fn(&Cx, D::Output) -> C + Clone + Send + Sync + 'static,
{
    effect_template(deps, move |deps| {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let (value, tracked_deps) = tracked(world, target, |cx| f(cx, deps));
            if let Ok(mut entity) = world.get_entity_mut(target)
                && entity.get::<C>() != Some(&value)
            {
                entity.insert(value);
            }
            tracked_deps
        })
    })
}

/// Sets the current entity's `Text` from tracked state.
///
/// ```ignore
/// Text("")
/// text((), |cx, ()| format!("Score: {}", cx.resource::<Score>().map_or(0, |s| s.0)))
/// ```
#[cfg(feature = "ui")]
pub fn text<D, S, F>(deps: D, f: F) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    S: Into<String>,
    F: Fn(&Cx, D::Output) -> S + Clone + Send + Sync + 'static,
{
    use bevy::ui::widget::Text;
    effect_template(deps, move |deps| {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let (value, tracked_deps) = tracked(world, target, |cx| f(cx, deps).into());
            if let Ok(mut entity) = world.get_entity_mut(target) {
                match entity.get_mut::<Text>() {
                    Some(mut text) => {
                        text.set_if_neq(Text(value));
                    }
                    None => {
                        entity.insert(Text(value));
                    }
                }
            }
            tracked_deps
        })
    })
}

/// Shows or hides the current entity's `Node` from tracked state.
///
/// Hiding sets `Display::None`; showing restores the previous display mode.
#[cfg(feature = "ui")]
pub fn show<D, F>(deps: D, f: F) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    F: Fn(&Cx, D::Output) -> bool + Clone + Send + Sync + 'static,
{
    use bevy::ui::{Display, Node};
    effect_template(deps, move |deps| {
        let f = f.clone();
        let mut shown_display = None;
        Box::new(move |world: &mut World, target| {
            let (visible, tracked_deps) = tracked(world, target, |cx| f(cx, deps));
            if let Ok(mut entity) = world.get_entity_mut(target)
                && let Some(mut node) = entity.get_mut::<Node>()
            {
                if visible {
                    if node.display == Display::None {
                        node.display = shown_display.take().unwrap_or_default();
                    }
                } else if node.display != Display::None {
                    shown_display = Some(node.display);
                    node.display = Display::None;
                }
            }
            tracked_deps
        })
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
                    name: "webby::spawn_child".into(),
                },
            );
            None
        }
    }
}

/// Renders the child scene for the current key, rebuilding it when the key
/// changes. Like Solid's `<Switch>`.
///
/// The current entity should have no other children.
///
/// ```ignore
/// Node
/// switch((), |cx, ()| cx.resource::<Screen>().copied(), |screen, ()| match screen {
///     Some(Screen::Menu) => Box::new(bsn! { @main_menu() }) as Box<dyn Scene>,
///     _ => Box::new(bsn! { @settings() }),
/// })
/// ```
pub fn switch<D, K, KF, S, R>(
    deps: D,
    key: KF,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    K: PartialEq + Send + Sync + 'static,
    KF: Fn(&Cx, D::Output) -> K + Clone + Send + Sync + 'static,
    S: Scene,
    R: Fn(&K, D::Output) -> S + Clone + Send + Sync + 'static,
{
    switch_optional(deps, key, move |key, deps| Some(render(key, deps)))
}

fn switch_optional<D, K, KF, S, R>(
    deps: D,
    key: KF,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    K: PartialEq + Send + Sync + 'static,
    KF: Fn(&Cx, D::Output) -> K + Clone + Send + Sync + 'static,
    S: Scene,
    R: Fn(&K, D::Output) -> Option<S> + Clone + Send + Sync + 'static,
{
    effect_template(deps, move |deps| {
        let (key, render) = (key.clone(), render.clone());
        let mut current: Option<(K, Option<Entity>)> = None;
        Box::new(move |world: &mut World, target| {
            let (next, tracked_deps) = tracked(world, target, |cx| key(cx, deps));
            if current.as_ref().is_none_or(|(key, _)| *key != next) {
                if let Some((_, Some(child))) = current.take() {
                    world.despawn(child);
                }
                let child = render(&next, deps).and_then(|scene| spawn_child(world, target, scene));
                current = Some((next, child));
            }
            tracked_deps
        })
    })
}

/// Renders the child scene while `cond` is true. Like Solid's `<Show>`.
///
/// The current entity should have no other children.
pub fn when<D, CF, S, R>(deps: D, cond: CF, render: R) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    CF: Fn(&Cx, D::Output) -> bool + Clone + Send + Sync + 'static,
    S: Scene,
    R: Fn(D::Output) -> S + Clone + Send + Sync + 'static,
{
    switch_optional(deps, cond, move |shown, deps| shown.then(|| render(deps)))
}

/// Renders one child per item, keyed by `key`. Like Solid's `<For>`.
///
/// Items whose key and value are unchanged keep their entity; changed items
/// are re-rendered, removed items are despawned, and children are reordered
/// to match. The current entity should have no other children.
///
/// ```ignore
/// Node
/// each(
///     (),
///     |cx, ()| cx.resource::<Inventory>().map(|inv| inv.items.clone()).unwrap_or_default(),
///     |item: &Item| item.id,
///     |item, ()| bsn! { Text({item.name.clone()}) },
/// )
/// ```
pub fn each<D, V, K, IF, KF, S, R>(
    deps: D,
    items: IF,
    key: KF,
    render: R,
) -> impl Template<Output = EmptySceneEffect>
where
    D: Deps,
    V: PartialEq + Send + Sync + 'static,
    K: Eq + Hash + Send + Sync + 'static,
    IF: Fn(&Cx, D::Output) -> Vec<V> + Clone + Send + Sync + 'static,
    KF: Fn(&V) -> K + Clone + Send + Sync + 'static,
    S: Scene,
    R: Fn(&V, D::Output) -> S + Clone + Send + Sync + 'static,
{
    effect_template(deps, move |deps| {
        let (items, key, render) = (items.clone(), key.clone(), render.clone());
        let mut rows: HashMap<K, (V, Option<Entity>)> = HashMap::default();
        let mut order: Vec<Entity> = Vec::new();
        Box::new(move |world: &mut World, target| {
            let (next, tracked_deps) = tracked(world, target, |cx| items(cx, deps));
            let mut next_rows = HashMap::default();
            let mut next_order = Vec::with_capacity(next.len());
            for item in next {
                let item_key = key(&item);
                let child = match rows.remove(&item_key) {
                    Some((old, child)) if old == item => child,
                    previous => {
                        if let Some((_, Some(child))) = previous {
                            world.despawn(child);
                        }
                        spawn_child(world, target, render(&item, deps))
                    }
                };
                next_order.extend(child);
                next_rows.insert(item_key, (item, child));
            }
            for (_, (_, child)) in rows.drain() {
                if let Some(child) = child {
                    world.despawn(child);
                }
            }
            rows = next_rows;
            if next_order != order
                && let Ok(mut entity) = world.get_entity_mut(target)
            {
                entity.replace_children(&next_order);
            }
            order = next_order;
            tracked_deps
        })
    })
}
