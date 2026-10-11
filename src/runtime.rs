//! Shared effect runtime used by `webby` and `coat`.

use std::{any::TypeId, cell::RefCell};

use bevy::{
    ecs::{
        change_detection::Tick,
        component::ComponentId,
        entity::EntityHashSet,
        entity_disabling::Disabled,
        lifecycle::HookContext,
        template::{EntityTemplate, Template, TemplateContext},
        world::DeferredWorld,
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

pub(crate) type RunFn = Box<dyn FnMut(&mut World, Entity) -> Vec<Dep> + Send + Sync>;

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
