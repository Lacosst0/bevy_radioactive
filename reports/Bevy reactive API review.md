# Fix seven bugs, then unify bevy_radioactive's reactivity

bevy_radioactive v0.4.0 (branch `reactive-api-improvements`, commit `e4e66a4`) ships a sound core with a fragmented surface, and its most urgent problems are bugs rather than missing features. Probe tests found **five confirmed defects and two confirmed misbehaviours** that the existing suite does not exercise: a memory leak through `Arc` cycles in webby `Instance` trees, leaked rows on duplicate keys in `coat` keyed `for`, a per-frame nesting limit tied to `max_passes = 8`, a panic that kills the app when a conditional sibling shifts positional hook slots, subscription entities that pile up after their sources despawn, double delivery of raw `ValueRemoved` subscriptions, and silent truncation when `max_passes` runs out. On top of the bugs sit three overlapping ways to write reactive UI (`bind`/`watch`, `coat!`, webby `#[component]`). They run on two execution models: push for ECS events and pull for polled effects. They have three effect signatures, duplicate entry points, and recurring boilerplate (manual signal cloning, placeholder initial values because `bind` does not replay, `settle`/`flush` choreography). Bevy 0.20 offers primitives the crate barely uses yet: immutable components with `Insert`/`Discard` as an exact "on mutate" signal, self-cleaning observer entities, `Ready`, resources-as-entities, and controlled `ValueChange` widgets. The closest prior art, viridia's bevy_reactor, has already shown how presence-tracked reads, entity-owned effects and two-phase effects work on BSN. The recommended path has four steps: ship the seven fixes as a non-breaking 0.4.1; make 0.5 an ergonomics-and-consolidation release built around one unified effect core and a new `computed` reactive-component primitive; add Bevy-native primitives (widget two-way binding, entity selection, query monitors, async tasks); and only then replace per-frame polling with push-based invalidation.

## Two execution models meet in one exclusive system

The crate has four layers that share one runtime ([codebase audit, Q1](../src/lib.rs)). **`reactivity`** publishes ECS changes as `ValueChanged<T>`/`ValueRemoved<T>` entity events and offers the BSN helpers `watch`, `bind`, `bind_or` and `on_removed`. **`subscription`** routes any `EntityEvent` to listeners through `SubscribedTo<E>` relationship entities, keeping the source ([src/subscription.rs:127-185](../src/subscription.rs)). **`coat`** provides out-of-ECS `Signal`/`Memo` cells plus a `coat!` macro that compiles reactive `if`/`for`/`match` into "region" effects ([src/coat.rs:194-438](../src/coat.rs)). **`webby`** provides React-style `#[component]` functions with hooks that re-render by despawning and respawning their subtree ([src/webby.rs:209-632](../src/webby.rs)).

The first two layers are push-based. A per-type `Changed<T>` publisher triggers events, which are routed through relationship entities to per-listener observers. The last two are pull-based: `run_effects` re-checks every recorded dependency (ECS change ticks and signal versions) of every host on every pass ([src/runtime.rs:138-206](../src/runtime.rs)). The two models meet only inside `run_reactive`, an exclusive `PostUpdate` system. It loops up to `ReactiveSettings::max_passes` (default 8). Each pass adds pending publishers, runs the `ReactivePublish` schedule, flushes, and polls effects, stopping when nothing was published and no effect ran ([src/reactivity.rs:130-146](../src/reactivity.rs)).

The bridge between the models runs one way only. Webby's `Cx` can read ECS state reactively through `Reads::get`/`resource`. `coat!`'s `$x` syntax works only on signals and memos. `coat::effect` throws its tracked `Reads` away (the `|_|` in `tracked(world, target, |_| ..)`). `patch` collects only signal dependencies ([src/coat.rs:463-479](../src/coat.rs), [src/coat.rs:553-569](../src/coat.rs)). Nothing derives a component from other components the way `bind` writes one. So a user who wants "this text shows that resource" picks between `bind` (push, but no replay), webby (pull, with full subtree respawn), or hand-written glue into a signal. This split is the root cause of most API friction below, and the plan unifies it around a single `Reads`-based effect core.

The crate passes its own tests: **37 integration tests across four suites, zero clippy warnings at `pedantic`**. But 9 of 11 doctests are `ignore`d, there are no unit tests in `src/`, and the `comparison_*` examples (gated behind `required-features = ["comparison"]`) are neither compiled nor linted in a default `--all-targets` run ([Cargo.toml:32-55](../Cargo.toml)). Every bug below was invisible to that suite.

## Seven defects hide behind a green test suite

Each defect was reproduced with a throwaway probe test against a scratch copy of the crate. All seven fixes are small and non-breaking.

| # | Defect | Root cause | Probe result |
|---|---|---|---|
| 1 | Webby `Instance` trees leak | `child()` stores `Arc::clone(self)` as the child's parent while the parent keeps the child in `children.current`, an `Arc` cycle ([src/webby.rs:146-179](../src/webby.rs)) | Despawning a `@parent` with a `@leaf` child dropped **0 of 2** `use_ref` values |
| 2 | Duplicate `#[key]` in keyed `for` leaks rows | `next_rows.insert(key, ..)` discards the displaced row, whose entities later become orphaned roots after `replace_children` ([src/coat.rs:620-644](../src/coat.rs)) | `labels before=2 after clearing=1` |
| 3 | Nesting depth limited per frame | `run_effects` snapshots `EffectHosts` at pass start, so hosts spawned mid-pass wait for the next pass, and each level costs one of 8 passes ([src/runtime.rs:158-163](../src/runtime.rs)) | A 13-level chain shows its leaf only on frame 2, so a partial UI is visible for one frame |
| 4 | Conditional sibling panics the app | Child instances are keyed by one positional counter per parent component, so inserting an unkeyed conditional component hands the sibling's state to the newcomer and `hook()` panics on a type mismatch ([src/webby.rs:165-179](../src/webby.rs), [src/webby.rs:244-249](../src/webby.rs)) | Panic "hook #0 changed type to `Signal<bool>`" inside the exclusive system. When the types match, state is silently swapped instead |
| 5 | Subscription entities outlive their sources | `Subscribers<E>` is not `linked_spawn`, and Bevy's non-linked `on_discard` only strips the relationship component ([src/subscription.rs:155-157](../src/subscription.rs), [bevy_ecs relationship/mod.rs](https://docs.rs/bevy_ecs/0.20.0/src/bevy_ecs/relationship/mod.rs.html)) | **5 of 5** subscription entities remained after all sources despawned |
| 6 | Raw value-event subscriptions misdeliver | `subscribe_to::<ValueRemoved<T>>` installs the generic `forward` observer, and `publish_removed` both routes and triggers, so the event is routed twice. `ValueChanged<T>` with untracked `T` delivers nothing and raises no error ([src/subscription.rs:363-372](../src/subscription.rs), [src/reactivity.rs:102-113](../src/reactivity.rs), [src/reactivity.rs:330-337](../src/reactivity.rs)) | 2 deliveries per removal; 0 before tracking |
| 7 | `max_passes` exhaustion is silent | `run_reactive` falls out of its loop without logging ([src/reactivity.rs:131-145](../src/reactivity.rs)) | A self-incrementing effect advanced exactly 8 per frame and logged nothing |

