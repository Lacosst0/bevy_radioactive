# bevy_radioactive codebase audit (branch `reactive-api-improvements`, v0.4.0, Bevy 0.20)

Source links are repo-relative to `C:\Code\bevy_radioactive` (format `file:line`). Line numbers are from the working tree at commit `e4e66a4`. "Probe" findings come from throwaway tests I ran against a scratch copy of the crate. They are reproducible, and no repo files were changed (the probe file lived at `%TEMP%\...\scratchpad\copy\tests\probe.rs`). Bevy behaviour was checked against `~/.cargo/registry/src/index.crates.io-*/bevy_ecs-0.20.0`.

## Q1. Core concepts and how they fit together (public surface)

### Takeaway
The crate has four layers that share one runtime. (1) `reactivity`: ECS change publication (`ValueChanged<T>`/`ValueRemoved<T>`) plus BSN helpers `watch`/`bind`/`bind_or`/`on_removed`. (2) `subscription`: source-preserving routing of any `EntityEvent` through `SubscribedTo<E>` relationship entities. (3) `coat`: out-of-ECS `Signal`/`Memo` plus the `coat!` macro that compiles reactive `if`/`for`/`match` into "region" effects. (4) `webby`: React-style `#[component]` functions with hooks, which re-render by despawning and respawning their subtree. Layers 3 and 4 both run as polled "effects" inside one exclusive system (`run_reactive`) in `PostUpdate` / `ReactiveSet::Publish`.

### Cited Findings
**Crate root and prelude**
- `lib.rs` re-exports only the reactivity and subscription items at the root. `coat` and `webby` are reached only through their modules, and `pub mod macros` is `#[doc(hidden)]`. — [src/lib.rs:7-29](src/lib.rs)
- `prelude` exports: `RadioactivePlugin, Reaction, ReactiveAppExt, Removal, Singleton, Subscription, SubscriptionAppExt, ValueChanged, ValueRemoved, bind, bind_or, on_removed, subscribe_to, watch`. It leaves out `ReactiveSet`, `ReactiveSettings`, `Signal`, `signal`, `coat!`, `component` and `Cx`. — [src/prelude.rs:3-6](src/prelude.rs)

**`reactivity` module**
- `enum ReactiveSet { Publish }` and `struct ReactivePublish` (a ScheduleLabel). `ReactiveSettings { pub max_passes: usize }` defaults to 8. — [src/reactivity.rs:44-70](src/reactivity.rs)
- `RadioactivePlugin` sets up the resources and the `ReactivePublish` schedule, adds `run_reactive` to `PostUpdate`, and with the `ui` feature orders the set before `UiSystems::Prepare`. — [src/reactivity.rs:76-90](src/reactivity.rs)
- `register::<T>` is idempotent through a `TypeId` set. It pushes `publish::<T>` into a pending list and installs the subscription forwarder for `ValueChanged<T>`. It calls `register_routed` for `ValueRemoved<T>` and adds a global `On<Remove<T>>` observer. — [src/reactivity.rs:102-113](src/reactivity.rs)
- `run_reactive` loops up to `max_passes` times. Each pass adds pending publishers, runs `ReactivePublish`, flushes, then runs `runtime::run_effects`. It stops when nothing was published and no effect ran. — [src/reactivity.rs:130-146](src/reactivity.rs)
- Event payloads:
  - `ValueChanged<T>` holds `{source, value: Arc<T>}` and offers `source()`, `value()`, `snapshot() -> Arc<T>` and `Deref<Target=T>`.
  - `ValueRemoved<T>` holds `{source}`.
  - Both are `EntityEvent`s.
  — [src/reactivity.rs:154-241](src/reactivity.rs)
- Type aliases: `Reaction<'w,'t,T> = Subscription<ValueChanged<T>>` and `Removal<'w,'t,T> = Subscription<ValueRemoved<T>>`. `Watch`/`WatchRemoval` are aliases of `Subscribe<...>`. — [src/reactivity.rs:32-35](src/reactivity.rs), [src/reactivity.rs:248-251](src/reactivity.rs)
- `trait ReactiveAppExt { fn track<T: Component+Clone>(&mut self)->&mut Self; fn observe_value<T,M>(&mut self, observer: impl IntoObserverSystem<ValueChanged<T>,M>)->&mut Self; }`. `track` adds the plugin automatically. — [src/reactivity.rs:256-291](src/reactivity.rs)
- `watch` and `on_removed` take `(source: S, observer: I)`, wrap `Subscribe::new`, and attach `with_check(ensure_tracked::<T>)`. — [src/reactivity.rs:390-407](src/reactivity.rs)
- `bind` and `bind_or`, with their bounds:
  - `bind<T,C,S,F>(source, write: F) -> impl Template<Output=EmptySceneEffect>` where `F: Fn(&T,&mut C)`.
  - `bind_or(source, write, removed: Fn(&mut C))` combines the two templates through a private `Both`.
  — [src/reactivity.rs:418-490](src/reactivity.rs)
- `Singleton` is a unit source that resolves resource `T`'s entity. It errors if the resource does not exist when the scene resolves. — [src/reactivity.rs:339-382](src/reactivity.rs)

**`subscription` module**
- `SubscriptionEvent<E>` holds `{delivery, target, source, event: Arc<E>}` and offers `new(target, event)`, `target()`, `source()`, `event()` and `Deref<Target=E>`. — [src/subscription.rs:27-81](src/subscription.rs)
- `Subscription<'w,'t,E>` is a `SystemInput` wrapper over `On<SubscriptionEvent<E>>` that adds `observer()`. — [src/subscription.rs:86-121](src/subscription.rs)
- The relationship types:
  - `SubscribedTo<E>` is the relationship. Its `on_add` hook asserts registration.
  - `Subscribers<E>` is the target. It is **not** `linked_spawn`.
  - `SubscriptionOf(pub Entity)` / `Subscriptions` link a subscription entity to its listener, with `linked_spawn`.
  — [src/subscription.rs:127-185](src/subscription.rs)
