//! Push-based invalidation: which effect hosts may need to run.
//!
//! Each run of an effect subscribes a [`Link`] to everything it read, and a
//! write pushes the link's host onto a per-phase dirty queue. The runtime
//! then checks only queued hosts, with the exact per-dependency test, so a
//! frame where nothing an effect read changed costs close to nothing.
//!
//! - Signals and memos hold the links weakly and mark them on `set`
//!   ([`Subscriber`]). Memos propagate marks with Reactively colouring.
//! - Presence and component reads are marked by `Insert` and
//!   `Remove` observers, installed once per component type read.
//! - Values of mutable components and resources can change through `Mut<T>`
//!   or `ResMut<T>`, which have no hook. They are found by scanning the
//!   change ticks of the (entity, component) pairs that some effect read,
//!   once at the start of each pass and again after effects ran. Immutable
//!   components are never scanned.
//!
//! Links are dropped when their effect runs again or its host despawns,
//! which unsubscribes them; dead entries are pruned as the indexes grow.

use std::{
    any::TypeId,
    cmp::Reverse,
    collections::BinaryHeap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use bevy::{
    ecs::{
        change_detection::Tick,
        component::{ComponentId, ComponentInfo},
        entity::EntityHashMap,
        entity_disabling::Disabled,
        lifecycle::{InsertEvent, RemoveEvent},
    },
    platform::collections::{HashMap, HashSet, hash_map::Entry},
    prelude::*,
};

use super::{Dep, DepKey, Phase};
use crate::coat::Subscriber;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hosts marked for one phase.
#[derive(Default)]
struct Queue {
    hosts: Mutex<Vec<Entity>>,
    /// Set after a push, so collecting an empty queue takes no lock.
    pending: AtomicBool,
}

impl Queue {
    fn extend(&self, hosts: impl IntoIterator<Item = Entity>) {
        lock(&self.hosts).extend(hosts);
        self.pending.store(true, Ordering::Release);
    }
}

/// State shared by the runtime, signal subscriptions and change observers.
#[derive(Default)]
pub(crate) struct Shared {
    main: Queue,
    post_layout: Queue,
    /// Hosts that were marked while `Disabled`, waiting to be enabled.
    parked: Mutex<Vec<(Entity, Phase)>>,
    index: Mutex<Index>,
    /// Live links of post-layout effects; without any, post-layout passes
    /// skip the scan.
    post_links: AtomicUsize,
}

impl Shared {
    fn queue(&self, phase: Phase) -> &Queue {
        match phase {
            Phase::Main => &self.main,
            Phase::PostLayout => &self.post_layout,
        }
    }

    /// Queues `host` to have its `phase` effects checked.
    pub(crate) fn mark(&self, host: Entity, phase: Phase) {
        self.queue(phase).extend([host]);
    }

    /// Whether a scan can mark a post-layout effect.
    pub(crate) fn has_post_links(&self) -> bool {
        self.post_links.load(Ordering::Relaxed) > 0
    }

    pub(crate) fn park(&self, host: Entity, phase: Phase) {
        lock(&self.parked).push((host, phase));
    }
}

/// The runtime's [`Shared`] state.
#[derive(Resource, Clone, Default)]
pub(crate) struct Invalidation(pub(crate) Arc<Shared>);

/// One run of one effect, subscribed to everything that run read.
pub(crate) struct Link {
    host: Entity,
    phase: Phase,
    shared: Arc<Shared>,
}

impl Link {
    pub(crate) fn new(host: Entity, phase: Phase, shared: &Arc<Shared>) -> Arc<Self> {
        if phase == Phase::PostLayout {
            shared.post_links.fetch_add(1, Ordering::Relaxed);
        }
        Arc::new(Self {
            host,
            phase,
            shared: Arc::clone(shared),
        })
    }

    fn mark(&self) {
        self.shared.mark(self.host, self.phase);
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        if self.phase == Phase::PostLayout {
            self.shared.post_links.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Subscriber for Link {
    fn mark(&self, _changed: bool) {
        Link::mark(self);
    }
}

fn mark_all(links: &[Weak<Link>]) {
    for link in links.iter().filter_map(Weak::upgrade) {
        link.mark();
    }
}

fn alive(link: &Weak<Link>) -> bool {
    link.strong_count() > 0
}

/// Pushes `link`, pruning dead links first when the list is full.
fn push(links: &mut Vec<Weak<Link>>, link: Weak<Link>) {
    if links.len() == links.capacity() {
        links.retain(alive);
    }
    links.push(link);
}

#[derive(Default)]
struct Watchers {
    /// Runs that read the component's value.
    value: Vec<Weak<Link>>,
    /// Runs that read only whether the entity has the component.
    presence: Vec<Weak<Link>>,
    /// Listed in [`Index::scanned`].
    scanned: bool,
}

struct ResourceWatch {
    links: Vec<Weak<Link>>,
    present: bool,
}

/// Which runs read which ECS data.
#[derive(Default)]
struct Index {
    components: HashMap<(Entity, ComponentId), Watchers>,
    /// Pairs whose value a run read, of mutable components.
    scanned: Vec<(Entity, ComponentId)>,
    resources: HashMap<ComponentId, ResourceWatch>,
    unregistered: Vec<(TypeId, Weak<Link>)>,
    /// Component types with change observers.
    observed: HashSet<ComponentId>,
    last_scan: Tick,
    sweep_at: usize,
}

impl Index {
    fn component_changed(&self, entity: Entity, id: ComponentId) {
        if let Some(watchers) = self.components.get(&(entity, id)) {
            mark_all(&watchers.value);
            mark_all(&watchers.presence);
        }
    }

    /// Drops entries no live run reads, once the map has doubled.
    fn sweep(&mut self) {
        if self.components.len() < self.sweep_at {
            return;
        }
        self.components.retain(|_, watchers| {
            watchers.value.retain(alive);
            watchers.presence.retain(alive);
            watchers.scanned &= !watchers.value.is_empty();
            !watchers.value.is_empty() || !watchers.presence.is_empty()
        });
        let components = &self.components;
        self.scanned.retain(|pair| {
            components
                .get(pair)
                .is_some_and(|watchers| watchers.scanned)
        });
        self.sweep_at = (self.components.len() * 2).max(1024);
    }
}

fn mutable(world: &World, id: ComponentId) -> bool {
    world
        .components()
        .get_info(id)
        .is_none_or(ComponentInfo::mutable)
}

/// Subscribes `link` to everything in `deps`.
pub(crate) fn subscribe(world: &mut World, shared: &Arc<Shared>, link: &Arc<Link>, deps: &[Dep]) {
    let weak = Arc::downgrade(link);
    let mut observe = Vec::new();
    if deps.iter().all(|dep| matches!(dep.key, DepKey::Signal(_))) {
        for dep in deps {
            if let DepKey::Signal(signal) = &dep.key {
                signal.subscribe(weak.clone());
            }
        }
        return;
    }
    {
        let mut index = lock(&shared.index);
        let index = &mut *index;
        for dep in deps {
            match &dep.key {
                &DepKey::Component(entity, id) => {
                    let watchers = index.components.entry((entity, id)).or_default();
                    push(&mut watchers.value, weak.clone());
                    if !watchers.scanned && mutable(world, id) {
                        watchers.scanned = true;
                        index.scanned.push((entity, id));
                    }
                    if index.observed.insert(id) {
                        observe.push(id);
                    }
                }
                &DepKey::Presence(entity, id) => {
                    let watchers = index.components.entry((entity, id)).or_default();
                    push(&mut watchers.presence, weak.clone());
                    if index.observed.insert(id) {
                        observe.push(id);
                    }
                }
                &DepKey::Resource(id) => {
                    let watch = index.resources.entry(id).or_insert_with(|| ResourceWatch {
                        links: Vec::new(),
                        present: resource_ticks(world, id).is_some(),
                    });
                    push(&mut watch.links, weak.clone());
                }
                &DepKey::Unregistered(type_id) => index.unregistered.push((type_id, weak.clone())),
                DepKey::Signal(signal) => signal.subscribe(weak.clone()),
            }
        }
        index.sweep();
    }
    if !observe.is_empty() {
        for id in observe {
            let inserted = Arc::clone(shared);
            world.spawn(
                Observer::new(move |event: On<InsertEvent>| {
                    lock(&inserted.index).component_changed(event.entity, id);
                })
                .with_component(id),
            );
            let removed = Arc::clone(shared);
            world.spawn(
                Observer::new(move |event: On<RemoveEvent>| {
                    lock(&removed.index).component_changed(event.entity, id);
                })
                .with_component(id),
            );
        }
        // Observers register through commands.
        world.flush();
    }
}

pub(super) fn resource_ticks(
    world: &World,
    id: ComponentId,
) -> Option<bevy::ecs::change_detection::ComponentTicks> {
    world
        .resource_entities()
        .get(id)
        .and_then(|entity| world.get_entity(entity).ok())
        .and_then(|entity| entity.get_change_ticks_by_id(id))
}

/// Marks the runs whose scanned reads changed since the last scan: values
/// of mutable components and resources, and types registered since.
pub(crate) fn scan(world: &mut World, shared: &Shared) {
    let this_run = world.change_tick();
    {
        let mut index = lock(&shared.index);
        let index = &mut *index;
        let last = index.last_scan;
        let world: &World = world;
        let components = &mut index.components;
        index.scanned.retain(|pair| {
            let changed = world
                .get_entity(pair.0)
                .ok()
                .and_then(|entity| entity.get_change_ticks_by_id(pair.1))
                .is_some_and(|ticks| ticks.is_changed(last, this_run));
            if !changed {
                return true;
            }
            let Some(watchers) = components.get_mut(pair) else {
                return false;
            };
            watchers.value.retain(alive);
            mark_all(&watchers.value);
            watchers.scanned = !watchers.value.is_empty();
            watchers.scanned
        });
        index.resources.retain(|&id, watch| {
            watch.links.retain(alive);
            if watch.links.is_empty() {
                return false;
            }
            let ticks = resource_ticks(world, id);
            if ticks.is_some() != watch.present
                || ticks.is_some_and(|ticks| ticks.is_changed(last, this_run))
            {
                watch.present = ticks.is_some();
                mark_all(&watch.links);
            }
            true
        });
        index.unregistered.retain(|(type_id, link)| {
            let Some(link) = link.upgrade() else {
                return false;
            };
            if world.components().get_id(*type_id).is_some() {
                link.mark();
                return false;
            }
            true
        });
        index.last_scan = this_run;
    }
    // Writes from now on get a newer tick than `last_scan`.
    world.increment_change_tick();
}

/// Queues parked hosts that are no longer disabled.
pub(crate) fn unpark(world: &World, shared: &Shared) {
    let parked = std::mem::take(&mut *lock(&shared.parked));
    if parked.is_empty() {
        return;
    }
    let mut still = Vec::new();
    for (host, phase) in parked {
        match world.get_entity(host) {
            Ok(entity) if entity.contains::<Disabled>() => still.push((host, phase)),
            Ok(_) => shared.mark(host, phase),
            Err(_) => {}
        }
    }
    lock(&shared.parked).extend(still);
}

/// Hosts to check in one pass, parent-first.
pub(crate) struct Pass {
    phase: Phase,
    heap: BinaryHeap<Reverse<(usize, u64, Entity)>>,
    /// `false` while queued, `true` once checked in this pass.
    seen: EntityHashMap<bool>,
    /// Hosts marked again after their check, for the next pass.
    deferred: Vec<Entity>,
    order: u64,
}

impl Pass {
    pub(crate) fn new(phase: Phase) -> Self {
        Self {
            phase,
            heap: BinaryHeap::new(),
            seen: EntityHashMap::default(),
            deferred: Vec::new(),
            order: 0,
        }
    }

    /// Moves marked hosts into the pass. A host checked earlier in the pass
    /// waits for the next one, so a feedback loop takes one pass per round.
    pub(crate) fn collect(&mut self, world: &World, shared: &Shared) {
        let queue = shared.queue(self.phase);
        if !queue.pending.swap(false, Ordering::Acquire) {
            return;
        }
        let marked = std::mem::take(&mut *lock(&queue.hosts));
        for host in marked {
            match self.seen.entry(host) {
                Entry::Occupied(checked) => {
                    if *checked.get() {
                        self.deferred.push(host);
                    }
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(false);
                    self.heap
                        .push(Reverse((super::depth(world, host), self.order, host)));
                    self.order += 1;
                }
            }
        }
    }

    pub(crate) fn pop(&mut self) -> Option<Entity> {
        let Reverse((_, _, host)) = self.heap.pop()?;
        self.seen.insert(host, true);
        Some(host)
    }

    /// Puts hosts deferred to the next pass back on the queue.
    pub(crate) fn finish(self, shared: &Shared) {
        if !self.deferred.is_empty() {
            shared.queue(self.phase).extend(self.deferred);
        }
    }
}
