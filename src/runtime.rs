//! Shared effect runtime used by `webby` and `coat`.

use std::{
    any::TypeId,
    cell::RefCell,
    sync::{Arc, Mutex, PoisonError},
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
    prelude::*,
    scene::{
        EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene, WorldSceneExt,
    },
};

use crate::reactivity::ReactiveSettings;

/// Tracked read access to the world.
///
/// Every read through `Reads` becomes a dependency of the running effect or
/// component.
pub struct Reads<'w> {
    world: &'w World,
    target: Entity,
    deps: RefCell<Vec<Dep>>,
}

impl<'w> Reads<'w> {
    fn new(world: &'w World, target: Entity) -> Self {
        Self {
            world,
            target,
            deps: RefCell::default(),
        }
    }

    /// Entity that owns the running effect: the BSN entity that declared
    /// it, or a `webby` component's host.
    #[must_use]
    pub fn entity(&self) -> Entity {
        self.target
    }

    /// Old name of [`entity`](Self::entity).
    #[deprecated(since = "0.5.0", note = "renamed to `entity`")]
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

#[derive(Clone)]
enum DepKey {
    Component(Entity, ComponentId),
    Resource(ComponentId),
    Unregistered(TypeId),
    Signal(crate::coat::SignalDep),
}

#[derive(Clone)]
pub(crate) struct Dep {
    key: DepKey,
    present: bool,
}

impl Dep {
    fn is_dirty(&self, world: &World, last_run: Tick, this_run: Tick) -> bool {
        let ticks = match &self.key {
            &DepKey::Component(entity, id) => world
                .get_entity(entity)
                .ok()
                .and_then(|entity| entity.get_change_ticks_by_id(id)),
            &DepKey::Resource(id) => world
                .resource_entities()
                .get(id)
                .and_then(|entity| world.get_entity(entity).ok())
                .and_then(|entity| entity.get_change_ticks_by_id(id)),
            DepKey::Signal(signal) => return signal.is_dirty(),
            &DepKey::Unregistered(type_id) => {
                return world.components().get_id(type_id).is_some();
            }
        };
        ticks.is_some() != self.present
            || ticks.is_some_and(|ticks| ticks.is_changed(last_run, this_run))
    }
}

/// Work queued by an effect to undo what it did, run with `Commands` before
/// the effect runs again and when its entity is despawned.
pub type Cleanup = Box<dyn FnOnce(&mut Commands) + Send + Sync>;

/// Values an effect may return: `()` for no cleanup, or a closure taking
/// `&mut Commands` that undoes the effect.
///
/// ```ignore
/// effect(move |reads: &Reads, commands: &mut Commands| {
///     let tooltip = commands.spawn(Tooltip).id();
///     move |commands: &mut Commands| commands.entity(tooltip).despawn()
/// })
/// ```
pub trait IntoCleanup {
    /// Converts into an optional cleanup.
    fn into_cleanup(self) -> Option<Cleanup>;
}

impl IntoCleanup for () {
    fn into_cleanup(self) -> Option<Cleanup> {
        None
    }
}

impl<F: FnOnce(&mut Commands) + Send + Sync + 'static> IntoCleanup for F {
    fn into_cleanup(self) -> Option<Cleanup> {
        Some(Box::new(self))
    }
}

impl IntoCleanup for Option<Cleanup> {
    fn into_cleanup(self) -> Option<Cleanup> {
        self
    }
}

/// Result of one effect run: what it read, and how to undo it.
pub(crate) struct Ran {
    pub(crate) deps: Vec<Dep>,
    pub(crate) cleanup: Option<Cleanup>,
}

impl From<Vec<Dep>> for Ran {
    fn from(deps: Vec<Dep>) -> Self {
        Self {
            deps,
            cleanup: None,
        }
    }
}

pub(crate) type RunFn = Box<dyn FnMut(&mut World, Entity) -> Ran + Send + Sync>;

struct Slot {
    run: RunFn,
    deps: Vec<Dep>,
    last_run: Option<Tick>,
    cleanup: Option<Cleanup>,
}

/// Effects owned by an entity, inserted by `effect`, `computed`, `coat!`
/// regions and `webby` components.
#[derive(Component, Default)]
#[component(on_add = register_host, on_remove = unregister_host)]
pub struct Effects(Vec<Slot>);

/// Runs `cleanups` with commands applied to `world` right away.
fn run_cleanups(world: &mut World, cleanups: impl IntoIterator<Item = Cleanup>) -> usize {
    let mut queue = CommandQueue::default();
    let mut commands = Commands::new(&mut queue, world);
    let mut ran = 0;
    for cleanup in cleanups {
        cleanup(&mut commands);
        ran += 1;
    }
    queue.apply(world);
    ran
}