- `trait Source<E> { fn resolve(&self, &mut TemplateContext, &mut Vec<Entity>) -> Result; fn clone_source(&self) -> Self; }` is implemented for `EntityTemplate`, `Entity`, `[S; N]` and `Vec<S>`. — [src/subscription.rs:192-248](src/subscription.rs)
- `Subscribe<E,S,I,M>` (template and scene) spawns one entity per resolved source: `(SubscriptionOf(listener), SubscribedTo::<E>::new(source))`, plus `.observe(observer.clone())`. — [src/subscription.rs:252-328](src/subscription.rs)
- `subscribe_to(source, observer)` builds a `Subscribe`. `SubscriptionAppExt::register_subscription::<E>()` exists for direct use of the relationship. — [src/subscription.rs:336-358](src/subscription.rs)
- `route` reads `Subscribers<E>` on the source. It then queues one `SubscriptionEvent` per subscriber through `commands.trigger`, and resolves the target from `SubscriptionOf`. — [src/subscription.rs:407-430](src/subscription.rs)

**`runtime` module (crate-private, partly re-exported through `webby`)**
- `Reads<'w>` provides `target()`, `get::<T>(entity)`, `this::<T>()`, `resource::<T>()` and `untracked() -> &World`. It records `DepKey::{Component, Resource, Unregistered, Signal}`. — [src/runtime.rs:27-99](src/runtime.rs)
- Effect storage and polling:
  - `Effects(Vec<Slot>)` is a public component. Its hooks keep an `EffectHosts(EntityHashSet)` resource up to date.
  - `run_effects` polls every host's deps on every pass.
  — [src/runtime.rs:138-206](src/runtime.rs)
- `trait Deps { type Output: Copy; fn resolve(..); fn clone_deps(..) }` is implemented for `()`, `EntityTemplate`, `Entity`, `[D; N]` and tuples of up to 4. — [src/runtime.rs:213-304](src/runtime.rs)

**`coat` module**
- `Signal<T>(Arc<{RwLock<T>, AtomicU64 version}>)` offers `new`, `with`, `with_untracked`, `get`, `get_untracked`, `set`, `set_if_neq -> bool`, `update`, `notify`, `ptr_eq`, plus `increment`/`decrement` through the `Step` trait. `signal(init)` is the free-function constructor. — [src/coat.rs:194-312](src/coat.rs)
- `Memo<T>` is lazy and equality-gated, with `with`, `with_untracked`, `get` and `get_untracked`. Its constructor is `memo(compute)`. — [src/coat.rs:337-438](src/coat.rs)
- Other public items:
  - `cloned!` macro. — [src/coat.rs:447-455](src/coat.rs)
  - `effect(Fn(&mut Commands))`. — [src/coat.rs:463-479](src/coat.rs)
  - `shard(args, render: Fn(&A,&World)->S)`. — [src/coat.rs:488-511](src/coat.rs)
  - `Region` component (a GhostNode under `ui`). — [src/coat.rs:517-520](src/coat.rs)
- Hidden helpers used by the macro: `patch`, `region`, `for_keyed`, `for_indexed` and `__private`. — [src/coat.rs:551-671](src/coat.rs)
- `coat!` and `coat_list!` are token-level proc macros. They rewrite `$name` into `name.get()`. Reactive control flow becomes `__CoatRegion __coat_region(...)` / `__coat_for(...)`, and `Comp($x)` becomes `Comp(init) __coat_patch(...)`. — [macros/src/lib.rs:23-57](macros/src/lib.rs), [macros/src/lib.rs:313-364](macros/src/lib.rs), [macros/src/lib.rs:493-730](macros/src/lib.rs)

**`webby` module**
- `Cx<'a,'w>` derefs to `Reads`. Its hooks:
  - `entity()`, `use_state(init) -> (T, SetState<T>)`, `use_reducer(init, reducer) -> (T, Dispatch<A>)`, `use_signal(init) -> Signal<T>`.
  - `use_memo(deps, compute)`, `use_effect(deps, FnOnce(&D, &mut Commands))`, `use_ref(init) -> Ref<T>`.
  - `use_resource::<R>() -> Option<R>`, `use_component::<C>(entity) -> Option<C>`, `provide_context(value)`, `use_context::<T>() -> Option<T>`.
  — [src/webby.rs:209-410](src/webby.rs)
- `SetState<T>` and `Dispatch<A>` are both `Arc<dyn Fn(_) + Send + Sync>`. `Ref<T>(Arc<Mutex<T>>)` offers `lock`, `get` and `set`. — [src/webby.rs:414-447](src/webby.rs)
- Mounting:
  - `Mount<D,F>` (template and scene) supports `.key(impl Hash)`. `keyed(key, mount)` is the free-function form.
  - `component(Fn(&Cx)->S)`, `component_with(deps, Fn(&Cx, D::Output)->S)`, and the `Render<P>` trait.
  — [src/webby.rs:459-632](src/webby.rs)
- `#[component]` rewrites `fn name(cx: &Cx, props..) -> impl Scene` into `fn name(props..) -> Mount<impl Deps, impl Render>`. `Entity`-typed props become `impl Deps<Output=Entity>`, so `#Name` works. Other props are cloned on every render. — [macros/src/lib.rs:748-875](macros/src/lib.rs)

### Inferences
- How the pieces fit:
  - `watch`/`bind` are push-based. A per-type `Changed<T>` publisher system triggers events, and those events are routed through relationship entities to per-listener observers.
  - `coat` and `webby` are pull-based. `run_effects` re-checks every recorded dependency (ECS change ticks and signal versions) on every pass.
  - The two models meet only in `run_reactive`'s loop and in `ReactiveSettings::max_passes`.
- Bridging the models works one way only. `webby` can read ECS state reactively (`Cx::get/resource`). `coat!`'s `$` works only on signals and memos, and `watch`/`bind` cannot feed a signal without user code.

### Gaps
- I did not check `cargo doc` output for broken intra-doc links.

## Q2. API inconsistencies, verbosity and surprises

### Takeaway
There are three overlapping ways to express reactive UI (`bind`, `coat!`, `webby`), and several duplicate entry points inside each layer. Naming and argument order drift from the React and Bevy conventions the docs claim to follow. The most visible verbosity is manual signal and setter cloning, plus the lack of initial-value replay for `bind`.

