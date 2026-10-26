# Reactive design patterns outside Bevy: transferable API ideas for an ECS-based reactive library

Scope note: sources were fetched on 2026-10-26. Items marked "(background)" come from prior knowledge and were not re-verified in this session. Treat them as leads, not citations. For context on what to compare against: bevy_radioactive already exports `signal`, `memo`, `effect`, `watch`, `resource`/`use_resource`, `provide_context`/`use_context`, `untracked`/`get_untracked`/`with_untracked`, `set_if_neq`, `patch`, `for_keyed`/`for_indexed`, `keyed`, `observer`, `on_removed`, `Region`, `snapshot`, `use_reducer`, `use_ref`, and the `Subscription*` relationship types (from `grep` of `C:\Code\bevy_radioactive\src`).

## Fine-grained reactivity in Rust (Leptos reactive_graph / reactive_stores, Dioxus, Xilem) and the algorithms behind glitch-free, minimal recomputation

### Takeaway
The Rust frameworks split into two camps. Leptos and Dioxus use push-pull fine-grained graphs: Leptos's algorithm is explicitly based on Reactively's three-colour scheme, and Dioxus makes handles `Copy` through a generational arena. Xilem avoids shared mutable state altogether by diffing view trees and threading `&mut AppState` through event dispatch. The coloring algorithm (mark CHECK/DIRTY on push, `updateIfNecessary` on pull) is the simplest known way to get glitch-free, minimal recomputation, and it maps cleanly onto per-entity marker state in an ECS.