/// Effect work deferred to the end of a pass: `webby` effects, which run
/// after the components rendered in that pass, children first.
#[derive(Resource, Default)]
pub(crate) struct PostEffects(pub(crate) Vec<Box<dyn FnOnce(&mut World) + Send + Sync>>);

/// Cleanups of unmounted `webby` components, waiting for world access.
#[derive(Resource, Clone, Default)]
pub(crate) struct Unmounted(pub(crate) Arc<Mutex<Vec<Cleanup>>>);

fn run_unmounted(world: &mut World) -> usize {
    let Some(unmounted) = world.get_resource::<Unmounted>() else {
        return 0;
    };
    let cleanups = std::mem::take(&mut *unmounted.0.lock().unwrap_or_else(PoisonError::into_inner));
    if cleanups.is_empty() {
        0
    } else {
        run_cleanups(world, cleanups)
    }
}

/// Every effect host, plus the hosts added since the current pass started.
#[derive(Resource, Default)]
struct EffectHosts {
    all: EntityHashSet,
    fresh: Vec<Entity>,
}

fn register_host(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<EffectHosts>() {
        hosts.all.insert(context.entity);
        hosts.fresh.push(context.entity);
    }
}

fn unregister_host(mut world: DeferredWorld, context: HookContext) {
    if let Some(mut hosts) = world.get_resource_mut::<EffectHosts>() {
        hosts.all.remove(&context.entity);
    }
    let cleanups: Vec<Cleanup> = world
        .get_mut::<Effects>(context.entity)
        .map(|mut effects| {
            effects
                .bypass_change_detection()
                .0
                .iter_mut()
                .filter_map(|slot| slot.cleanup.take())
                .collect()
        })
        .unwrap_or_default();
    let mut commands = world.commands();
    for cleanup in cleanups {
        cleanup(&mut commands);
    }
}

/// Number of `ChildOf` ancestors of `entity`.
fn depth(world: &World, mut entity: Entity) -> usize {
    let mut depth = 0;
    while let Some(parent) = world.get::<ChildOf>(entity) {
        entity = parent.parent();
        depth += 1;
    }
    depth
}

/// Runs every dirty effect once. Returns the number of effects that ran.
///
/// Hosts run parent-first, so a parent that re-renders despawns stale
/// children before they run. Hosts spawned during the pass (child
/// components and nested regions) run in the same pass, so a pass settles a
/// whole tree rather than one level of it.
pub(crate) fn run_effects(world: &mut World) -> usize {
    let mut queue: Vec<Entity> = {
        let Some(mut hosts) = world.get_resource_mut::<EffectHosts>() else {
            return 0;
        };
        hosts.fresh.clear();
        hosts.all.iter().copied().collect()
    };
    queue.sort_by_cached_key(|&host| depth(world, host));
    let mut ran = run_unmounted(world);
    let mut next = 0;
    while next < queue.len() {
        let host = queue[next];
        next += 1;
        ran += run_host(world, host);
        let fresh = std::mem::take(&mut world.resource_mut::<EffectHosts>().fresh);
        queue.extend(fresh);
    }
    ran += run_unmounted(world);
    let post = world
        .get_resource_mut::<PostEffects>()
        .map(|mut post| std::mem::take(&mut post.0))
        .unwrap_or_default();
    for effect in post.into_iter().rev() {
        effect(world);
        ran += 1;
    }
    ran
}

fn run_host(world: &mut World, host: Entity) -> usize {
    let Ok(mut entity) = world.get_entity_mut(host) else {
        return 0;
    };
    if entity.contains::<Disabled>() {
        return 0;
    }
    let Some(mut effects) = entity.get_mut::<Effects>() else {
        return 0;
    };
    let mut slots = std::mem::take(&mut effects.bypass_change_detection().0);
    let mut ran = 0;
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
        if let Some(cleanup) = slot.cleanup.take() {
            run_cleanups(world, [cleanup]);
        }
        world.increment_change_tick();
        let tick = world.read_change_tick();
        let result = (slot.run)(world, host);
        slot.deps = result.deps;
        slot.cleanup = result.cleanup;
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
    } else {
        // The host was despawned by one of its own effects.
        run_cleanups(world, slots.into_iter().filter_map(|slot| slot.cleanup));
    }
    ran
}

pub(crate) fn init(world: &mut World) {
    world.init_resource::<EffectHosts>();
    world.init_resource::<PostEffects>();
    world.init_resource::<Unmounted>();
}