### Cited Findings
**Duplicate entry points**
- `webby` offers three ways to mount a component: `#[component]`, `component(|cx| ..)` and `component_with(deps, |cx, e| ..)`. It offers two ways to key one: `.key(k)` and `keyed(k, mount)`. — [src/webby.rs:469-479](src/webby.rs), [src/webby.rs:588-609](src/webby.rs)
- `Cx::use_component::<C>(e)` is just `cx.get::<C>(e).cloned()`, and `Cx::use_resource::<R>()` is `cx.resource::<R>().cloned()`. Both pairs are reachable on `Cx` through `Deref<Target=Reads>`. — [src/webby.rs:369-378](src/webby.rs), [src/webby.rs:216-222](src/webby.rs)
- The same entity has three names: `Reads::target()`, `Cx::entity()` (which returns `reads.target()`), and `Reads::this::<T>()` (which reads on the target). Meanwhile `Subscription::target()` means *listener* in the subscription layer. — [src/runtime.rs:44-69](src/runtime.rs), [src/webby.rs:227-229](src/webby.rs), [src/subscription.rs:53-55](src/subscription.rs)
- There are two signal constructors: `signal(|| v)` and `Signal::new(v)`. `webby` re-exports `Signal` but not `signal`/`memo`, and `coat` exports `signal`/`memo` but not `Cx`. — [src/coat.rs:200-212](src/coat.rs), [src/webby.rs:116-120](src/webby.rs)

**Effect and tracking API drift**
- The three effect APIs have different shapes:
  - `coat::effect(Fn(&mut Commands))` gets no tracked `Reads`. The `|_|` in `tracked(world, target, |_| ..)` throws it away, so a coat effect can depend only on signals.
  - `webby::use_effect(deps, FnOnce(&D,&mut Commands))` is keyed on deps.
  - `shard(args, Fn(&A,&World))` makes world reads explicitly untracked.
  — [src/coat.rs:463-479](src/coat.rs), [src/coat.rs:488-511](src/coat.rs), [src/webby.rs:331-353](src/webby.rs)
- `coat`'s `patch` collects signal deps only (`scope(..)` → `signal_deps`), not ECS reads. `region` and `for_keyed` use `tracked` and would record ECS reads, but the macro gives users no handle to make them. — [src/coat.rs:553-569](src/coat.rs), [src/coat.rs:583](src/coat.rs)
- Plugin requirements differ by entry point. `track()` adds `RadioactivePlugin` automatically, but `watch`/`bind`/`webby`/`coat` fail at scene resolve with "add `RadioactivePlugin`" if it is missing. — [src/reactivity.rs:277-283](src/reactivity.rs), [src/reactivity.rs:294-306](src/reactivity.rs), [src/runtime.rs:398-407](src/runtime.rs)

**Naming and argument order**
- `use_reducer(init, reducer)` reverses React's `useReducer(reducer, initialArg)`, even though the docs present it as React parity. — [src/webby.rs:280-292](src/webby.rs), [src/webby.rs:59-60](src/webby.rs)
- The `watch` / `on_removed` / `bind_or` names are inconsistent: one verb-noun, one `on_*` event style, one `_or` suffix whose meaning is "on removal". — [src/reactivity.rs:390-460](src/reactivity.rs)

**Inconsistent semantics and boilerplate**
- `coat!` regions behave differently for `if` and `match`:
  - A reactive `if` is keyed on the *branch index*, so it re-renders only when the branch changes.
  - A reactive `match` is keyed on the *scrutinee value*, so `match $state { Loading(p) => .. }` rebuilds the arm on every value change, even when the arm stays the same.
  — [macros/src/lib.rs:547-588](macros/src/lib.rs), [macros/src/lib.rs:720-729](macros/src/lib.rs); coat docs claim "re-renders only when the branch changes" at [src/coat.rs:24-25](src/coat.rs)
- `$x` always expands to `x.get()`, which clones. `$todos.len()` clones the whole `Vec` just to read its length. A keyed `for` clones the entire list (`get()`) and then collects it into a new `Vec` on every run. — [macros/src/lib.rs:196-224](macros/src/lib.rs), [macros/src/lib.rs:618-620](macros/src/lib.rs)
- The `#[component]` macro has leftover and heuristic code:
  - It contains dead code: `let _ = uses_cx;`.
  - It detects `Entity` props by the *last path segment* being `Entity`, so `Option<Entity>`, `Vec<Entity>` and type aliases do not accept `#Name`.
  — [macros/src/lib.rs:810-821](macros/src/lib.rs)
- The `coat!` macro relies on heuristics:
  - It treats any identifier named like a `$`-read signal *anywhere* in the invocation as a signal, using thread-local state.
  - Pattern bindings are "every lowercase identifier".
  - Both are approximate and can mis-detect, for example a field named `count` when `$count` exists.
  — [macros/src/lib.rs:161-193](macros/src/lib.rs), [macros/src/lib.rs:181-193](macros/src/lib.rs), [macros/src/lib.rs:881-917](macros/src/lib.rs)
- The `ui` feature is required for `GhostNode` transparency. Without it, `Region` and component hosts are plain entities that do affect layout and transform propagation. — [src/coat.rs:517-520](src/coat.rs), [src/webby.rs:570-583](src/webby.rs)

**Documentation errors**
- The README says "Removal is delivered synchronously, while the source still exists". In fact routing reads `Subscribers` synchronously, but delivery is queued through `commands.trigger` and runs at the next flush, after despawn has finished. The tests only pass because they call `flush()`. — [README.md:275](README.md), [src/subscription.rs:423-428](src/subscription.rs), [tests/reactivity.rs:470-472](tests/reactivity.rs)
- The README `on_removed` example writes to *every* `Text` in the world (`for mut text in &mut q`) instead of `q.get_mut(r.target())`. — [README.md:129-133](README.md)
- MIGRATION.md still carries the whole 0.1→0.2 guide, which:
  - references removed APIs (`track_resource`, `watch_resource`, `observe_resource`) and Bevy 0.19;
  - says "Each listener supports one source per event type", which contradicts 0.4.
  — [MIGRATION.md:33-337](MIGRATION.md), [MIGRATION.md:256-257](MIGRATION.md)

### Inferences
- Unifying the effect signature, for example on `Fn(&Reads, &mut Commands)`, would remove the biggest conceptual split between `coat` and `webby`.
- `use_component` and `use_resource` add little value. They could be kept as React-flavoured aliases, but the docs should name a single canonical spelling.

### Gaps
- No user feedback or issue tracker was available to confirm which overlaps users actually find confusing.

## Q3. Correctness and performance risks

### Takeaway
Probe tests confirmed five real bugs:
1. A memory leak of webby `Instance` trees, caused by an `Arc` parent/child cycle.
2. Leaked and orphaned rows in `coat` keyed `for` when keys repeat.
3. A per-frame nesting-depth limit: each component or region level costs one pass, and the default is 8.
4. A panic when a conditional sibling component shifts positional hook slots.
5. Subscription entities that pile up after their sources despawn.