Defects 1, 4 and 5 matter most. Defect 1 frees nothing below the first level of any unmounted subtree: hook state, signals and contexts all stay allocated. Defect 4 converts a common UI pattern, `if open { @panel() }` followed by a sibling component, into a crash, because Bevy 0.20 routes system panics to `FallbackErrorHandler`, which re-panics by default ([Bevy 0.20 news](https://bevy.org/news/bevy-0-20/)). Defect 5 grows without bound for any listener that watches dynamic sources such as `watch(vec_of_entities, ..)`. Each leftover pair also costs two entities, the subscription entity and its `.observe()` observer entity ([src/subscription.rs:294-298](../src/subscription.rs)).

The fix for defect 5 gets help from Bevy. An observer that watches specific entities despawns itself once all of them are gone: the `ObservedBy` despawn hook counts despawned watched entities and calls `try_despawn` on the observer ([bevy_ecs distributed_storage.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/observer/distributed_storage.rs)). Despawning the internal subscription entity is therefore enough; its observer follows.

Beyond the seven, the audit flagged latent hazards worth fixing alongside them. `Signal` wraps a `std::sync::RwLock`, so `s.with(|_| s.set(..))` re-enters the lock and deadlocks or panics. A `Memo` that reads itself recurses without bound. `patch` marks its component changed on every run even when the value is identical, which republishes `ValueChanged<C>` for tracked components ([src/coat.rs:215-270](../src/coat.rs), [src/coat.rs:349-364](../src/coat.rs), [src/coat.rs:561-565](../src/coat.rs)). `Disabled` is handled inconsistently: publishers include `Disabled` sources and `bind` writes to `Disabled` targets, while `run_effects` skips `Disabled` hosts ([src/reactivity.rs:313](../src/reactivity.rs), [src/runtime.rs:168-170](../src/runtime.rs)).

The documentation also states two falsehoods. The README claims "Removal is delivered synchronously, while the source still exists", but delivery is queued through `commands.trigger` and runs after despawn has finished; the tests pass only because they call `flush()` ([README.md:275](../README.md), [src/subscription.rs:423-428](../src/subscription.rs)). The README `on_removed` example writes to every `Text` in the world instead of `q.get_mut(r.target())` ([README.md:129-133](../README.md)). MIGRATION.md still carries the full 0.1→0.2 guide, with removed APIs and the outdated "one source per event type" rule ([MIGRATION.md:33-337](../MIGRATION.md)).

## Three paradigms, three effect shapes and recurring boilerplate

The API friction falls into three groups: overlap, drift and boilerplate.

**Overlap.** Webby can mount a component three ways (`#[component]`, `component(|cx| ..)`, `component_with(deps, ..)`) and key one two ways (`.key(k)`, `keyed(k, mount)`). `Cx::use_component`/`use_resource` are just `cx.get(..).cloned()`/`cx.resource().cloned()` and are reachable on the same `Cx` through `Deref<Target=Reads>`. The current entity has three names (`Reads::target()`, `Cx::entity()`, `Reads::this::<T>()`), while `Subscription::target()` means the *listener* in the subscription layer ([src/webby.rs:216-229](../src/webby.rs), [src/runtime.rs:44-69](../src/runtime.rs), [src/subscription.rs:53-55](../src/subscription.rs)). There are two signal constructors (`signal(|| v)` and `Signal::new(v)`). Webby re-exports `Signal` but not `signal`/`memo`, and the prelude exports none of `Signal`, `coat!`, `component`, `Cx`, `ReactiveSet` or `ReactiveSettings` ([src/prelude.rs:3-6](../src/prelude.rs)).

**Drift.** The three effect APIs have three shapes:

| API | Signature | What it can depend on |
|---|---|---|
| `coat::effect` | `Fn(&mut Commands)` | Signals only |
| `webby::use_effect` | `FnOnce(&D, &mut Commands)` | Explicit deps; no cleanup |
| `coat::shard` | `Fn(&A, &World)` | Explicitly untracked world reads |

([src/coat.rs:463-511](../src/coat.rs), [src/webby.rs:331-353](../src/webby.rs))

`use_reducer(init, reducer)` reverses React's `useReducer(reducer, initialArg)`, even though the docs present it as React parity ([src/webby.rs:280-292](../src/webby.rs)). The family `watch` / `on_removed` / `bind_or` mixes three naming styles, and the `_or` suffix in `bind_or` means "on removal" ([src/reactivity.rs:390-460](../src/reactivity.rs)).

The `if` and `match` regions in `coat!` follow different rules. A reactive `if` is keyed on the branch index, while a reactive `match` is keyed on the scrutinee *value*, so `match $state { Loading(p) => .. }` rebuilds the arm on every progress tick. That contradicts the module doc's claim that a region "re-renders only when the branch changes" ([macros/src/lib.rs:547-588](../macros/src/lib.rs), [macros/src/lib.rs:720-729](../macros/src/lib.rs), [src/coat.rs:24-25](../src/coat.rs)). Plugin requirements also differ: `track()` adds `RadioactivePlugin` automatically, but `watch`/`bind`/`coat`/`webby` fail at scene resolve if it is missing ([src/reactivity.rs:277-306](../src/reactivity.rs)).

**Boilerplate.** The examples and tests repeat five patterns:

- Signals are cloned by hand before every `coat!`, because the macro moves its captures: `let (open, count) = (open.clone(), count.clone());` ([tests/coat.rs:65-66](../tests/coat.rs), [examples/coat_todos.rs:30-31](../examples/coat_todos.rs)).
- Setters are cloned once per handler: `let (minus, plus, reset) = (set_count.clone(), set_count.clone(), set_count);` ([examples/comparison/webby.rs:44](../examples/comparison/webby.rs)).
- The initial value is hand-duplicated next to `bind`, as in `Text("0")`, because `bind` does not replay. That is wrong as soon as the state starts non-zero ([examples/comparison/reactivity.rs:57-60](../examples/comparison/reactivity.rs), [README.md:272-274](../README.md)).
- The only way to drive component state from outside is a `use_effect(())` that inserts a resource holding the setter ([examples/webby_counter.rs:16-18](../examples/webby_counter.rs)).
- `settle(app) { app.update(); world.flush(); }` is redefined in several test files ([tests/reactivity.rs:40-43](../tests/reactivity.rs), [tests/subscription.rs:30](../tests/subscription.rs)).

Inside `coat!`, `$x` always expands to `x.get()`, which clones: `$todos.len()` clones the whole `Vec`, and a keyed `for` clones and re-collects the list on every run ([macros/src/lib.rs:196-224](../macros/src/lib.rs), [macros/src/lib.rs:618-620](../macros/src/lib.rs)). A webby component cannot branch between two `bsn!` scene types without boxing; the probe got E0308 ([examples/comparison/webby.rs:76-82](../examples/comparison/webby.rs)).

**Performance.** Several costs compound:

- `run_effects` allocates a `Vec` of every host and checks every dependency on every pass of every frame, which is O(hosts × deps) even when idle ([src/runtime.rs:158-206](../src/runtime.rs)).
- Host order follows `EntityHashSet` iteration rather than tree depth, so a child can render just before its parent despawns it ([src/runtime.rs:162-164](../src/runtime.rs)).
- `publish::<T>` clones `T` into a fresh `Arc` per changed entity even with no subscribers, and the delivered event is double-boxed (`Arc<ValueChanged<T>{ value: Arc<T> }>`) ([src/reactivity.rs:312-324](../src/reactivity.rs), [src/subscription.rs:416-427](../src/subscription.rs)).
- Webby re-renders despawn and respawn the whole subtree, which loses `Hovered`, focus, scroll and Feathers widget state ([src/webby.rs:543-556](../src/webby.rs)).
- `for_keyed` respawns a row whenever its value changes rather than patching it ([src/coat.rs:622-631](../src/coat.rs)).

The README itself advises keeping components small ([README.md:286-289](../README.md)), which suggests the author already sees the ceiling.

## Bevy 0.20 and the ecosystem already supply the missing pieces

Upstream Bevy has deliberately left reactivity to the ecosystem. Cart's canonical July 2024 position calls fine-grained, observer-style reactivity "the only viable strategy for a general purpose reactive system in the context of Bevy". He wants it to build on change detection, hooks and observers as "an additive layer". He calls top-level field reactivity the easy part and branching, looping and adding or removing children the hard part ([bevy discussion #14437](https://github.com/bevyengine/bevy/discussions/14437)). The 0.20 "What's Next" list covers `.bsn` assets, assets-as-entities and Template-powered required components, not reactivity ([Bevy 0.20 news](https://bevy.org/news/bevy-0-20/)). A library built from Templates, observers and relationships is therefore aligned with upstream and unlikely to be made obsolete soon. The keyed-versus-data-identity debate for lists was never resolved upstream, which leaves design room ([bevy discussion #14437](https://github.com/bevyengine/bevy/discussions/14437)).

### Engine primitives the crate underuses

Several 0.20 features map directly onto gaps.

**Immutable components plus `Insert`/`Discard`.** Bevy has no mutation observer for ordinary components; the docs call a per-`&mut` hook "not practical". But immutable components "are guaranteed to never have an exclusive reference", which turns the `Insert` and `Discard` hooks "into a `OnMutate` hook" ([bevy_ecs component/mod.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/component/mod.rs)). That is an exact, push-based change signal with no polling, and it is the officially documented pattern.

**`Discard`/`Remove` run before the value is removed**, so the old data is still readable ([bevy_ecs lifecycle.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/lifecycle.rs)). `ValueRemoved<T>` could carry the last value instead of just `{source}`.

**`Ready`** is a new entity event. It fires per scene entity after its full spawn, descendants included, bottom-up ([Bevy 0.20 news](https://bevy.org/news/bevy-0-20/)). It is the right hook for replaying initial values and for effects that need the subtree to exist.

**`World::register_tracked_system`** returns a `SystemHandle` whose entity is despawned when the last clone drops, which gives RAII lifetimes for system-backed effects ([bevy_ecs system_registry.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/system/system_registry.rs)).

**Resources are now entities** (`World::resource_entity<R>()`) ([bevy_ecs world/mod.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/world/mod.rs)). That suggests the `Singleton` source could tolerate a resource that appears later instead of failing at resolve time ([src/reactivity.rs:347-362](../src/reactivity.rs)).

**`#[component(summary_tick)]`** lets queries skip a whole unchanged table column ([Bevy 0.20 news](https://bevy.org/news/bevy-0-20/)). That makes "nothing changed this frame" nearly free for a pull-based scheduler, though per-entity cost on busy frames is unchanged.

**Headless widgets are deliberately controlled.** They emit `ValueChange<T> { source, value, is_final }` as a change request, and the app writes the state component (`SliderValue`, which is itself `#[component(immutable)]`, `Checked`, `SelectedTab`). The widget reacts when that component is inserted or replaced, and handlers should stop propagation ("consume what you use") ([bevy_ui_widgets lib.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ui_widgets-0.20.0/src/lib.rs), [slider.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ui_widgets-0.20.0/src/slider.rs)). This is exactly the shape of a two-way binding, and bevy_radioactive has none.

Two caveats constrain the design. Commands queued by observers are applied only after all queued observers run (bevy#19569), so derived-value cascades through Commands happen in waves. Observers cannot be retargeted after spawning (bevy#19587), so dynamic dependency sets belong in library-owned edges rather than in an observer's watched-entity list ([bevy_ecs distributed_storage.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/observer/distributed_storage.rs)).

### Prior art in the Bevy ecosystem

**bevy_reactor** (viridia) is the only signal library rebuilt natively on 0.20 BSN `Template`s ([Cargo.toml](https://github.com/viridia/bevy_reactor/blob/main/Cargo.toml)), and it supplies four directly reusable ideas.

First, its `TrackingScope` records `(Entity, ComponentId, present)`, so inserting or removing a tracked component is a reactive input at no extra cost. The change test is `ct.is_changed(..) || !present`, which falls back to `present` when the component is missing ([tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)). bevy_radioactive's `DepKey::Component` has no presence bit, so `this::<Pressed>()` returning `None` cannot reliably wake an effect when `Pressed` is later added.

Second, effects are two-phase. A tracked deps lens feeds an untracked apply closure that gets `&mut EntityWorldMut`, which separates reads from writes and prevents self-triggering ([effect.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/effect.rs)).

Third, effect satellites are owned through `OwnedBy`/`OwnerOf` with `linked_spawn`, so lifetimes follow the owner ([owner.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/owner.rs)).

Fourth, its convergence loop caps divergence at 32 and offers an optional `TrackingScopeTracing` resource that records which reactions ran ([tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)). bevy_radioactive has the cap but no diagnostics.

viridia's own critique of this design is just as instructive: about three hidden entities per button, a single exclusive non-parallel system, dependencies polled every frame, and a `Cx` that is not idiomatic Bevy ([discussion #21654](https://github.com/bevyengine/bevy/discussions/21654)). bevy_radioactive shares all four costs.

**jonmo** contributes `component_signal`, where a signal producing `Option<C>` inserts or removes `C` on the built entity. That is an elegant "reactive component" shape ([jonmo README](https://github.com/databasedav/jonmo)). It also contributes topological-level processing that keeps downstream nodes from seeing stale upstream values within a frame ([graph.rs](https://github.com/databasedav/jonmo/blob/main/src/graph.rs)). haalka, built on jonmo, is explicitly "eventually consistent", with reactions that may lag a frame or more ([haalka docs](https://docs.rs/crate/haalka/0.5.0)). bevy_radioactive's same-frame convergence is an advantage worth keeping.

**bevy_cobweb** shows trigger tuples (`(resource_mutation::<A>(), entity_insertion::<B>(e))`) and revocable reactors ([REACT.md](https://github.com/UkoeHB/bevy_cobweb/blob/master/src/react/REACT.md)). bevy_radioactive's multi-source `Source` impls already cover the first idea.

### Patterns from outside Bevy

The non-Bevy patterns that port best are these:

- **Push-pull graph coloring with equality cut-off** (Reactively, Solid, Leptos): push marks nodes Check/Dirty, and pull recomputes only when a source actually changed ([Milo, Super-charging fine-grained reactive performance](https://dev.to/modderme123/super-charging-fine-grained-reactive-performance-47ph), [docs.rs reactive_graph](https://docs.rs/reactive_graph/latest/reactive_graph/)).
- **Effects that return a cleanup run before the next run** ([Preact signals guide](https://preactjs.com/guide/v10/signals/)), plus watch options for `immediate` and flush timing (`pre`/`post`/`sync`) ([Vue watchers](https://vuejs.org/guide/essentials/watchers.html)).
- **Keyed boolean selectors** that notify only the keys whose match state flips ([Solid createSelector](https://docs.solidjs.com/reference/secondary-primitives/create-selector)).
- **Query-membership monitors and "yield existing"**: run once for current matches when created ([flecs Observers manual](https://www.flecs.dev/flecs/ObserversManual.html)).
- **The `<For>`/`<Index>` split**: in keyed mode the row is fixed per key and its index changes; in indexed mode the slot is fixed and its item changes ([Solid list rendering](https://docs.solidjs.com/concepts/control-flow/list-rendering)).

The poor fits are just as clear. Runtime-checked guards panic on overlap ([Dioxus signals](https://dioxuslabs.com/learn/0.7/essentials/basics/signals)). Unbounded keyed caches leak unless params are removed explicitly ([Jotai family docs](https://jotai.org/docs/utilities/family)). Writing derived state from effects is "officially discouraged" ([Leptos book](https://book.leptos.dev/reactivity/working_with_signals.html)). Call-order hook slots are fragile under conditional rendering, and bevy_radioactive's defect 4 shows that failure.

## New reactive primitives that fit BSN and ECS

The primitives below are ranked by leverage. Each fills a gap the audit or the ecosystem survey identified, and each maps onto an existing Bevy mechanism rather than inventing a parallel one.

**`computed`: a reactive component (highest leverage).** This is a BSN template that inserts a component computed from tracked reads and rewrites it with `set_if_neq` when any dependency changes. A variant returning `Option<C>` inserts or removes the component, following jonmo's `component_signal` ([jonmo README](https://github.com/databasedav/jonmo)). It subsumes most `bind` uses, gives `coat` users ECS reads, replaces viridia's `insert_dyn` button-styling pattern ([discussion #21654](https://github.com/bevyengine/bevy/discussions/21654)), and reuses the existing `Reads`/`Effects` runtime. It needs presence tracking in `Reads` to handle marker components like `Pressed`.

**Two-way widget binding.** This connects a signal or an ECS component to a headless widget's state component and consumes the widget's `ValueChange<T>`, so Feathers and bevy_ui_widgets become first-class targets ([bevy_ui_widgets lib.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ui_widgets-0.20.0/src/lib.rs)).

**Entity selection by marker move.** This is Solid's `createSelector` in ECS form: moving a `Selected` marker from entity A to entity B wakes exactly two entities' presence-tracked effects, at O(1) cost in list size ([Solid createSelector](https://docs.solidjs.com/reference/secondary-primitives/create-selector)). It also avoids Jotai-style leaks by construction, because the state lives on the keyed entity ([Jotai family docs](https://jotai.org/docs/utilities/family)).

**Query monitors.** Observers on each term's `Add`/`Remove` re-evaluate a filter and toggle a marker component, which then becomes an ordinary reactive source ([flecs Observers manual](https://www.flecs.dev/flecs/ObserversManual.html)).

**Async tasks.** A `use_task(deps, |d| async ..)` hook returns `Pending | Ready(T)` and cancels stale tasks on deps change by dropping them. `#[component]` currently rejects `async` outright ([macros/src/lib.rs:778-783](../macros/src/lib.rs)).

**Lower priority, but cheap:**

- **`Value<T>` props** that accept a constant, a `Signal<T>` or a `Memo<T>`, after bevy_reactor's `IntoSignal` ([signal.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/signal.rs)).
- **Effect flush phases.** Post-layout effects can read `ComputedNode`/`GlobalTransform`, mirroring Vue's `post` flush ([Vue watchers](https://vuejs.org/guide/essentials/watchers.html)).
- **Inherited context over `ChildOf`**, generalizing webby's instance-chain `use_context` to `coat` and `computed`, after bevy_reactor's `get_inherited_component` ([cx.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/cx.rs)).
- **Field-level stores** (`#[derive(ReactiveFields)]` with per-field dirty bits, after Leptos `reactive_stores`). This is deferred because it is large and speculative ([docs.rs reactive_stores](https://docs.rs/reactive_stores/latest/reactive_stores/)).

Two architectural moves are worth planning but not rushing. The first is push-based invalidation. Signals keep subscriber host lists, ECS dependencies become an index keyed by `(Entity, ComponentId)` and are fed by the existing per-type publishers and by `Insert` observers on immutable components, and memos use coloring. This turns per-frame cost from O(all deps) into O(changed). The second is entity-backed `Copy` signals, as in bevy_reactor's `Mutable<T> { cell: Entity, component, marker }` ([mutable.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/mutable.rs)). They would end clone boilerplate and make signals inspectable, but reads would need world access, which collides with `coat!`'s closure-captured `$x` model. That is a 0.6 design question, not a 0.5 task.

## Prioritized implementation plan

The plan has five phases. Phase 0 is a non-breaking 0.4.1 patch. Phases 1 and 2 together form 0.5: Phase 1 is mostly additive, and Phase 2 batches every breaking rename behind `#[deprecated]` aliases so users migrate once. Phase 3 adds new primitives in 0.5 or 0.6. Phase 4 is architectural work for 0.6. All code is a proposal sketch against the current file layout and has not been prototyped. Items marked "verify" depend on Bevy or macro behaviour that the research did not confirm.

| ID | Item | Kind | Breaking | Files |
|---|---|---|---|---|
| P0-1 | Break `Instance` `Arc` cycle | Bug | No | `src/webby.rs` |
| P0-2 | Handle duplicate keys in `for_keyed` | Bug | No | `src/coat.rs` |
| P0-3 | Drain hosts spawned mid-pass, parent-first | Bug | No | `src/runtime.rs` |
| P0-4 | Reset instead of panic on hook type change | Bug | No (behaviour) | `src/webby.rs` |
| P0-5 | Despawn orphaned subscription entities | Bug | No | `src/subscription.rs` |
| P0-6 | Warn on `max_passes` exhaustion | Bug | No | `src/reactivity.rs` |
| P0-7 | Fix raw `ValueChanged`/`ValueRemoved` subscriptions | Bug | No | `src/subscription.rs`, `src/reactivity.rs` |
| P0-8 | Signal re-entrancy, `patch` change-guard, `Disabled` consistency | Bug | No | `src/coat.rs`, `src/runtime.rs`, `src/reactivity.rs` |
| P0-9 | Docs fixes, probes as regression tests, CI feature matrix | Hygiene | No | `README.md`, `MIGRATION.md`, `tests/`, CI |
| P1-1 | `bind` replays current value | API | Behaviour (opt-out) | `src/reactivity.rs` |
| P1-2 | Presence-tracked reads | API | No | `src/runtime.rs` |
| P1-3 | `computed` / `computed_opt` reactive component | New primitive | No | `src/reactivity.rs` or new `src/computed.rs` |
| P1-4 | One effect core with cleanup and `on_unmount` | API | Yes | `src/runtime.rs`, `src/coat.rs`, `src/webby.rs` |
| P1-5 | `coat!` borrows captures; `$` reads without cloning | API | No | `macros/src/lib.rs` |
| P1-6 | `match` regions keyed by arm | API | Behaviour | `macros/src/lib.rs` |
| P1-7 | Skip publishing with no listeners; single `Arc` | Perf | No | `src/reactivity.rs`, `src/subscription.rs` |
| P1-8 | `testing::settle`, diagnostics resource | API | No | new `src/testing.rs`, `src/reactivity.rs` |
| P2-1 | Naming, prelude, duplicate removal | Consolidation | Yes (deprecations) | `src/reactivity.rs`, `src/prelude.rs`, `src/webby.rs` |
| P2-2 | Call-site keys for unkeyed components | API | Behaviour | `macros/src/lib.rs`, `src/webby.rs` |
| P2-3 | `#[component]` entity props, `Result` renders, `use_reducer` order | API | Yes | `macros/src/lib.rs`, `src/webby.rs` |
| P3-1 | Two-way widget binding | New primitive | No | new `src/widgets.rs` (feature `ui`) |
| P3-2 | Entity selection (marker move) | New primitive | No | new `src/select.rs` |
| P3-3 | Query monitors | New primitive | No | new `src/monitor.rs` |
| P3-4 | `use_task` async hook | New primitive | No | `src/webby.rs`, `src/runtime.rs` |
| P3-5 | Keyed rows receive an item signal | API/perf | Behaviour | `src/coat.rs`, `macros/src/lib.rs` |
| P3-6 | `Value<T>` props, flush phases, inherited context | New primitive | No | `src/coat.rs`, `src/runtime.rs` |
| P4-1 | Push-based invalidation with coloring | Architecture | Internal | `src/runtime.rs`, `src/coat.rs` |
| P4-2 | Webby child retention / props memo | Architecture | Behaviour | `src/webby.rs` |
| P4-3 | Explore entity-backed `Copy` signals | Research | Yes | `src/coat.rs` |

### Phase 0: ship the seven fixes as 0.4.1

**P0-1. Break the `Instance` cycle** ([src/webby.rs:146-179](../src/webby.rs)). Make the parent link weak. The host's `Effects` slot keeps each instance alive, so a parent dies when its host despawns, and its children map drops with it. Also clear `children` when a host's `Effects` is removed.

```rust
// before
struct Instance {
    parent: Option<Arc<Instance>>,
    children: Mutex<ChildSlots>, // current: HashMap<_, Arc<Instance>>
    /* hooks, contexts, ... */
}
fn child(self: &Arc<Self>, key: ChildKey) -> Arc<Instance> {
    /* ... */ Instance { parent: Some(Arc::clone(self)), /* ... */ }
}

// after
struct Instance {
    parent: Option<Weak<Instance>>,
    children: Mutex<ChildSlots>,
    /* ... */
}
fn child(self: &Arc<Self>, key: ChildKey) -> Arc<Instance> {
    /* ... */ Instance { parent: Some(Arc::downgrade(self)), /* ... */ }
}
// use_context walks with upgrade:
let mut cur = self.instance.parent.as_ref().and_then(Weak::upgrade);
while let Some(inst) = cur {
    if let Some(v) = inst.context::<T>() { return Some(v); }
    cur = inst.parent.as_ref().and_then(Weak::upgrade);
}
```

Regression test: port `probe_instance_tree_freed_on_despawn`. Hold a `use_ref` value that has a `Drop` counter in a 2-level tree, despawn the root, and assert 2 drops.

**P0-2. Duplicate keys in `for_keyed`** ([src/coat.rs:620-644](../src/coat.rs)). Despawn the displaced row and warn once. Silently de-duplicating hides user bugs, and panicking repeats defect 4.

```rust
// before
next_rows.insert(item_key, (item, children));

// after
if let Some((_, dup)) = next_rows.insert(item_key, (item, children)) {
    warn_once!("coat!: duplicate #[key] in reactive `for`; earlier row replaced");
    next_order.retain(|e| !dup.contains(e));
    for e in dup { if let Ok(ent) = world.get_entity_mut(e) { ent.despawn(); } }
}
```

Regression test: render `[a, a]`, then `[]`, and assert zero labels remain and no orphaned roots exist.

**P0-3. Drain hosts spawned during a pass, parent-first** ([src/runtime.rs:143-206](../src/runtime.rs)). This removes the depth ≈ `max_passes` ceiling and the one-frame partial UI. It also stops wasted child renders caused by `EntityHashSet` order.

```rust
// before
let hosts: Vec<Entity> = world.resource::<EffectHosts>().0.iter().copied().collect();
for host in hosts { /* poll deps, run dirty slots */ }

// after
#[derive(Resource, Default)]
pub(crate) struct EffectHosts { all: EntityHashSet, fresh: Vec<Entity> }
// register_host hook: hosts.all.insert(e); hosts.fresh.push(e);

pub(crate) fn run_effects(world: &mut World) -> bool {
    world.resource_mut::<EffectHosts>().fresh.clear();
    let mut queue: Vec<Entity> = world.resource::<EffectHosts>().all.iter().copied().collect();
    sort_parent_first(world, &mut queue); // by ChildOf depth
    let mut ran = false;
    let mut i = 0;
    while i < queue.len() {
        let host = queue[i];
        i += 1;
        if world.get_entity(host).is_err() { continue; } // despawned by an ancestor's re-render
        ran |= poll_host(world, host);
        let fresh = std::mem::take(&mut world.resource_mut::<EffectHosts>().fresh);
        queue.extend(fresh); // children rendered in the same pass
    }
    ran
}
```

Each `max_passes` iteration now settles a whole tree depth, so passes count feedback rounds rather than nesting levels. Regression test: port `probe_nesting_depth_per_frame` (a 13-level chain) and assert the leaf exists after one `app.update()`.

**P0-4. Reset instead of panic on a hook type change** ([src/webby.rs:231-253](../src/webby.rs)). This is the safety net. P2-2 removes most causes.

```rust
// before
let hook = slot.downcast_mut::<H>()
    .unwrap_or_else(|| panic!("hook #{idx} changed type to `{}`", type_name::<H>()));

// after
if !slot.is::<H>() {
    warn!(
        "webby: hook #{idx} in `{}` changed type to `{}`; resetting component state. \
         Give conditionally rendered sibling components a `.key(..)`.",
        self.name, type_name::<H>()
    );
    self.instance.reset(); // drop all hook slots and child instances, then re-init
    *slot = Box::new(init());
}
let hook = slot.downcast_mut::<H>().expect("just initialised");
```

Regression test: port `probe_positional_hook_state_shift` and assert no panic and a single warning. Add a second case where both hooks are `Signal<i32>` to document the remaining silent swap until P2-2 lands.

**P0-5. Despawn orphaned subscription entities** ([src/subscription.rs:127-157](../src/subscription.rs)). Add a removal hook that despawns only crate-internal subscription entities, those that also carry `SubscriptionOf`, so direct users of `SubscribedTo` keep their entities. The observer entity then cleans itself up through `ObservedBy` ([bevy_ecs distributed_storage.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/observer/distributed_storage.rs)).

```rust
// after
#[derive(Component)]
#[relationship(relationship_target = Subscribers<E>)]
#[component(on_remove = despawn_internal_subscription)] // verify: coexists with relationship-derived hooks
pub struct SubscribedTo<E: EntityEvent> { /* ... */ }

fn despawn_internal_subscription(mut world: DeferredWorld, ctx: HookContext) {
    if world.get::<SubscriptionOf>(ctx.entity).is_some() {
        world.commands().entity(ctx.entity).try_despawn();
    }
}
```

Verify that the `Relationship` derive accepts an extra `on_remove` hook. It generates `on_insert`/`on_discard`, and `on_remove` should be free, but this is unconfirmed. If not, make `Subscribers<E>` `linked_spawn` and document that despawning a source also despawns direct `SubscribedTo` holders. Regression test: port `probe_subscription_entities_after_source_despawn` and assert 0 of 5 remain, and that observer entities are gone too.

**P0-6. Warn on `max_passes` exhaustion** ([src/reactivity.rs:130-146](../src/reactivity.rs)).

```rust
// after
let mut settled = false;
for _ in 0..max_passes {
    /* add pending publishers, run ReactivePublish, flush */
    let ran = runtime::run_effects(world);
    if !published && !ran { settled = true; break; }
}
if !settled {
    warn_once!(
        "bevy_radioactive: hit ReactiveSettings::max_passes = {max_passes}; \
         remaining reactive work deferred to next frame (possible feedback cycle)"
    );
    world.resource_mut::<ReactiveDiagnostics>().exhausted_frames += 1;
}
```

**P0-7. Fix raw value-event subscriptions** ([src/subscription.rs:363-372](../src/subscription.rs), [src/reactivity.rs:102-113](../src/reactivity.rs)). Mark the crate's own events so `install_subscription` delegates to `register::<T>` instead of installing the generic `forward` observer. Subscribing to `ValueChanged<T>` then also tracks `T` instead of failing silently.

```rust
// after
pub trait RoutedEvent: EntityEvent { fn ensure_routed(world: &mut World); }
impl<T: Component + Clone> RoutedEvent for ValueChanged<T> { fn ensure_routed(w: &mut World) { register::<T>(w) } }
impl<T: Component + Clone> RoutedEvent for ValueRemoved<T> { fn ensure_routed(w: &mut World) { register::<T>(w) } }

// in install_subscription::<E>: specialise via a registry lookup keyed by TypeId,
// populated by `register::<T>`; skip `forward::<E>` when E is a RoutedEvent.
```

Stable Rust has no specialization, so in practice the dispatch is a runtime `TypeId` check against a set filled by `register::<T>`, plus a guard in `register::<T>` that despawns any `forward::<ValueRemoved<T>>` observer installed earlier. Regression test: port `probe_raw_subscribe_to_value_events` and assert exactly 1 delivery per removal and 1 per change without calling `track`.

**P0-8. Latent hazards.**

- Make `patch` compare before writing so tracked components are not republished. Require `C: PartialEq` for macro-generated patches and write through `set_if_neq` ([src/coat.rs:553-569](../src/coat.rs)).
- In `Signal::set`/`update`, document the re-entrancy rule now. Long term, swap `std::sync::RwLock` for a lock that detects same-thread re-entry and returns an error, or take the value out, run the closure, and put it back ([src/coat.rs:215-270](../src/coat.rs)).
- Give `Memo::refresh` a "computing" flag that reports a cycle instead of recursing ([src/coat.rs:349-364](../src/coat.rs)).
- Pick one `Disabled` policy. Recommended: skip `Disabled` sources in publishers and `Disabled` targets in `bind`, which matches `run_effects` ([src/reactivity.rs:313](../src/reactivity.rs), [src/reactivity.rs:427](../src/reactivity.rs)).

**P0-9. Hygiene.**

- Rewrite README line 275 to say removal is routed while the source exists but delivered at the next command flush.
- Fix the `on_removed` example to use `q.get_mut(r.target())`.
- Move the 0.1→0.2 guide out of MIGRATION.md.
- Adopt all seven probes as regression tests.
- Add CI jobs for `cargo clippy --all-targets --features comparison` and `cargo test --no-default-features`, since the non-`ui` path changes layout behaviour ([src/coat.rs:517-520](../src/coat.rs)).
- Turn the `ignore`d doctests into `no_run` where they compile.

### Phase 1: 0.5 ergonomics around one effect core

**P1-1. `bind` replays the current value** ([src/reactivity.rs:386-490](../src/reactivity.rs)). `bind` exists to keep a target in sync with state, so replaying should be the default, like Vue's `immediate` or flecs's yield-existing ([Vue watchers](https://vuejs.org/guide/essentials/watchers.html), [flecs Observers manual](https://www.flecs.dev/flecs/ObserversManual.html)). `watch` keeps event semantics and gains an opt-in `.immediate()`. The replay must run after the whole scene entity is built. BSN applies templates in patch order, so `bind(..)` can appear before the `Text(..)` it writes, and an `EntityTemplate` source may be a sibling that is not built yet. Queue the replay as a command from `build_template` (or run it on `Ready`).

```rust
// before (examples/comparison/reactivity.rs:57-60)
Text("0")
bind(Singleton, |c: &Count, t: &mut Text| **t = c.0.to_string())

// after
Text::default()
bind(Singleton, |c: &Count, t: &mut Text| **t = c.0.to_string())          // replays on spawn
bind(Singleton, |c: &Count, t: &mut Text| **t = c.0.to_string()).deferred() // old behaviour
watch(#Player, |r: Reaction<Health>| { /* ... */ }).immediate()              // opt-in for watch

// inside Bind::build_template, after resolving `sources` and `target`:
let (write, sources) = (self.write.clone(), sources.clone());
ctx.entity.world_scope(|w| w.commands().queue(move |w: &mut World| {
    for src in sources {
        let Some(v) = w.get::<T>(src).cloned() else { continue };
        if let Some(mut c) = w.get_mut::<C>(target) { write(&v, &mut c); }
    }
}));
```

**P1-2. Presence-tracked reads** ([src/runtime.rs:27-99](../src/runtime.rs)). Record whether a component was present when read, following bevy_reactor ([tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)). Every later primitive depends on this.

```rust
// before
enum DepKey { Component(Entity, ComponentId), Resource(ComponentId), Unregistered, Signal(SignalDep) }

// after
enum DepKey { Component { entity: Entity, id: ComponentId, present: bool }, /* ... */ }
fn is_dirty(&self, world: &World, since: Tick, now: Tick) -> bool {
    match *self {
        DepKey::Component { entity, id, present } => match world.get_entity(entity) {
            Ok(e) => match e.get_change_ticks_by_id(id) {
                Some(t) => !present || t.is_changed(since, now),
                None => present, // removed since last run
            },
            Err(_) => present, // entity despawned
        },
        /* ... */
    }
}
```

**P1-3. `computed` / `computed_opt` reactive component.** This new primitive lives in `src/reactivity.rs` or a new `src/computed.rs`. It is an Effects slot on the target that runs a tracked `Fn(&Reads) -> C` and writes the result with `set_if_neq`. Reads of `C` on the same entity are not tracked, which prevents self-triggering. This is bevy_reactor's two-phase split, folded into one closure ([effect.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/effect.rs)).

```rust
pub fn computed<C, F>(f: F) -> impl Template<Output = EmptySceneEffect>
where C: Component<Mutability = Mutable> + PartialEq, F: Fn(&Reads) -> C + Clone + Send + Sync + 'static;

pub fn computed_opt<C, F>(f: F) -> impl Template<Output = EmptySceneEffect>
where C: Component + PartialEq, F: Fn(&Reads) -> Option<C> + Clone + Send + Sync + 'static;
// Some(c) -> insert/replace (works for immutable components too), None -> remove
```

Before, styling a button by interaction state took either a webby component (respawn on every hover) or a hand-written `Changed<Hovered>` system. Showing a count took `bind` plus a placeholder:

```rust
// before
bsn! {
    Button
    Text("0")
    bind(Singleton, |c: &Count, t: &mut Text| **t = c.0.to_string())
}
// + a separate system: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), Changed<Hovered>> ...

// after
bsn! {
    Button
    computed(|r: &Reads| {
        let pressed = r.this::<Pressed>().is_some();        // presence-tracked (P1-2)
        let hovered = r.this::<Hovered>().is_some_and(|h| h.0);
        BackgroundColor(match (pressed, hovered) { (true, _) => PRESSED, (_, true) => HOVER, _ => IDLE })
    })
    computed(|r: &Reads| Text(r.resource::<Count>().map_or(0, |c| c.0).to_string()))
    computed_opt(|r: &Reads| r.this::<Locked>().map(|_| InteractionDisabled))
}
```

`computed` also gives `coat!` ECS reads with no new macro syntax: `coat! { Node computed(|r| ..) }` passes through as an ordinary template. Regression tests should cover initial value on spawn, update on a source change, insert and remove on presence flips, no rewrite when equal, and no self-trigger.

**P1-4. One effect core with cleanup** ([src/runtime.rs:129](../src/runtime.rs), [src/coat.rs:463-479](../src/coat.rs), [src/webby.rs:331-353](../src/webby.rs)). Every effect gets tracked `Reads` and may return a cleanup. `Slot` gains `cleanup: Option<Box<dyn FnOnce(&mut Commands) + Send + Sync>>`, which runs before each re-run and from the `Effects` `on_remove` hook. This follows Preact's return-a-cleanup contract ([Preact signals guide](https://preactjs.com/guide/v10/signals/)).

```rust
pub trait IntoCleanup { fn into_cleanup(self) -> Option<Cleanup>; }
impl IntoCleanup for () { fn into_cleanup(self) -> Option<Cleanup> { None } }
impl<F: FnOnce(&mut Commands) + Send + Sync + 'static> IntoCleanup for F { /* Some(Box::new(self)) */ }

// before
effect(move |cmds| { let _ = count.get(); /* signals only */ });
cx.use_effect(deps, |d, cmds| { /* no cleanup possible */ });

// after
effect(move |r: &Reads, cmds: &mut Commands| {
    let hp = r.get::<Health>(player);                    // ECS reads now tracked
    let e = cmds.spawn(Tooltip::new(hp)).id();
    move |cmds: &mut Commands| { cmds.entity(e).try_despawn(); }
});
cx.use_effect(deps, |d, cmds| { /* ... */ move |cmds: &mut Commands| { /* cleanup */ } });
cx.on_unmount(|cmds| { /* ... */ });
```

`shard(args, Fn(&A, &World))` becomes a deprecated alias over `effect` with `r.untracked()`. Change `use_effect` to run after the host's child hosts have rendered, which the P0-3 queue makes possible by deferring effect slots to the end of the host's subtree. Today effects see empty child hosts ([src/webby.rs:553-565](../src/webby.rs)).

**P1-5. `coat!` borrows captures; `$` reads avoid clones** ([macros/src/lib.rs:23-91](../macros/src/lib.rs), [macros/src/lib.rs:196-224](../macros/src/lib.rs)). The macro already hoists per-site `__coat_N_x = x.clone()` clones. Emitting them from `&x` instead of moving `x` lets callers keep using their signals after the macro.

```rust
// before (tests/coat.rs:65-80)
let scene = { let (open, count) = (open.clone(), count.clone()); coat! { /* ... */ } };
coat! { Text($(format!("{}", $todos.len()))) }   // clones the whole Vec

// after
let scene = coat! { /* ... */ };                    // `open`, `count` still usable afterwards
coat! { Text($(format!("{}", $todos.len()))) }   // expands to todos.with(|todos| todos.len())
```

Expand `$x.method(args)` to `x.with(|x| x.method(args))` when the call is not borrowed out. Add an explicit `$&x` meaning `x.with(..)` for other cases. Have keyed `for` iterate inside `with`, collecting only keys and the row values it needs ([macros/src/lib.rs:618-620](../macros/src/lib.rs)). Verify against `bsn!`'s internal `move` closures in `bevy_scene_macros`, which the audit did not inspect.

**P1-6. Key `match` regions by arm** ([macros/src/lib.rs:720-729](../macros/src/lib.rs)). Generate a selector closure that maps the scrutinee to an arm index and drives the region key. Arm bindings used inside `$`-patches get a projection closure that re-reads them from the scrutinee, so the arm patches in place instead of rebuilding.

```rust
// before: region keyed on the whole value -> Loading(0.1) -> Loading(0.2) rebuilds the arm
coat! { match $state { Load::Loading(p) => Text($(format!("{p:.0}%"))), Load::Done(v) => @view(v) } }

// after (generated, sketch)
__coat_region(
    { let s = state.clone(); move || s.with(|s| match s { Load::Loading(_) => 0usize, Load::Done(_) => 1 }) },
    |arm| match arm {
        0 => /* Text patched from: state.with(|s| match s { Load::Loading(p) => format!("{p:.0}%"), _ => unreachable!() }) */,
        1 => /* @view(...) rebuilt only when entering arm 1 */,
        _ => unreachable!(),
    },
)
```

If projections prove too intricate for the token-level macro, the fallback is to document value-keyed semantics and fix the misleading module doc at [src/coat.rs:24-25](../src/coat.rs).

**P1-7. Publish less** ([src/reactivity.rs:312-324](../src/reactivity.rs)). Query `Has<Subscribers<ValueChanged<T>>>` and skip the clone/`Arc` when no subscriber and no global observer exists. `observe_value` already records global interest; users of a raw `add_observer(On<ValueChanged<T>>)` call `track_global::<T>()`. Collapse the double box by storing `ValueChanged<T>` inline in `SubscriptionEvent` (`event: E` behind one `Arc`) ([src/subscription.rs:416-427](../src/subscription.rs)).

**P1-8. Testing and diagnostics.** Add a `testing` module with `pub fn settle(app: &mut App) { app.update(); app.world_mut().flush(); }` and `settle_n`. Also add a `ReactiveDiagnostics` resource (passes per frame, effects run, publications, exhausted frames). Diagnostics are behind a feature, after bevy_reactor's `TrackingScopeTracing` ([tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)). Export `ReactiveSet` and add a `ReactiveSet::Settled` marker after `Publish` so user systems can order after reactive convergence.

### Phase 2: 0.5 breaking consolidation, with deprecation aliases

**P2-1. Naming, prelude, duplicates.**

```rust
// before                                   // after (old names #[deprecated] for one release)
on_removed(#Src, |r: Removal<Hp>| ..)       watch_removed(#Src, |r: Removal<Hp>| ..)
bind_or(#Src, write, |c| ..)                bind_or_else(#Src, write, |c| ..)
keyed(k, @row())  /  @row().key(k)          @row().key(k)            // drop `keyed`
cx.entity() / reads.target()                cx.entity()              // Reads::target -> crate-private
cx.use_component::<C>(e)                    cx.get::<C>(e).cloned()  // keep alias, document as such
use bevy_radioactive::prelude::*;           // now also: ReactiveSet, ReactiveSettings, Signal,
                                            // signal, memo, coat, computed, computed_opt
```

Add `prelude::webby` and `prelude::coat` sub-preludes so each layer brings in its constructors. Either rename `Singleton` to `ResourceSource`, or keep the name and document it. In either case, make it tolerate a resource that appears after resolve by deferring resolution to the resource's entity once it is created ([src/reactivity.rs:339-382](../src/reactivity.rs)). Make every BSN entry point add `RadioactivePlugin` lazily, as `track()` does, instead of erroring at resolve.

**P2-2. Call-site keys for unkeyed components** ([src/webby.rs:165-179](../src/webby.rs)). Key children by `(call-site Location, occurrence index)` instead of one counter per parent. The `#[component]` expansion captures the location with `#[track_caller]`, the same technique bevy_immediate uses for its `ui.tch()` ids ([bevy_immediate README](https://github.com/PPakalns/bevy_immediate)). Inserting a conditional sibling then no longer shifts anyone else's state.

```rust
// before: ChildKey::Position(n)   where n counts every unkeyed child in the subtree
// after:  ChildKey::Site { loc: &'static Location<'static>, nth: u32 } | ChildKey::User(u64)
#[track_caller]
pub fn mount<D, F>(deps: D, render: F) -> Mount<D, F> { Mount { site: Location::caller(), /* ... */ } }
```

Calls inside loops still collide on `loc`. `nth` disambiguates them positionally, and the P0-4 warning tells users to add `.key()`.

**P2-3. Macro and webby API cleanups.**

- Swap to `use_reducer(reducer, init)` to match React ([src/webby.rs:280-292](../src/webby.rs)).
- Replace last-path-segment `Entity` detection with an explicit `#[entity]` prop attribute, so `Option<Entity>`, `Vec<Entity>` and aliases accept `#Name` ([macros/src/lib.rs:810-821](../macros/src/lib.rs)), and delete the dead `let _ = uses_cx;`.
- Allow `-> Result<impl Scene>` renders, routing errors to Bevy's error handler with the component name as context ([src/webby.rs:614-632](../src/webby.rs)).
- Add a boxed-scene helper (`either(cond, || bsn!{..}, || bsn!{..})`) so branches of different scene types compile without the `.then(|| bsn_list!{..})` workaround.

### Phase 3: Bevy-native primitives

**P3-1. Two-way widget binding** (new `src/widgets.rs`, `ui` feature). One trait describes each controlled widget. The binding writes the state component when the source changes and consumes `ValueChange<T>`. It must not be combined with the widget's `*_self_update` observer, or state is written twice ([bevy_ui_widgets lib.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ui_widgets-0.20.0/src/lib.rs)).

```rust
pub trait ControlledWidget: Send + Sync + 'static {
    type Value: Clone + PartialEq + Send + Sync + 'static;
    fn write_state(entity: &mut EntityCommands, v: &Self::Value);  // e.g. insert(SliderValue(v))
}
impl ControlledWidget for Slider   { type Value = f32;            /* insert SliderValue(v) */ }
impl ControlledWidget for Checkbox { type Value = bool;           /* insert/remove Checked */ }
impl ControlledWidget for TabList  { type Value = Option<Entity>; /* insert SelectedTab(v) */ }

// before: hand-written observer + manual SliderValue sync + remembering propagate(false)
// after
bsn! { @FeathersSlider { /* ... */ } bind_widget::<Slider>(volume) }               // Signal<f32>
bsn! { @FeathersSlider { /* ... */ }
       bind_widget_to::<Slider, Volume>(ResourceSource, |v| v.0, |v, x| v.0 = x)    // ECS state
           .commit_on_final() }                                                     // only is_final
```

Echo suppression comes from `set_if_neq` on both sides. Because `SliderValue` is immutable, external observers of the widget can use `On<Insert<SliderValue>>` push-style ([slider.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ui_widgets-0.20.0/src/slider.rs)).

**P3-2. Entity selection by marker move** (new `src/select.rs`). Selecting moves a marker. Combined with P1-2 presence tracking, only the old and new entity's `computed`/effects wake.

```rust
#[derive(Component)] pub struct Selection<M: Component + Default> { current: Option<Entity>, _m: PhantomData<M> }
pub trait SelectCommandsExt { fn select<M: Component + Default>(&mut self, owner: Entity, target: Option<Entity>); }
// impl: remove M from old current, insert M::default() on target, update `current`.

// usage inside a list row
bsn! { Button
       on(move |_: On<Activate>, mut c: Commands| c.select::<Selected>(list, Some(row)))
       computed(|r: &Reads| BorderColor::all(if r.this::<Selected>().is_some() { ACCENT } else { NONE })) }
```

Because the state lives on the selected entity, despawn cleans it up, which avoids the keyed-cache leak Jotai warns about ([Jotai family docs](https://jotai.org/docs/utilities/family)). Add an `On<Despawn>` guard that clears `current` when the selected entity dies.

**P3-3. Query monitors** (new `src/monitor.rs`). A monitor maintains a marker component for "entity matches this spec" from `Add`/`Remove` observers on the named components. The marker then works with all existing machinery: `watch`, `computed`, `track`.

```rust
app.add_monitor::<Alive>(Monitor::new().with::<Health>().without::<Dead>());
// observers on Add<Health>, Remove<Health>, Add<Dead>, Remove<Dead> queue a re-evaluation
// command (Remove observers run before removal, so evaluate after the flush), then insert/remove Alive.
bsn! { computed(|r: &Reads| Text(format!("{} alive", r.count::<Alive>()))) } // r.count: tracked, P4
```

`Reads::count::<M>()` needs a global dependency on `M`'s presence set. Ship the marker first and the count later.

**P3-4. `use_task` async hook** ([macros/src/lib.rs:778-783](../macros/src/lib.rs)).

```rust
pub enum TaskState<T> { Pending, Ready(T) }
let user = cx.use_task(user_id, |id| async move { load_user(id).await });
match user { TaskState::Pending => bsn!{ Text("Loading...") }.boxed(), TaskState::Ready(u) => @card(u).boxed() }
```

The hook spawns on `AsyncComputeTaskPool` and stores the `Task` in the hook slot. A completion check in `run_effects` marks the host dirty. A deps change drops the old `Task`, which cancels it, and dropping the hook slot on unmount does the same.

**P3-5. Keyed rows receive an item signal** ([src/coat.rs:622-631](../src/coat.rs)). Keyed rows persist per key and receive an item signal updated with `set_if_neq`, so toggling one todo's `done` patches that row instead of respawning it. Indexed rows keep slot identity and also receive an item signal, mirroring Solid's `<For>`/`<Index>` split ([Solid list rendering](https://docs.solidjs.com/concepts/control-flow/list-rendering)). Fix `spawn_list` to stop inserting `ChildOf` per entity before `replace_children` reorders a second time ([src/coat.rs:524-543](../src/coat.rs)).

**P3-6. Smaller primitives.**

- `Value<T>` props: `enum Value<T> { Const(T), Signal(Signal<T>), Memo(Memo<T>) }`, with `From` impls, and `Value::with(&self, &Reads, f)` that tracks when reactive ([signal.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/signal.rs)).
- `EffectFlush::{Pre, PostLayout}`: a second drain system after UI layout and transform propagation, for effects reading `ComputedNode`/`GlobalTransform` ([Vue watchers](https://vuejs.org/guide/essentials/watchers.html)).
- `Reads::inherited::<C>()`: walks `ChildOf` to the nearest ancestor with `C`, tracking each ancestor's presence and the provider's tick. It stops at the first override, following flecs's up-traversal rule ([flecs Observers manual](https://www.flecs.dev/flecs/ObserversManual.html), [cx.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/cx.rs)).

### Phase 4: architecture for 0.6

**P4-1. Push-based invalidation.**

- Signals keep `subscribers: Mutex<SmallVec<[Entity; 4]>>` of host entities, and `notify()` pushes those hosts onto a dirty queue (a `Mutex<Vec<Entity>>` shared through the runtime resource, since signals are `Send + Sync`).
- ECS dependencies go into a `HashMap<(Entity, ComponentId), SmallVec<Host>>` index. The per-type `Changed<T>` publishers that `track::<T>()` already installs feed it, as do `Insert`/`Discard` observers for immutable components, the documented "on mutate" pattern ([bevy_ecs component/mod.rs](file:///C:/Users/Computer/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/bevy_ecs-0.20.0/src/component/mod.rs)). Untracked component types fall back to polling only the hosts that read them.
- Memos adopt Reactively-style coloring: push marks Check/Dirty, and pull recomputes only if a source changed ([Milo](https://dev.to/modderme123/super-charging-fine-grained-reactive-performance-47ph)).

`run_effects` then drains only dirty hosts in parent-first order, so idle frames cost close to zero. No benchmark exists for the current polling cost, so measure before and after with a 1k-host scene.

**P4-2. Webby child retention.** Full reconciliation of webby renders is large. The cheaper step that captures most of the benefit is to keep a child component's host entity alive across a parent re-render when its key and `PartialEq` props are unchanged (`React.memo` semantics). Only plain entities are respawned. Recommend `coat` plus `computed` as the primary path for large UIs, and say so in the README.

**P4-3. Explore entity-backed `Copy` signals.** Prototype `Mutable<T> { cell: Entity, .. }` with the value in an immutable component on an owned satellite entity (`OwnedBy` with `linked_spawn`). Writes re-insert the component and fire `Insert` observers, which gives exact push notification, inspector visibility and `Copy` handles that end setter cloning ([mutable.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/mutable.rs), [owner.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/owner.rs)). The open cost is that every read needs `&Reads` or `&World`, which breaks `coat!`'s closure-captured `$x`. Also budget for entity count: viridia reports about three hidden entities per button in the same design ([discussion #21654](https://github.com/bevyengine/bevy/discussions/21654)). Decide after P4-1 shows whether `Arc` signals with subscriber lists are good enough.

## Conclusion

The research changes the order of work more than its content. Before the probes, the natural reading of this crate was "an API that needs polishing". After them, it is "a runtime with lifetime and identity bugs that polishing would bury". Every Phase 0 item is a few dozen lines, and together they remove the crash, the leaks and the one-frame partial UI that would otherwise surface as user bug reports against any new API. The single most valuable addition is not another paradigm but a bridge: presence-tracked `Reads` plus `computed`. That makes the ECS layer, `coat!` and webby share one dependency model, turns `bind` into a special case, and gives Feathers-style interaction styling a one-line expression. The ecosystem shows that no Bevy crate yet offers both same-frame convergence and push invalidation over BSN templates. bevy_reactor polls and haalka lags, so finishing P4-1 on top of the unified core would give bevy_radioactive a distinct position rather than parity.

Two uncertainties should gate the later phases. First, the `coat!` capture change (P1-5) and the arm-keyed `match` (P1-6) depend on macro and `bsn!` closure semantics the audit did not inspect, so prototype them before promising them in 0.5 notes. Second, no benchmark exists for polling cost, observer-trigger cost, or entity-per-signal overhead in Bevy 0.20. P4-1 and P4-3 should start with a measurement harness, because the right answer between `Arc` signals and entity-backed signals is an empirical one.