/// Entity dependencies resolved when a `webby` scene spawns.
pub trait Deps: Send + Sync + 'static {
    /// Resolved form passed to effect closures.
    type Output: Clone + Send + Sync + 'static;

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

impl<D: Deps> Deps for Option<D> {
    type Output = Option<D::Output>;

    fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output> {
        self.as_ref().map(|dep| dep.resolve(context)).transpose()
    }

    fn clone_deps(&self) -> Self {
        self.as_ref().map(Deps::clone_deps)
    }
}

impl<D: Deps> Deps for Vec<D> {
    type Output = Vec<D::Output>;

    fn resolve(&self, context: &mut TemplateContext) -> Result<Self::Output> {
        self.iter().map(|dep| dep.resolve(context)).collect()
    }

    fn clone_deps(&self) -> Self {
        self.iter().map(Deps::clone_deps).collect()
    }
}

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
        ensure_runtime(context)?;
        let deps = self.deps.resolve(context)?;
        attach(context, (self.make)(deps));
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

pub(crate) fn effect_template<D: Deps>(
    deps: D,
    make: impl Fn(D::Output) -> RunFn + Clone + Send + Sync + 'static,
) -> impl Template<Output = EmptySceneEffect> + Scene {
    Effect { deps, make }
}

pub(crate) fn tracked<R>(
    world: &World,
    target: Entity,
    f: impl FnOnce(&Reads) -> R,
) -> (R, Vec<Dep>) {
    let reads = Reads::new(world, target);
    let (value, signals) = crate::coat::scope(|| f(&reads));
    let mut deps = reads.deps.into_inner();
    deps.extend(signals.into_iter().map(|signal| Dep {
        key: DepKey::Signal(signal),
        present: true,
    }));
    (value, deps)
}

/// Removes the dependency on `C` of `target`, so an effect that writes `C`
/// does not depend on its own output.
pub(crate) fn forget<C: Component>(world: &World, deps: &mut Vec<Dep>, target: Entity) {
    let id = world.component_id::<C>();
    deps.retain(|dep| match dep.key {
        DepKey::Component(entity, other) => entity != target || Some(other) != id,
        DepKey::Unregistered(type_id) => type_id != TypeId::of::<C>(),
        _ => true,
    });
}

pub(crate) fn spawn_child<S: Scene>(world: &mut World, parent: Entity, scene: S) -> Option<Entity> {
    match world.spawn_scene(scene) {
        Ok(mut child) => Some(child.insert(ChildOf(parent)).id()),
        Err(error) => {
            let handler = world.fallback_error_handler();
            handler(
                BevyError::from(error),
                bevy::ecs::error::ErrorContext::Command {
                    name: "bevy_radioactive::spawn_child".into(),
                },
            );
            None
        }
    }
}

/// Converts signal reads collected outside [`tracked`] into dependencies.
pub(crate) fn signal_deps(signals: Vec<crate::coat::SignalDep>) -> Vec<Dep> {
    signals
        .into_iter()
        .map(|signal| Dep {
            key: DepKey::Signal(signal),
            present: true,
        })
        .collect()
}

/// Fails unless [`RadioactivePlugin`](crate::RadioactivePlugin) is installed.
pub(crate) fn ensure_runtime(context: &mut TemplateContext) -> Result {
    let installed = context
        .entity
        .world_scope(|world| world.contains_resource::<ReactiveSettings>());
    if installed {
        Ok(())
    } else {
        Err("reactive effects need the reactive runtime; add `RadioactivePlugin` to the app".into())
    }
}

/// Adds an effect to the entity being built.
pub(crate) fn attach(context: &mut TemplateContext, run: RunFn) {
    let slot = Slot {
        run,
        deps: Vec::new(),
        last_run: None,
        cleanup: None,
    };
    if let Some(mut effects) = context.entity.get_mut::<Effects>() {
        effects.0.push(slot);
    } else {
        context.entity.insert(Effects(vec![slot]));
    }
}

/// `GhostNode` requires `Transform`, but UI nodes never have a
/// `GlobalTransform`, so Bevy warns (B0004) about a ghost node under a UI
/// parent. UI layout does not use transforms; remove them from hosts.
#[cfg(feature = "ui")]
pub(crate) fn strip_ghost_transform(world: &mut World, entity: Entity) {
    if let Ok(mut entity) = world.get_entity_mut(entity) {
        entity.remove::<(Transform, GlobalTransform)>();
    }
}

#[cfg(feature = "ui")]
pub(crate) fn strip_ghost_transform_hook(mut world: DeferredWorld, context: HookContext) {
    world
        .commands()
        .entity(context.entity)
        .remove::<(Transform, GlobalTransform)>();
}