Two more were confirmed: double delivery for raw `subscribe_to::<ValueRemoved<T>>`, and silent truncation at `max_passes` with no diagnostic. On performance, effects are polled with a full scan of all hosts every pass, every frame. Values are cloned and allocated per publication even when nobody listens. Webby re-renders respawn whole subtrees.

### Cited Findings
**Confirmed bugs**
- **Instance `Arc` cycle leak (confirmed).**
  - `Instance { parent: Option<Arc<Instance>>, children: Mutex<ChildSlots{current: HashMap<_, Arc<Instance>>}> }`. `child()` stores `Arc::clone(self)` as the child's parent while the parent stores the child in `current`. — [src/webby.rs:146-179](src/webby.rs)
  - Probe: despawning a `@parent` with a `@leaf` child dropped **0 of 2** `use_ref` values (`drops after despawn: 0`). Hook state, signals and contexts of any unmounted subtree two or more levels deep are never freed. — probe `probe_instance_tree_freed_on_despawn`
- **Duplicate keys in a keyed `for` (confirmed).**
  - `next_rows.insert(item_key, (item, children))` ignores the displaced value. The first row's entities stay as children and are never despawned. On the next run they are not in `rows`, and `replace_children` detaches them into orphaned roots. — [src/coat.rs:620-644](src/coat.rs)
  - Probe: `labels before=2 after clearing=1`. — probe `probe_duplicate_keys_leak_rows`
- **Nesting depth limited per frame (confirmed).**
  - `run_effects` snapshots `EffectHosts` into a `Vec` at the start of a pass. Hosts spawned during a render, such as child components and nested regions, run only in the *next* pass. — [src/runtime.rs:158-163](src/runtime.rs)
  - With `max_passes = 8` ([src/reactivity.rs:66-70](src/reactivity.rs)), a 13-level component chain showed its leaf only on frame 2 (`leaf present after 1 frame: 0, after 2 frames: 1`). The partial UI is visible for a frame. — probe `probe_nesting_depth_per_frame`
- **Positional hook-slot shift panics (confirmed).**
  - Child instances are keyed by a single position counter per parent *component*, counted across that component's whole rendered subtree. — [src/webby.rs:165-179](src/webby.rs)
  - Inserting a conditional unkeyed component before a sibling hands the sibling's instance to the new component, and `hook()` panics: "hook #0 changed type to `Signal<bool>`". — [src/webby.rs:244-249](src/webby.rs); probe `probe_positional_hook_state_shift`
  - The panic happens inside `run_reactive`, an exclusive system, so it takes down the app. When the hook types happen to match, state is silently swapped between unrelated components instead.
- **Orphaned subscription entities (confirmed).**
  - `Subscribers<E>` is not `linked_spawn` ([src/subscription.rs:155-157](src/subscription.rs)). Bevy's non-linked `RelationshipTarget::on_discard` only removes the `Relationship` component from the source-side entities. — [bevy_ecs-0.20.0/src/relationship/mod.rs:307-327](https://docs.rs/bevy_ecs/0.20.0/src/bevy_ecs/relationship/mod.rs.html)
  - The subscription entities, and their observer entities, therefore survive until the *listener* despawns. Probe: 5 of 5 remained after all 5 sources despawned. Listeners that watch dynamic or short-lived sources (for example `watch(vec_of_entities, ..)`) keep accumulating them. — probe `probe_subscription_entities_after_source_despawn`
- **Double delivery or silent no-op for raw value events (confirmed).**
  - `subscribe_to` on `ValueRemoved<T>` installs the generic `forward` observer when `T` is not yet tracked. `register::<T>` only `register_routed`s it, and `publish_removed` routes it *and* triggers it, so `forward` routes it a second time. — [src/subscription.rs:363-372](src/subscription.rs), [src/reactivity.rs:102-113](src/reactivity.rs), [src/reactivity.rs:330-337](src/reactivity.rs)
  - Probe: `removed deliveries for one removal: 2`.
  - `subscribe_to::<ValueChanged<T>>` with `T` untracked delivers nothing and gives no error (`changed deliveries before track: 0`). — probe `probe_raw_subscribe_to_value_events`
- **Silent `max_passes` truncation (confirmed).**
  - `run_reactive` falls out of its loop with no log entry when changes are still pending. — [src/reactivity.rs:131-145](src/reactivity.rs)
  - Probe: a self-incrementing effect advanced exactly 8 per frame and logged nothing (`cycle value after 1 frame: 8`). A real feedback cycle would spin forever, 8 passes per frame, with no warning. — probe `probe_max_passes_cycle_is_silent`

**Panics and deadlocks**
- Panicky paths:
  - `assert_subscription_registered` panics in a component hook. — [src/subscription.rs:378-386](src/subscription.rs)
  - `hook()` panics on a type mismatch. — [src/webby.rs:244-249](src/webby.rs)
  - `Memo::with_untracked` calls `expect`. — [src/coat.rs:409-413](src/coat.rs)
- Signals can deadlock or panic on re-entry:
  - `Signal` uses a `std::sync::RwLock`, so `s.with(|_| s.set(..))` or `s.update(|_| s.get())` re-enters the lock on the same thread, which deadlocks or panics. — [src/coat.rs:215-270](src/coat.rs)
  - A `Memo` whose `compute` reads itself recurses without bound, because `refresh` sees `value: None` and computes again. — [src/coat.rs:349-364](src/coat.rs)

**Performance**
- Every pass polls every host:
  - `run_effects` allocates `hosts: Vec<Entity>` and iterates *every* effect host, checking every dep, on every pass of every frame. That is O(hosts × deps) even when nothing changed. The README acknowledges this. — [src/runtime.rs:158-206](src/runtime.rs), [README.md:286-289](README.md)
  - Each effect run also calls `world.increment_change_tick()`. — [src/runtime.rs:188](src/runtime.rs)
- Host processing order is unspecified:
  - It follows `EntityHashSet` iteration order, not tree depth. A child can render in the same pass before its parent, and the parent's re-render then despawns it, wasting the child's render. — [src/runtime.rs:143](src/runtime.rs), [src/runtime.rs:162-164](src/runtime.rs)
- Publishing allocates even when nobody listens:
  - `publish::<T>` clones `T` into a new `Arc` for every changed entity, even when the entity has no `Subscribers<ValueChanged<T>>` and no global observer exists. — [src/reactivity.rs:312-324](src/reactivity.rs)
  - The delivered event is double-boxed: `SubscriptionEvent { event: Arc<ValueChanged<T> { value: Arc<T> }> }`. — [src/subscription.rs:416-427](src/subscription.rs)