### Cited Findings
**Leptos `reactive_graph` (0.2.x)**
- Three node categories. "Signals: atomic units of state, which can be directly mutated." Computations (sync and async) are derived values, and effects sync the graph with the outside world. Signals and computations are "source" nodes, while computations and effects are "subscriber" nodes. — [docs.rs reactive_graph](https://docs.rs/reactive_graph/latest/reactive_graph/)
- The algorithm "is based on that of Reactively". "Effects are expensive," so the design avoids re-running effects even at some cost in raw speed. "Subscribers unsubscribe from their sources between runs," which gives dynamic dependencies, so inactive branches don't trigger. — [docs.rs reactive_graph](https://docs.rs/reactive_graph/latest/reactive_graph/)
- Writes are immediate, but "effects that depend on it will not run until the next 'tick' of the async runtime". Effects are always deferred and batched by the scheduler. — [docs.rs reactive_graph](https://docs.rs/reactive_graph/latest/reactive_graph/)
- The `owner` module is "The reactive ownership model, which manages effect cancellation, cleanups, and arena allocation." — [docs.rs reactive_graph](https://docs.rs/reactive_graph/latest/reactive_graph/)
- Access API: `.get()` clones and tracks. `.read()` returns a guard: "you cannot update the value of the signal until this guard is dropped, or it will cause a runtime error". `.with(|v| ..)` gives scoped access by reference. `.set()`, `.write()` (mutable guard), `.update(|v| ..)`, and `.try_update` returns a value. The methods are blanket traits composed from `ReadUntracked + Track`. — [Leptos book: working with signals](https://book.leptos.dev/reactivity/working_with_signals.html)
- Guidance: writing B from an effect whenever A changes is "officially discouraged"; prefer derived signals or memos (top-down). `Memo::new(move |_| ..)` and a derived closure `move || count.get()*2` are the two derivation forms. Thread-safe by default (`Send + Sync`), with `*_local` variants for `!Send`. — [Leptos book](https://book.leptos.dev/reactivity/working_with_signals.html)
- Context: `provide_context(x)` / `use_context::<T>()` / `expect_context`. It "works across any distance", and a context-provided signal "only causes reactive updates where it is read". — [Leptos book: global state](https://book.leptos.dev/15_global_state.html)

**Leptos `reactive_stores` (0.4.x): field-level reactivity**
- `#[derive(Store)]` generates a `FooStoreFields` trait with one getter per field, giving "arbitrarily-deep access to the fields of some data structure" without wrapping each field in a signal. Types: `Store` (arena), `ArcStore` (refcounted), `Field`, `Subfield`, `ArcField`, `KeyedSubfield`, `FieldKeys`, `KeyedAccess`. — [docs.rs reactive_stores](https://docs.rs/reactive_stores/latest/reactive_stores/)
- Implementation: a single `Arc<RwLock<T>>` plus a map from field paths to triggers. `.read()` tracks the trigger for that path and `.write()` notifies it. "Updating a field will notify its parents and children, but not its siblings." — [docs.rs reactive_stores](https://docs.rs/reactive_stores/latest/reactive_stores/)
- `#[derive(Patch)]`: "provide an entirely new value, but only notify fields that have changed". `PatchFieldKeyed` patches collections "after doing a keyed diff". Vec access is through `at_unkeyed(i)`. Enums get variant accessors (`first()`, `second_0()`), Option gets `unwrap()`. — [docs.rs reactive_stores](https://docs.rs/reactive_stores/latest/reactive_stores/)
- Example: `let count = state.count(); *count.write() += 1;` does not notify readers of `state.name`. — [Leptos book: global state](https://book.leptos.dev/15_global_state.html)

**Dioxus signals (0.7)**
- `Signal<T>` is `Copy` because it is a handle into a `GenerationalBox` arena: "the `GenerationalBox` simply copies a handle to the value." Each signal is owned by its component scope and dropped on unmount. Reading a disposed signal "will cause a runtime panic". `Signal::new()` inside a component lives until unmount, so memory can grow. — [Dioxus signals](https://dioxuslabs.com/learn/0.7/essentials/basics/signals)
- Borrow rules are checked at runtime: `.write()` "checks at runtime if the value is safe to access", and overlapping guards panic. Holding a guard across `.await` is the classic bug. — [Dioxus signals](https://dioxuslabs.com/learn/0.7/essentials/basics/signals)
- Reading a memo right after writing its dependency recomputes it immediately (pull consistency). "Writes are batched and effects are deduplicated before the next step." — [Dioxus signals](https://dioxuslabs.com/learn/0.7/essentials/basics/signals)

**Xilem (view-diffing alternative)**
- The app is a function from state to a short-lived view tree, diffed against the previous tree to `rebuild` a persistent widget tree. "At any point in time, there are at most two copies of the view tree in existence." — [Raph Levien, Xilem architecture](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)
- Events are routed by **id path** (the sequence of ids from the root), and handlers get a `&mut` borrow of app state. Adapt nodes (descended from Druid lenses) take "a closure that takes mutable access to the parent state, and calls into a child". Memoize nodes rerun only when a `Clone + PartialEq` value changes; `Arc` pointer equality makes this cheap. — [Xilem architecture](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)
- Rationale: the observer/setter pattern needs shared mutable access, "awkward in Rust". — [Xilem architecture](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)

**Algorithms**
- Reactively coloring: "When a node changes, we color it red (dirty) and all of its children green (check)." Push only marks. On read, `updateIfNecessary()` handles each state. CLEAN returns the cache. DIRTY recomputes. CHECK walks the sources in order, calling `updateIfNecessary` on each, and breaks as soon as the node becomes dirty, so sources no longer used are never updated. If no source changed, "nothing has changed and the visited nodes are set uncolored." — [Milo, Super-charging fine-grained reactive performance](https://dev.to/modderme123/super-charging-fine-grained-reactive-performance-47ph)
- Comparison in the same article: MobX used an eager two-pass count of pending parents (later moved lazy). Preact uses version numbers on nodes and edges. Solid queues updates and uses the same coloring. — [Milo](https://dev.to/modderme123/super-charging-fine-grained-reactive-performance-47ph)
- alien-signals (StackBlitz): push-pull, combining "Vue 3's propagation, Preact's double-linked-list approach, Svelte's effect scheduling, and Reactively's graph coloring". There is no Array/Set/Map and no recursion. The graph is made of intrusive `Link {dep, sub, nextDep, nextSub}` nodes with bit flags (Dirty, Pending, Mutable, Watching, Recursed…). The core was "ported back to Vue 3.6", and a Rust port exists. It has `effectScope(fn)` for group disposal and `trigger()` for manual notify after in-place mutation. — [alien-signals](https://github.com/stackblitz/alien-signals)

### Inferences
- **Coloring maps directly onto ECS.** Store `ReactiveState { Clean | Check | Dirty }` as a component on memo and effect entities, with dependency edges as relationship components (bevy_radioactive already has `Subscription*`). Push = an observer or system that walks `Subscribers` and sets Check/Dirty. Pull = memo read calls `update_if_necessary`. Exclusive `&mut World` access in the pull path is needed because recomputation may touch arbitrary components. This argues for the memo pull path being an exclusive-world operation, while effects run in a scheduled system that drains `Dirty` effects in a fixed order.
- **Effects deferred to a tick, as in Leptos.** In Bevy terms: mark during commands or observers, then run effects in a dedicated schedule (for example after `Update`, or in `PostUpdate` before rendering). That gives batching for free and avoids re-entrancy inside observers.
- **Arena handle vs Arc.** Bevy's `Entity` already is a generational handle, like Dioxus's `GenerationalBox` and Leptos's arena `Store`. A `Signal<T>` that is a typed `Entity` wrapper is Copy and naturally has use-after-despawn semantics (return `Option`/`Result` rather than panic, unlike Dioxus).
- **Stores fit components better than per-field signals.** Leptos's "one `RwLock<T>` + path→trigger map" equals "one component + per-field dirty bits". A `#[derive(ReactiveFields)]` on a component could generate field accessors whose `.write()` sets a bit in a sidecar `FieldChanged<C>(u64)` bitmask. Subscribers record `(entity, ComponentId, field_mask)`. Parent/child notification and sibling isolation are then bitmask overlap tests. `Patch` maps onto bevy_radioactive's `patch`/`set_if_neq`: diff the old and new value per field and set only the changed bits.
- **Xilem's lesson for Bevy:** the `&mut World` exclusivity model is closer to Xilem than to Leptos. Where possible, prefer passing `&mut` state into handlers (observers already receive `On<E>` plus system params) over shared-handle closures. Id paths roughly correspond to Bevy's entity hierarchy plus event bubbling (`propagate`).

### Gaps
- Sycamore and Floem were not fetched this session. (background) Both are signal-based like Leptos. Floem uses an arena-backed `RwSignal` with views built once and updated by effects. Sycamore 0.9 moved to Copy signals tied to a root/owner. Not verified.
- I did not fetch the internals of Leptos `ArcRwSignal` versus `RwSignal`, or of `Resource`/`AsyncDerived`. (background) The Arc variants are refcounted and independent of an Owner; the plain variants are arena-stored and disposed with their Owner. `AsyncDerived` is an async memo whose futures track reads across `.await` points.
- I did not capture the `#[store(key = ...)]` attribute syntax for keyed store fields. The docs page lists the types but not the attribute.

## JS ecosystem primitives (SolidJS, Preact, TC39 Signals, Vue, Svelte 5, Jotai; MobX and React only from background)

### Takeaway
The JS ecosystem has converged on four ideas: lazy, cached computeds with dynamic auto-tracking; effects scheduled separately from propagation (TC39 deliberately standardises only `State`, `Computed` and a low-level `Watcher`); equality cut-off; and explicit escape hatches (`untrack`/`peek`). The most portable specialised primitives are Solid's `createSelector` (keyed boolean subscriptions), Solid's `<For>`/`<Index>` split, Vue's watch flush modes and cleanup registration, Preact's `batch`, and Jotai's `atomFamily` as a warning about keyed caches leaking memory.

### Cited Findings
**TC39 Signals (Stage 1)**
- API: `new Signal.State(v, {equals})` with `.get()/.set()`; `new Signal.Computed(fn)` with `.get()`; `Signal.subtle.Watcher(notify)` with `watch/unwatch/getPending`; `Signal.subtle.untrack(fn)`; introspection with `introspectSources/introspectSinks/hasSinks/hasSources/currentComputed`. — [tc39/proposal-signals](https://github.com/tc39/proposal-signals)
- Computeds "track their dependencies dynamically" and "are only re-evaluated when they are accessed". The model is "push-pull". Glitch-free: "no unnecessary calculations are ever performed". Errors are cached like values. — [proposal-signals](https://github.com/tc39/proposal-signals)
- Deliberately excluded: "The Signal API does not include any built-in function like `effect`", "There is no built-in batching", and async and transactions are "Omitted for now". The Watcher `notify` runs synchronously during `.set()`, and "No signals may be read or written during the notify." — [proposal-signals](https://github.com/tc39/proposal-signals)

**SolidJS**
- `createSelector(source, fn?) => (key) => boolean` is "a keyed boolean accessor derived from a source accessor". Each subscriber tracks only its own key, so when the selection changes, only subscribers whose key started or stopped matching update. Typical use: `classList={{active: isSelected(item.id)}}` inside `<For>`. — [Solid docs: createSelector](https://docs.solidjs.com/reference/secondary-primitives/create-selector)
- `<For>` keeps the item fixed per node and the index is a signal; on reorder it moves existing nodes and updates their index. `<Index>` keeps the index fixed and the item is a signal: "only the content at the specified index is updated". Use `<Index>` for primitives and inputs. — [Solid docs: list rendering](https://docs.solidjs.com/concepts/control-flow/list-rendering)
- Solid queues updates and processes them with Reactively-style coloring (per a comment by Milo). — [Milo](https://dev.to/modderme123/super-charging-fine-grained-reactive-performance-47ph)

**Preact signals**
- `signal`, `computed` (read-only, lazy: "only compute new values when their value has been accessed"), and `effect(fn)`, where returning a function registers cleanup that runs before the next run. Calling the returned disposer unsubscribes. Assigning an equal value is a no-op. — [Preact signals guide](https://preactjs.com/guide/v10/signals/)
- `batch(fn)` combines writes "into one 'commit' at the end". Nested batches flush at the outermost one. Reads inside a batch see new values. Invalidated computeds recompute only when read, and effects wait until the batch ends. `untracked(fn)` and `.peek()` read without subscribing. — [Preact signals guide](https://preactjs.com/guide/v10/signals/)

**Vue 3.5/3.6**
- `watch(source, cb)` tracks only the explicit source, while `watchEffect(fn)` auto-tracks everything read during its synchronous execution, with dependencies after the first `await` not tracked. — [Vue watchers](https://vuejs.org/guide/essentials/watchers.html)
- Flush modes: `pre` (default: after parent updates, before the owner's DOM update), `post` (`watchPostEffect`), and `sync` (`watchSyncEffect`). "Sync watchers do not have batching." — [Vue watchers](https://vuejs.org/guide/essentials/watchers.html)
- Cleanup: `onWatcherCleanup(() => controller.abort())` must be called synchronously, while the `onCleanup` argument is bound to the watcher and can be used after `await`. Other options: `deep` (can be a number for max depth), `once`, `immediate`. Watchers created synchronously in setup are auto-stopped on unmount; watchers created asynchronously "must be stopped manually to avoid memory leaks". — [Vue watchers](https://vuejs.org/guide/essentials/watchers.html)
- Vue 3.5 moved to a Preact-like pull algorithm, and 3.6 ports alien-signals. — [alien-signals README](https://github.com/stackblitz/alien-signals)

**Svelte 5**
- `$derived(expr)` ≡ `$derived.by(() => expr)`, and it should be side-effect free. Push-pull: changes are pushed immediately, but "derived values are not re-evaluated until they are actually read". Equality cut-off: "If the new value of a derived is referentially identical to its previous value, downstream updates will be skipped." Since 5.25, deriveds can be temporarily overridden by assignment (optimistic UI). State read after `await` is tracked in async deriveds. `untrack` exempts reads. — [Svelte $derived](https://svelte.dev/docs/svelte/$derived)

**Jotai**
- `atomFamily(init, areEqual)` returns `(param) => Atom`, cached by param. "Internally, atomFamily is just a Map whose key is a param and whose value is an atom config… Unless you explicitly remove unused params, this leads to memory leaks." It offers `remove(param)`, `setShouldRemove((createdAt, param) => bool)`, `getParams()`, and an unstable `unstable_listen` for CREATE/REMOVE events. In Jotai v3 it was removed from `jotai/utils` and moved to the `jotai-family` package, which also adds `atomTree`. — [Jotai family docs](https://jotai.org/docs/utilities/family)

### Inferences
- **Selector → ECS.** Implement it as a `Selector<K>` resource or entity holding `current: Option<K>` and a `HashMap<K, SmallVec<SubscriberEntity>>`. On change from `a` to `b`, notify only the subscribers of `a` and `b`, which is O(1) in list size. With `K = Entity` this is the "selected entity" or "hovered entity" pattern. An even more ECS-native form is to move a marker component (`Selected`) from `a` to `b` and let `Added<Selected>`/`On<Remove, Selected>` observers fire on exactly two entities. Bevy observers effectively give a selector for free when the key is an entity.
- **For vs Index** maps to bevy_radioactive's existing `for_keyed`/`for_indexed`. The transferable detail is which side is reactive: in keyed mode, child entities persist per key and receive an index signal; in indexed mode, child entities persist per slot and receive an item signal. With BSN scenes, keyed should reuse spawned scene entities and reorder `Children` rather than despawn and respawn.
- **TC39's minimalism is a good layering guide.** Make the core `State`/`Computed`/`Watcher` scheduler-agnostic, and build `effect` on top as a Bevy system or schedule. A Watcher-like hook (notify on "maybe changed", no reads allowed) maps to a Bevy observer or a push of a `Dirty` marker. Reading inside notify is forbidden, just as structural changes inside some Bevy hooks must go through `Commands`.
- **Vue flush modes map to schedule placement:** `pre` = before the UI layout system, `post` = after layout or transform propagation (needed for effects reading `ComputedNode`/`GlobalTransform`), `sync` = immediate observer (`On<Insert>`). Exposing `EffectFlush::{Pre, Post, Sync}`, or a `.in_schedule(...)` option, is a cheap, high-value API.
- **Cleanup registration:** offer both a return-a-cleanup closure (Preact) and an `on_cleanup` registered with the current owner (Vue/Solid `onCleanup`). In ECS, an owner is an entity, and cleanups can run from an `On<Despawn>` / `On<Remove, Effect>` observer, or before each rerun.
- **Batching:** Bevy's command buffer plus deferred effects already batch within a frame. An explicit `batch(|world| ..)` is only needed for synchronous/exclusive writes, to suppress intermediate pushes. Preact's semantics (reads inside see new values, deriveds recompute lazily on read, effects run at the end) are the right contract.
- **atomFamily warning → ECS advantage.** Keying atoms by `Entity` and storing them as components on that entity solves the leak problem, because despawn drops them. Any off-entity `HashMap<Entity, _>` cache, including selector maps, needs an `On<Despawn>` cleanup observer.
- **Svelte's overridable deriveds** correspond to an "optimistic local override" component that shadows a memo until its source next changes. That is niche, but useful for UI drag previews.

### Gaps
- Not fetched: Solid stores (`createStore`, `produce`, `reconcile`), Solid 2.0 async design, MobX, Recoil (archived in 2025, background), React hooks rules, Vue `computed`, Svelte `$state`/`$effect`. (background) Solid `createStore` uses proxy-based per-property signals and `reconcile` diffs keyed data. MobX has `autorun`/`reaction`/`computed` and `action` transactions. React's rules of hooks require call-order stability, which bevy_radioactive's `use_*` API inherits if it uses slot indices. Svelte `$effect` runs after DOM updates and `$effect.pre` before.
- I did not confirm whether `createSelector` is documented as O(1). The Solid page makes no Big-O claim.

## ECS-specific reactivity (flecs, Unity DOTS, EnTT; Godot and Arch not researched)

### Takeaway
ECS engines expose structural events (add/remove/set hooks and observers), query-membership monitors, and coarse change versions (chunk- or table-level). None of the engines researched offers a fine-grained derived-value graph. The most transferable ideas are flecs monitors (enter/leave a query), propagation along relationships, yield-existing, and filter terms that match without triggering; EnTT's reactive storage, which accumulates matched entities into a set drained later; and Unity's warning that write access, not actual mutation, bumps versions.

### Cited Findings
**flecs**
- "Observers are similar to systems, in that they are queries that are combined with a callback." The events are OnAdd (only when actually added), OnRemove, OnSet, Wildcard, and custom events via `ecs_emit`. OnSet is not triggered by direct system writes, so the application must call `modified`. — [flecs Observers manual](https://www.flecs.dev/flecs/ObserversManual.html)
- Multi-term observers fire only for entities that match all terms; internally they are built from single-term observers plus a query evaluation on the source. `EcsInOutFilter` terms participate in matching but don't trigger. A `Not` term inverts the event: an OnAdd observer with a `Not` term fires when that component is removed. — [flecs Observers](https://www.flecs.dev/flecs/ObserversManual.html)
- **Monitors** (`EcsMonitor`) fire OnAdd when an entity starts matching a query and OnRemove when it stops. They are costlier because the query is evaluated against both the old and new archetype. — [flecs Observers](https://www.flecs.dev/flecs/ObserversManual.html)
- **Yield existing** runs the observer for entities that already match when it is created (OnAdd/OnSet/OnRemove only), with `EcsObserverYieldOnCreate`/`OnDelete` variants. — [flecs Observers](https://www.flecs.dev/flecs/ObserversManual.html)
- **Propagation:** with `up` traversal, events on a parent propagate to children "until a leaf entity is found, or an entity with the propagated component is found". **Forwarding:** adding a `ChildOf`/`IsA` pair emits OnAdd for components now reachable. OnSet also fires for inherited components when an `IsA` pair is added, when an override is removed, or when a prefab value changes; opt out with `self`. — [flecs Observers](https://www.flecs.dev/flecs/ObserversManual.html)
- Ordering: order between observers is undefined, as is order across entities ("allows batching"). OnSet and custom events are delivered in emission order. Hooks (one per component per event) run before OnAdd/OnSet events and after OnRemove. "Observers should never mutate a component." Fixed-source terms (`.src.id = Game`) cover singletons. — [flecs Observers](https://www.flecs.dev/flecs/ObserversManual.html)

**Unity Entities 1.3**
- "For efficiency, the change filter applies to whole archetype chunks, and not individual entities." The filter "only checks whether a system that declared write access to the component has run", so "always declare read-only access to components that you don't need to modify." The API is `query.SetChangedVersionFilter(typeof(T))`; shared-component filters select by value (at most two). — [Unity change filters](https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/systems-entityquery-filters.html)

**EnTT**
- Per-type sinks: `registry.on_construct<T>()`, `on_update<T>()`, `on_destroy<T>()` with `.connect<&fn>()`, and listener signature `void(registry&, entity)`. Construct listeners fire after creation, destroy listeners before removal. `on_update` fires only through `patch`/`replace`/`emplace_or_replace`, so the change notification is opt-in via the mutation API. — [EnTT wiki: ECS](https://github.com/skypjack/entt/wiki/Entity-Component-System)
- Reactive storage: `entt::reactive_mixin<entt::storage<void>>`, configured as `.on_construct<position>().on_update<velocity>().on_destroy<renderable>()`, collects matched entities for later iteration (`storage.view<..>(exclude<..>)`). "all configurations are in _or_ and never in _and_." "a reactive storage never deletes its entities"; call `clear()`. Destroying it without disconnecting is UB. — [EnTT wiki](https://github.com/skypjack/entt/wiki/Entity-Component-System)

### Inferences
- **Bevy already has most of the flecs primitives**: `On<Add/Insert/Replace/Remove/Despawn>` observers, component hooks, `Changed<T>`/`Added<T>` filters (per-entity ticks, finer than Unity's per-chunk), and entity-event propagation. Missing pieces to consider in the library:
  - (a) **Monitor**: "entity enters/leaves query Q". Implement it with observers on each term's Add/Remove that re-evaluate Q on the entity and toggle a `Matches<Q>` marker. That marker then becomes a reactive source.
  - (b) **Yield-existing** for reactive subscriptions: when an effect or observer is created, run it once for current matches. Solid and Leptos effects do the same by running immediately; Vue does with `immediate`.
  - (c) **Filter terms**: read without subscribing, which is the ECS face of `untracked`.
  - (d) **Up-traversal propagation** for inherited reactive context, so a theme or context change on a parent notifies descendants until one overrides it. This is exactly flecs's "until an entity with the propagated component is found" rule and the natural semantics for `provide_context` on entity hierarchies.
- **Unity's lesson applies to Bevy too**: `Mut<T>` deref-mut marks changed even when the value is unchanged. The library should default to `set_if_neq`-style writes (already present) and make memo equality cut-off the norm, or else the push phase floods.
- **EnTT's opt-in `patch`** is the analogue of Bevy's `Mut` change detection. Its reactive storage, a set accumulated by events and drained by a system, is a good pattern for batching per-frame reactive work. Observers push entity ids into a `Dirty` set, and one system drains it in a deterministic order. That avoids per-event effect execution.
- **Archetypal fit:** structural events (add/remove) are cheap in observers, while value changes are only cheap as ticks scanned per query. A reactive layer should therefore use push (observers, hooks) for structure and explicit `set`, and pull or scan (`Changed<T>`) for bulk component writes made by ordinary systems, bridging the latter into the graph once per frame.

### Gaps
- Godot signals, Arch (C#), and flecs's own change detection (table dirty state, `ecs_query_changed`) and query caching were not fetched. (background) Godot signals are named per-object, connected to Callables, with `CONNECT_DEFERRED`/`ONE_SHOT` flags. flecs tracks change per table column and query (`ecs_query_changed`, `ecs_iter_skip`), which is coarser than Bevy's per-entity ticks.
- I found no evidence that any of the ECS engines examined has a built-in memo/derived graph. This is absence of evidence from the pages read, not a confirmed survey result.

## Techniques worth porting, mapped onto Bevy (entities, components, observers, systems, BSN)

### Takeaway
High-value ports: coloring-based lazy memos with equality cut-off; keyed selectors (entity-keyed via marker components); field-masked stores with `Patch`; effect flush phases plus owner-scoped cleanup; explicit batch for exclusive writes; flecs-style monitors and yield-existing; context via hierarchy up-traversal; and keyed/indexed list reconciliation that reuses BSN-spawned entities. Each has a direct cited precedent above.

### Cited Findings
- Selector: keyed boolean accessor; only keys whose match changes update. — [Solid createSelector](https://docs.solidjs.com/reference/secondary-primitives/create-selector)
- Families keyed by param, with leak caveats and `remove`/`setShouldRemove`. — [Jotai family](https://jotai.org/docs/utilities/family)
- Per-field store notifications: parents and children are notified, siblings are not. `Patch` notifies only changed fields, with a keyed diff for collections. — [reactive_stores](https://docs.rs/reactive_stores/latest/reactive_stores/)
- Effects with cleanup: the returned closure runs before the next run. — [Preact](https://preactjs.com/guide/v10/signals/); `onWatcherCleanup`/`onCleanup` — [Vue](https://vuejs.org/guide/essentials/watchers.html)
- Untracked reads: `untrack`, `untracked`, `.peek()`. — [TC39](https://github.com/tc39/proposal-signals), [Preact](https://preactjs.com/guide/v10/signals/), [Svelte](https://svelte.dev/docs/svelte/$derived)
- Batching: `batch()` commits at the outermost exit. — [Preact](https://preactjs.com/guide/v10/signals/); Leptos defers effects to the next tick — [reactive_graph](https://docs.rs/reactive_graph/latest/reactive_graph/)
- Async: deriveds track reads after `await`. — [Svelte](https://svelte.dev/docs/svelte/$derived); Vue `watchEffect` loses tracking after the first await — [Vue](https://vuejs.org/guide/essentials/watchers.html)
- Lenses and two-way binding: Xilem adapt nodes, from Druid lenses. — [Xilem](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)
- Keyed lists: `<For>` vs `<Index>`. — [Solid](https://docs.solidjs.com/concepts/control-flow/list-rendering)
- Context: provide/inject that "works across any distance". — [Leptos](https://book.leptos.dev/15_global_state.html); propagation along relationships — [flecs](https://www.flecs.dev/flecs/ObserversManual.html)
- Group disposal: `effectScope`. — [alien-signals](https://github.com/stackblitz/alien-signals)

### Inferences (catalogue with Bevy mapping)
| Technique | API shape (Rust sketch) | Algorithmic properties | Bevy mapping / assessment |
|---|---|---|---|
| Lazy memo with coloring | `let m = cx.memo(\|w\| a.get(w) * 2);` | Push marks Check/Dirty, pull recomputes; glitch-free; equality cut-off stops propagation | `ReactiveState` component on memo entity; `Subscribers` relationship as edges. **Strong fit.** Pull needs `&mut World` (exclusive). |
| Selector | `let is_sel = cx.selector(selected); is_sel.get(w, item_entity)` | O(changed keys) notification | `HashMap<K, Vec<Entity>>` on selector entity; or **marker-move pattern** (`Selected` component) using built-in observers. **Strong fit**; marker form is idiomatic. |
| Family keyed by entity | `hp_text.of(entity)` → reactive node stored as component on `entity` | cache per key | Store derived nodes as components or children of the keyed entity; despawn = GC. **Strong fit**, and fixes the Jotai leak by construction. |
| Field-level store | `#[derive(ReactiveFields)] struct Stats {hp, mana}`; `stats.hp().set(w, 3)` | per-path triggers; parent/child notify, sibling isolation | Sidecar `FieldDirty<C>(u64)` bitmask + subscriber field mask; `patch` diffs per field. **Good fit**; avoids splitting components into many tiny ones. |
| Effect + cleanup + flush phase | `cx.effect(Flush::Post, \|w, on_cleanup\| {..})` | deferred, deduped, ordered | Effect entities drained by a system in chosen schedule; cleanups run before rerun and in `On<Remove, Effect>`. **Strong fit.** |
| Untracked / filter term | `sig.get_untracked(w)`; `untracked(\|\| ..)` | no edge recorded | Already present; ECS analogue = query filter terms. |
| Batch | `world.batch(\|w\| { a.set(w,1); b.set(w,2); })` | single commit at outer exit | Needed only for synchronous exclusive writes; Commands already batch. **Medium value.** |
| Monitor (enter/leave query) | `cx.monitor::<(With<A>, Without<B>)>()` → source of `Entity` set | push on structural change | Observers on term Add/Remove re-evaluate entity; toggle `Matches<Q>`. **Strong fit**, fills a Bevy gap. |
| Yield-existing | `.immediate()` / default on effect creation | one initial run | Run once for current matches on spawn. Trivial. |
| Async resource / suspense | `cx.resource(\|w\| key.get(w), \|k\| async move {..})` → `Pending\|Ready(T)\|Err` | tracks the key synchronously, then runs the async part; cancel stale tasks via cleanup | Bevy `Task` stored as component; poll system writes the result signal; cancellation = drop task on key change. Already have `resource`; add stale-cancel + `Suspense`-like "any pending child" aggregate via hierarchy. |
| Two-way binding / lens | `bind(sig, \|s\| &mut s.field)` | write-back through projection | Leptos `Field`/Xilem adapt; for widgets (`TextInput` ↔ signal) needs echo suppression (`set_if_neq`). `bind` exists. |
| Keyed list diff | `for_keyed(items, \|it\| it.id, \|cx, it, idx_sig\| scene!)` | LIS/move minimal; per-key entity reuse | Reorder `Children` instead of respawn; BSN scene per key spawned once. Exists; verify reorder-not-respawn. |
| Show / Switch | `show(cond, \|cx\| scene_a, \|cx\| scene_b)` | memoised condition so toggling only on boolean change | Spawn/despawn subtree, or toggle `Visibility`/`Display::None` for cheap retention (option). |
| Context | `provide_context(entity, Theme)`; `use_context::<Theme>()` walks `ChildOf` up | lookup cost O(depth) unless cached | flecs up-traversal rule: propagate change to descendants until an override. Good fit. |
| Group disposal (owner/scope) | `Region`/`effectScope` | dispose subtree of reactive nodes | Entity hierarchy despawn = owner disposal. Natural. |

### Gaps
- No benchmark data comparing per-entity reactive nodes against side-table graphs in Bevy was found; any perf claims need measurement.

## Poor fits for ECS or Rust ownership, and why

### Takeaway
Ideas built on implicit shared mutable references, proxy interception, or call-order-sensitive hook slots fit Rust and ECS poorly. So do synchronous notify-and-read inside mutations, and unbounded global keyed caches. They clash with exclusive world borrows, runtime-panicking guards, and system parallelism.

### Cited Findings
- Runtime borrow panics: Dioxus `.write()` checks at runtime, and overlapping guards panic. — [Dioxus](https://dioxuslabs.com/learn/0.7/essentials/basics/signals); Leptos guard rule: "you cannot update the value … until this guard is dropped, or it will cause a runtime error". — [Leptos book](https://book.leptos.dev/reactivity/working_with_signals.html)
- Use after dispose: reading a disposed Dioxus signal "will cause a runtime panic". — [Dioxus](https://dioxuslabs.com/learn/0.7/essentials/basics/signals)
- Shared mutable observers are "awkward in Rust"; Xilem uses `&mut` dispatch instead. — [Xilem](https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html)
- Synchronous notification during `set` forbids reads and writes in notify. — [TC39](https://github.com/tc39/proposal-signals); sync watchers have no batching. — [Vue](https://vuejs.org/guide/essentials/watchers.html)
- Unbounded keyed caches leak. — [Jotai](https://jotai.org/docs/utilities/family)
- Coarse or write-access-based change detection causes false positives. — [Unity](https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/systems-entityquery-filters.html)
- Effects that write derived state are "officially discouraged". — [Leptos book](https://book.leptos.dev/reactivity/working_with_signals.html)
- Observers "should never mutate a component". — [flecs](https://www.flecs.dev/flecs/ObserversManual.html)

### Inferences
- **Proxy-based deep stores** (Solid `createStore`, Vue `reactive`, Svelte deep `$state`) need transparent interception of field access. Rust has none, so use derive-generated accessors (the reactive_stores approach) or field bitmasks instead.
- **Global thread-local "current observer" auto-tracking** conflicts with Bevy's parallel executor. Tracking must be explicit through a `Cx`/`&mut World` parameter, or confined to exclusive systems. bevy_radioactive's `Cx` already follows this pattern.
- **Synchronous effects inside observers**, Vue `sync`-style, risk re-entrancy and observer cascades. Push marks synchronously, but defer effects to a system. Keep a sync escape hatch only for cheap, write-free callbacks.
- **Closure-captured `Copy` handles** (Dioxus, Leptos arena) are convenient. In ECS, though, they hide which components a closure touches, which defeats Bevy's access-based scheduling. Effects that do run as systems (`IntoSystem`) get static access metadata and could run in parallel. This is a real design trade-off: closures over `&mut World` are simple but serial.
- **React-style hook slots** (`use_state` by call order) are fragile under conditional logic. Keyed or `Entity`-based identity (BSN names, `keyed`) is more robust and fits ECS's explicit identity.
- **Effects writing signals** (the derived-via-effect antipattern) create extra frames of latency in a deferred-effect ECS design and can oscillate. Prefer memos or derived components.

### Gaps
- No primary source was found that quantifies parallelism lost by closure-based effects in Bevy; this is an inference from Bevy's scheduler design.