- Subscriptions are entity-heavy:
  - Each `(listener, source)` pair costs two entities: the subscription entity plus the observer entity spawned by `.observe()`. — [src/subscription.rs:294-298](src/subscription.rs)
- Webby re-renders are expensive:
  - Every re-render despawns the whole rendered subtree and respawns it. Plain entities lose transient state (`Interaction`, focus, scroll, Feathers widget state), and every descendant component re-renders. There is no props memoization. — [src/webby.rs:543-556](src/webby.rs), [src/webby.rs:84-90](src/webby.rs)
  - `use_effect` callbacks run right after `spawn_child`, but child components only render in the next pass, so effects see empty child hosts. — [src/webby.rs:553-565](src/webby.rs)
- `coat` row updates rebuild instead of patching:
  - `for_keyed` respawns a row whenever its value changes (`old == item` check), rather than patching it in place. Toggling one todo's `done` rebuilds that row. — [src/coat.rs:622-631](src/coat.rs)
  - `spawn_list` inserts `ChildOf` once per entity, and `replace_children` then rebuilds the order a second time. — [src/coat.rs:524-543](src/coat.rs), [src/coat.rs:639-643](src/coat.rs)
  - `patch` marks `C` as changed on every run through `world.get_mut`, even when the value is identical. If `C` is tracked, this republishes `ValueChanged<C>`. — [src/coat.rs:561-565](src/coat.rs)
- `Disabled` is handled inconsistently:
  - Publishers include `Disabled` sources (`Allow<Disabled>`), and `bind` writes to `Disabled` targets.
  - `run_effects` skips `Disabled` hosts.
  — [src/reactivity.rs:313](src/reactivity.rs), [src/reactivity.rs:427](src/reactivity.rs), [src/runtime.rs:168-170](src/runtime.rs)

### Inferences
- Fixes for the confirmed bugs:
  - **Leak:** make `parent` a `Weak<Instance>`, and clear `children` when a host's `Effects` is removed.
  - **Duplicate keys:** despawn the displaced row, or emit a warning/`debug_assert!` on duplicate keys and suffix them.
  - **Depth limit:** turn `run_effects` into a work queue that drains hosts added during the pass (for example, have `register_host` push onto a "new hosts" queue that is polled until empty), and order it by hierarchy depth.
  - **Hook shift:** scope positional keys by `(render-pass position within parent ChildOf list)`, or require keys for conditionally rendered components. In either case, replace the panic with an error plus an instance reset.
  - **Orphans:** add an `on_remove` hook for `SubscribedTo<E>` that despawns the subscription entity when the relationship is dropped. Alternatively, make `Subscribers` `linked_spawn`, though that would despawn on source despawn, which is the desired behaviour here because the subscription entity is internal.
- Push-based dirty marking would turn per-frame cost from O(all deps) into O(changed):
  - Signals can keep a subscriber list of host entities and mark them dirty on `set`.
  - ECS deps can reuse the existing `ValueChanged` machinery or per-type `Changed<T>` queries.

### Gaps
- I did not benchmark the polling cost at scale. The O(hosts × deps) characterization comes from reading the code.
- I did not measure whether `increment_change_tick` per effect run causes any problem with tick wrap or `check_change_ticks`. Bevy handles wrap-around, so it is probably benign.

## Q4. What examples and tests reveal as pain points

### Takeaway
The same boilerplate shows up again and again:
- cloning signals before `coat!`;
- cloning `SetState` per event handler;
- exporting component state through `use_effect` + `insert_resource`;
- writing a placeholder initial value next to `bind`, because it does not replay;
- `settle`/`flush` choreography in tests and examples.

### Cited Findings
- Signals are cloned by hand before every `coat!`, because the macro moves its captures: `let (open, count) = (open.clone(), count.clone());` and similar. — [tests/coat.rs:65-66](tests/coat.rs), [tests/coat.rs:101-102](tests/coat.rs), [tests/coat.rs:141-142](tests/coat.rs), [examples/coat_tabs.rs:37-38](examples/coat_tabs.rs), [examples/coat_todos.rs:30-31](examples/coat_todos.rs)
- `memo` dependencies also need a manual clone block. — [examples/coat_tabs.rs:32-35](examples/coat_tabs.rs), [tests/coat.rs:136-139](tests/coat.rs)
- `SetState` is cloned once per handler: `let (minus, plus, reset) = (set_count.clone(), set_count.clone(), set_count);`. — [examples/comparison/webby.rs:44](examples/comparison/webby.rs)
- The only way to drive component state from outside is a `use_effect(())` that inserts a resource holding the setter. — [examples/webby_counter.rs:16-18](examples/webby_counter.rs), [tests/webby.rs:65-69](tests/webby.rs), [tests/webby.rs:272-274](tests/webby.rs)
- `bind` does not replay the current value, so the initial text is duplicated by hand (`Text("0")`). It would be wrong if `Count` started non-zero. The `Reset` button's `Display::None` has to be pre-set by hand for the same reason. — [examples/comparison/reactivity.rs:57-60](examples/comparison/reactivity.rs), [examples/comparison/reactivity.rs:70-80](examples/comparison/reactivity.rs); semantics documented at [README.md:272-274](README.md)
- The subscription comparison needs a hand-written `CountChanged` event, a `Labels` query alias and an `apply` helper to emulate state. — [examples/comparison/subscription.rs:77-96](examples/comparison/subscription.rs)
- Flush choreography:
  - Tests define `settle(app) { app.update(); world.flush(); }` again in several files. — [tests/reactivity.rs:40-43](tests/reactivity.rs), [tests/subscription.rs:30](tests/subscription.rs)
  - The subscription example needs `flush(); update(); flush();` plus another flush after `trigger`. — [examples/subscription.rs:32-34](examples/subscription.rs), [examples/subscription.rs:47](examples/subscription.rs)
- Reading a component's output needs `Children[0]` indexing into a ghost host. — [examples/webby_counter.rs:40-41](examples/webby_counter.rs)
- `coat!` patched text needs `Text($(format!("{}", $count)))` even for a plain display. — [examples/comparison/coat.rs:27](examples/comparison/coat.rs)
- `coat!` event handlers write `count.set($count - 1)` instead of `count.decrement()`, a read-modify-write. — [examples/comparison/coat.rs:24](examples/comparison/coat.rs), [examples/comparison/coat.rs:34](examples/comparison/coat.rs)
- Webby components cannot branch to different scene types without boxing:
  - Probe: `if depth == 0 { bsn!{..} } else { bsn!{..} }` in a `#[component]` fails with E0308.
  - The comparison example works around it with `{(count != 0).then(|| bsn_list!{..})}`.
  — [examples/comparison/webby.rs:76-82](examples/comparison/webby.rs); probe compile error.
- Missing test coverage:
  - duplicate keys, deep nesting, unmount and leak, conditional siblings, effect ordering, `max_passes` exhaustion, removal via raw `subscribe_to`, `Disabled` hosts;
  - no unit tests in `src/` at all;
  - 9 of 11 doctests are `ignore`d.
  — test listing in [tests/](tests/), doctest run output (Q7)

### Inferences
- These would each remove a repeated pattern:
  - `coat!` auto-cloning captured signals from the enclosing scope (it already clones per site internally), or `Copy` signal handles.
  - A `bind_now`/`replay` option.
  - A `Signal` you can pass in as a prop to drive state from outside, with docs that say so.
  - A `bevy_radioactive::testing::settle` helper.

### Gaps
- I could not run the `comparison_*` examples interactively; they need a window and the `comparison` feature.

## Q5. Features users would expect that are missing

### Takeaway
The obvious gaps are:
- an initial-value replay for `watch`/`bind`;
- computed or derived *ECS* values;
- effect cleanup and unmount hooks;
- ECS reads from `coat!`;
- in-place keyed-row updates;
- async/task hooks;
- error-returning renders;
- diagnostics.

### Cited Findings
- **No replay.** `watch`/`bind` "do not replay the current value when a listener attaches". — [README.md:272-274](README.md), [src/reactivity.rs:386-389](src/reactivity.rs)
- **No ECS-derived values.** `memo` only tracks signals (`scope` collects `SignalDep`s), and nothing derives one component from others the way `bind` does but standalone. — [src/coat.rs:349-364](src/coat.rs)
- **No effect cleanup or unmount hook.** `use_effect` takes `FnOnce(&D, &mut Commands)` with no return value, and `coat::effect` has the same gap. — [src/webby.rs:331-353](src/webby.rs), [src/coat.rs:463-479](src/coat.rs)
- **No ECS reads in `coat!`.** `$` maps to `.get()` on a signal, and there is no syntax for resources or components. — [macros/src/lib.rs:196-224](macros/src/lib.rs)
- **No async.** `#[component]` explicitly rejects `async`, and there is no hook for tasks or futures. — [macros/src/lib.rs:778-783](macros/src/lib.rs)
- **No error handling in renders.** Render must return `impl Scene`, and spawn errors go to `fallback_error_handler`. — [src/runtime.rs:370-384](src/runtime.rs)
- **No props memoization.** Nothing like `React.memo` exists, so child components always re-render with their parent, because their entities are respawned. — [src/webby.rs:543-556](src/webby.rs)
- **No diagnostics.** There are no `tracing` spans for passes, effect counts or a `max_passes` warning. — [src/reactivity.rs:130-146](src/reactivity.rs)
- **Resource/component asymmetry.** `Singleton` must exist at resolve time, and the error fires otherwise. There is no "resource may appear later" source, even though `Reads::resource` handles absence. — [src/reactivity.rs:347-362](src/reactivity.rs), [src/runtime.rs:72-80](src/runtime.rs)
- **No `ReactiveSet` ordering helpers or a `ReactiveSet::Settled` marker.** Users have to order `.before(ReactiveSet::Publish)` themselves, and `ReactiveSet` is not in the prelude. — [src/reactivity.rs:38-45](src/reactivity.rs), [src/prelude.rs:3-6](src/prelude.rs)

### Inferences
- Sketches for each gap:
  - **Replay:** a `Source` modifier such as `Current(#A)`, or `bind(..).immediate()`. It would run the writer at build time from the current world value (the template has `world_scope` access).
  - **Derived components:** `derive::<C>(sources, |a: &A, b: &B| C)` registered as a webby-style effect that writes `C` through `set_if_neq`.
  - **Effect cleanup:** follow the Leptos/Solid/React shape. `use_effect(deps, |d, cmds| -> impl FnOnce(&mut Commands))`, and `Cx::on_unmount`.
  - **ECS reads in `coat!`:** `$res(Counter)` / `$get(entity, Health)`, or let `coat::effect`, `patch` and `region` accept `&Reads`.
  - **Async:** `cx.use_task(deps, async move { .. }) -> Option<T>` using `AsyncComputeTaskPool`, re-rendering when the task completes.

### Gaps
- I did not survey comparable Bevy reactive crates such as `bevy_reactor`, `haalka`, `quill` or `bevy_cobweb` in this assignment. That is out of scope (codebase-only); another researcher may cover them.

## Q6. Proposed improvements (before/after, files, breaking, priority)

### Takeaway
Fix the five confirmed bugs first. All are non-breaking and small. Next, cut the boilerplate with signal auto-capture, `bind` replay and effect cleanup. Then consolidate the duplicate entry points and naming in one breaking 0.5 pass. Longer term, move from polling to push-based dirtiness and patch webby subtrees instead of respawning them.

### Cited Findings
Each item cites the code it changes. All before/after sketches are my own proposals.

**HIGH priority**

1. **Break the `Instance` cycle.** Non-breaking. File: [src/webby.rs:146-179](src/webby.rs).
   ```rust
   // before
   struct Instance { parent: Option<Arc<Instance>>, .. }
   // after
   struct Instance { parent: Option<Weak<Instance>>, .. }
   // use_context: let mut ancestor = self.instance.parent.as_ref().and_then(Weak::upgrade);
   ```
   Also clear `children` when the host's `Effects` is dropped. Add a regression test, modelled on `probe_instance_tree_freed_on_despawn`.

2. **Handle duplicate keys in `for_keyed`.** Non-breaking. File: [src/coat.rs:620-644](src/coat.rs).
   ```rust
   // before
   next_rows.insert(item_key, (item, children));
   // after
   if let Some((_, dup)) = next_rows.insert(item_key, (item, children)) {
       warn_once!("coat: duplicate #[key]; extra rows replaced");
       despawn_all(world, &dup); next_order.retain(|e| !dup.contains(e));
   }
   ```

3. **Drain hosts spawned during a pass.** Non-breaking. Files: [src/runtime.rs:143-206](src/runtime.rs), [src/reactivity.rs:130-146](src/reactivity.rs). This removes the depth ≈ `max_passes` limit and the one-frame partial UI.
   ```rust
   // before: let hosts: Vec<Entity> = hosts.0.iter().copied().collect(); for host in hosts { .. }
   // after: a VecDeque `pending` fed by register_host; loop { let Some(h) = pop() else break; .. }
   //        sorted/ordered parent-before-child (e.g. by ChildOf depth) to avoid wasted child renders.
   ```

4. **Stop the panic and state swap from positional hooks.** Behaviour change, but non-breaking at the API level. File: [src/webby.rs:165-179](src/webby.rs), [src/webby.rs:231-253](src/webby.rs).
   - Option A: key unkeyed children by their BSN call-site, a `#[track_caller]`/`Location` captured in `#[component]` expansion, plus an occurrence index. That is more stable than one global counter.
   - Option B: on a type mismatch, drop the instance's hooks, log a `warn!`, and re-init instead of panicking.
   ```rust
   // before
   let hook = slot.downcast_mut::<H>().unwrap_or_else(|| panic!(..));
   // after
   if !slot.is::<H>() { warn!(..); slot = Box::new(init()); }
   ```

5. **Despawn orphaned subscription entities.** Non-breaking. File: [src/subscription.rs:127-157](src/subscription.rs).
   ```rust
   // add to SubscribedTo<E>: #[component(on_remove = despawn_if_internal)]
   // fn despawn_if_internal(mut w: DeferredWorld, ctx: HookContext) {
   //     if w.get::<SubscriptionOf>(ctx.entity).is_some() { w.commands().entity(ctx.entity).try_despawn(); }
   // }
   ```
   Alternative: make `Subscribers<E>` `linked_spawn`. That is safe because BSN subscription entities are internal, but direct `SubscribedTo` users would also be despawned.

6. **Warn when `max_passes` is exhausted.** Non-breaking. File: [src/reactivity.rs:130-146](src/reactivity.rs).
   ```rust
   // after the for-loop
   warn_once!("bevy_radioactive: reactive passes hit max_passes={max_passes}; possible cycle (remaining work deferred)");
   ```

**MEDIUM priority**

7. **Stop double delivery and the silent no-op for raw value events.** Non-breaking. Files: [src/subscription.rs:363-372](src/subscription.rs), [src/reactivity.rs:102-113](src/reactivity.rs).
   - Option A: give `ValueChanged`/`ValueRemoved` a marker trait (`RoutedEvent`) so `install_subscription` skips `forward` and calls `register::<T>`.
   - Option B: in `register::<T>`, remove any `forward::<ValueRemoved<T>>` observer that is already installed.

8. **Add initial-value replay for `watch`/`bind`.** Additive. File: [src/reactivity.rs:386-490](src/reactivity.rs), with a new method on `Subscribe` in [src/subscription.rs:252-311](src/subscription.rs).
   ```rust
   // before (examples/comparison/reactivity.rs:57-60)
   Text("0")
   bind(Singleton, |c: &Count, t: &mut Text| **t = c.0.to_string())
   // after
   Text::default()
   bind(Singleton, |c: &Count, t: &mut Text| **t = c.0.to_string()).immediate()
   // build_template: after resolving sources, read T from each source via world_scope and call write once.
   ```

9. **Unify the effect signature and add cleanup.** Breaking for `coat::effect` and `use_effect`. Files: [src/coat.rs:463-479](src/coat.rs), [src/webby.rs:331-353](src/webby.rs), [src/runtime.rs:129](src/runtime.rs). `Slot` gains `cleanup: Option<Box<dyn FnOnce(&mut World)>>`, which runs before a re-run and in `Effects`'s `on_remove` hook.
   ```rust
   // before
   effect(move |cmds| ..);                 cx.use_effect(deps, |d, cmds| ..);
   // after
   effect(move |reads: &Reads, cmds| { ..; /* optional */ Cleanup::new(|cmds| ..) });
   cx.use_effect(deps, |d, cmds| { ..; move |cmds: &mut Commands| { /* cleanup */ } });
   cx.on_unmount(|cmds| ..);
   ```

10. **Let `coat!` capture signals automatically, or make signals `Copy`.** Changing the macro is non-breaking; `Copy` handles are a breaking redesign. Files: [macros/src/lib.rs:23-57](macros/src/lib.rs), [src/coat.rs:194-331](src/coat.rs).
    - Macro option: the macro already hoists `let __coat_N_x = x.clone()` into the prelude ([macros/src/lib.rs:78-91](macros/src/lib.rs)). If it also emitted `let x = x.clone();` *outside* the returned block, callers would not need `let (a, b) = (a.clone(), b.clone());`. In practice the remaining problem is that the caller keeps using `a` after the macro, so the macro should borrow (clone from `&x`) instead of moving. Check that the outer `bsn!` closure capture is `move` of the hoisted clones only.
      ```rust
      // before (tests/coat.rs:65-80)
      let scene = { let (open, count) = (open.clone(), count.clone()); coat! { .. } };
      // after
      let scene = coat! { .. };   // `open`/`count` still usable afterwards
      ```
    - Long-term option: arena-backed `Copy` signals (Leptos 0.7 style), stored in a world resource keyed by `(Entity owner, index)`. This also removes `SetState` cloning ([examples/comparison/webby.rs:44](examples/comparison/webby.rs)).

11. **Make `match` regions key on the arm index, as `if` regions do.** Behaviour change. File: [macros/src/lib.rs:720-729](macros/src/lib.rs). Generate a selector `match $x { P1 => 0, P2 => 1, .. }` and pass bound values through a separate patched or re-run path, or document the current value-keyed semantics.

12. **Avoid cloning in `$` reads and list items.** Non-breaking. File: [macros/src/lib.rs:196-224](macros/src/lib.rs), [macros/src/lib.rs:618-620](macros/src/lib.rs).
    - Expand `$x.method(..)` to `x.with(|x| x.method(..))` when the result does not borrow, or add `$&x` meaning `x.with(..)`.
    - For `for`, iterate inside `with` and collect only keys and row values.

13. **Skip allocation when nobody is listening.** Non-breaking. File: [src/reactivity.rs:312-324](src/reactivity.rs).
    ```rust
    // query (Entity, &T, Has<Subscribers<ValueChanged<T>>>); skip Arc/clone when !has && !registry.global_observers.contains(TypeId::of::<T>())
    ```
    This needs `observe_value` to record global interest. Users of a plain `add_observer(On<ValueChanged<T>>)` would need `track_global::<T>()`, or the check is skipped when unsure.

14. **Make `patch` honour `set_if_neq`.** Non-breaking. File: [src/coat.rs:553-569](src/coat.rs). Patch through `Mut::bypass_change_detection` + compare, or require `C: PartialEq` for macro-generated patches, so tracked components are not republished needlessly.

15. **Clean up the docs.** Non-breaking.
    - Fix the README "synchronously" claim ([README.md:275](README.md)) and the global-mutation `on_removed` example ([README.md:129-133](README.md)).
    - Trim the 0.1→0.2 MIGRATION content or move it to the changelog ([MIGRATION.md:33-337](MIGRATION.md)).

**LOW priority (consolidation; breaking, best done in one 0.5 pass)**

16. **Naming.** Files: [src/reactivity.rs](src/reactivity.rs), [src/lib.rs](src/lib.rs), [src/prelude.rs](src/prelude.rs).
    - Rename `on_removed` → `watch_removed`, so the family reads `watch` / `watch_removed` / `bind` / `bind_or_else`.
    - Keep the old names as `#[deprecated]` aliases for one release.
    - Rename `Singleton` → `ResourceOf`, or document why it is called `Singleton`.

17. **Prelude coverage.** File: [src/prelude.rs:3-6](src/prelude.rs). Add `ReactiveSet`, `ReactiveSettings`, `Signal`, `signal`, `memo` and `coat`. Alternatively, ship `prelude::{coat, webby}` sub-preludes so `webby::*` also brings in `signal`/`memo`.

18. **Collapse duplicates.**
    - Remove `keyed()` in favour of `.key()` ([src/webby.rs:477-479](src/webby.rs)). If `@keyed(..)` must stay for BSN `@` syntax, drop `.key` from the docs instead.
    - Document `use_component`/`use_resource` as aliases of `cx.get(..).cloned()`, or drop them.
    - Pick one of `Cx::entity()` and `Reads::target()`.

19. **Swap `use_reducer` arguments** to `(reducer, init)` to match React. File: [src/webby.rs:280-292](src/webby.rs).

20. **Fix `#[component]` Entity-prop detection.** File: [macros/src/lib.rs:810-821](macros/src/lib.rs). Detect via a `#[entity]` attribute on the prop, or accept `impl Deps` explicitly, so that `Option<Entity>` and aliases work. Remove the dead `let _ = uses_cx;`.

21. **Offer `Result<impl Scene>` renders.** File: [src/webby.rs:614-632](src/webby.rs). Errors would go to Bevy's error handler with a component-name context, instead of forcing infallible renders.

22. **Ship a testing helper.** New module `testing` with `fn settle(app: &mut App)`. It would replace the copies in [tests/reactivity.rs:40-43](tests/reactivity.rs) and [tests/subscription.rs:30](tests/subscription.rs).

**Long-term / architectural (MEDIUM impact, large effort)**

23. **Push-based dirtiness instead of polling.**
    - Signals: give `SignalInner` a `subscribers: Mutex<SmallVec<HostId>>`, and have `notify()` push them onto a dirty queue resource (signals are `Send+Sync`, so use a global `crossbeam`/`Mutex` queue).
    - ECS deps: index by `ComponentId` and check once per pass with `Changed` archetype filters.
    - Files: [src/runtime.rs:94-206](src/runtime.rs), [src/coat.rs:120-187](src/coat.rs).

24. **Reconcile webby renders instead of respawning.** Webby re-render would diff against the previous BSN output, patching components in place and keeping entity identity. This could reuse `coat`'s `patch`/`for_keyed` machinery per child list. File: [src/webby.rs:520-568](src/webby.rs).

### Inferences
- Items 1–7 can ship in a 0.4.x patch with regression tests derived from the probes.
- Items 8–15 make a natural 0.5 "ergonomics" release.
- Items 16–22 should be batched, with deprecation aliases, so users migrate once. MIGRATION.md already uses a table format that suits this.
- Items 23–24 are what would make webby viable beyond small UIs. The README's own advice to "keep components small" ([README.md:288-289](README.md)) suggests the author already sees this limit.

### Gaps
- The proposals have not been prototyped. Effort estimates are judgment calls. For item 10, the exact macro capture semantics need checking against `bsn!`'s internal `move` closures, which I did not inspect in `bevy_scene_macros`.

## Q7. `cargo test` and `cargo clippy --all-targets` results

### Takeaway
Everything passes and clippy is clean at `pedantic` on default features. This is because the existing tests do not exercise the bug paths above. Those failed when I ran them in the scratch copy.

### Cited Findings
- `cargo test` (default features) passed:

  | Suite | Result |
  | --- | --- |
  | `tests/coat.rs` | 7 passed |
  | `tests/reactivity.rs` | 14 passed |
  | `tests/subscription.rs` | 7 passed, including one `should_panic` |
  | `tests/webby.rs` | 9 passed |
  | Doctests | 2 passed, 9 ignored (all the `ignore`-fenced examples in README, `coat`, `webby` and `bind`/`bind_or`) |

  — local run on 2026-10-26.
- `cargo clippy --all-targets` finished with **zero warnings**, with `clippy::pedantic` set to warn in `[lints.clippy]`. — [Cargo.toml:32-35](Cargo.toml); local run.
- The `comparison_*` examples have `required-features = ["comparison"]`, so the default `--all-targets` run skipped them. Neither compiled nor linted. — [Cargo.toml:37-55](Cargo.toml)
- Probe tests, run in a scratch copy against the same target directory, showed 5 failures and confirmed 2 behaviours:
  - Failures: `probe_instance_tree_freed_on_despawn` (0/2 drops), `probe_duplicate_keys_leak_rows` (1 leaked), `probe_nesting_depth_per_frame` (leaf only on frame 2), `probe_positional_hook_state_shift` (panic at `src/webby.rs:245`), `probe_subscription_entities_after_source_despawn` (5 orphans).
  - Confirmed behaviours: `probe_raw_subscribe_to_value_events` (2 deliveries per removal; 0 before tracking) and `probe_max_passes_cycle_is_silent` (8 per frame, no log).
  — local probe run.

### Inferences
- CI should add `cargo clippy --all-targets --features comparison` and `cargo test --no-default-features`, so the `ui`-off code paths and the comparison examples are built.
- The probe tests could be adopted directly as regression tests.

### Gaps
- I did not run `cargo clippy --all-features` or `--no-default-features`, because a full Bevy UI/Feathers build takes a long time. Warnings in those configurations are unknown.
- I did not run the `comparison` examples.
