# Bevy Ecosystem Reactive / Declarative UI Libraries: A Comparative Survey (as of Oct 2026)

Repository activity facts below ("pushed", stars, archived flag) come from the GitHub REST API (`gh api repos/<owner>/<repo>`), queried 2026-10-26. Source code quotes for bevy_reactor and jonmo come from their `main` branches, fetched the same day through the GitHub contents API.

## Q1. What primitives does each crate expose (Signal / Mutable / Derived / Memo / Effect / Cx / For / Switch / Cond / keyed lists)?

### Takeaway
The ecosystem has split into four reactive models: (1) viridia's fine-grained "Solid-style" model (bevy_reactor, earlier quill), with `Mutable`/`Derived`/`Signal`/`Effect`/`Cx` and control-flow templates (`if_then_else`, `switch`, `for_each`); (2) databasedav's FRP push-signal graph (jonmo, with haalka on top), with `signal::from_*` combinators, `SignalVec`/`SignalMap` and a `Builder`; (3) UkoeHB's trigger/reactor model (bevy_cobweb), with `ReactRes`/`React<T>` plus systems registered on triggers; (4) non-signal approaches: immediate mode (bevy_immediate), markup and templates (bevy_hui, bevy_cobweb_ui, bevy_superui, bevy_pf), CSS styling (bevy_flair), layout (bevy_lunex), and Changed-driven widget systems (woodpecker_ui).

### Cited Findings

**bevy_reactor (viridia): active, on Bevy 0.20 with BSN**
- Description "Fine-grained reactive framework for Bevy". Last pushed 2026-09-18, 23 stars, not archived. The workspace pins `bevy = { version = "0.20.0-rc.1", features = ["ghost_nodes", "bevy_feathers", ...] }`. Sibling crates: `bevy_reactor_formulae`, `bevy_reactor_gizmoids`, `bevy_reactor_nodegraph`, `bevy_reactor_prop_inspect`, `bevy_keratin` — [Cargo.toml](https://github.com/viridia/bevy_reactor/blob/main/Cargo.toml). The latest commit message reads "Migrate to Bevy 0.22-rc.1" (2026-09-18), but Cargo.toml says 0.20.0-rc.1. The message is probably a typo; the manifest is authoritative — [commits](https://github.com/viridia/bevy_reactor/commits/main)
- Core modules: `conditional.rs, cx.rs, derived.rs, effect.rs, foreach.rs, lcs.rs, mutable.rs, owner.rs, reaction.rs, signal.rs, tracking_scope.rs` — [src tree](https://github.com/viridia/bevy_reactor/tree/main/crates/bevy_reactor/src)
- `Mutable<T>` is a Copy handle: `{ cell: Entity, component: ComponentId, marker }`. The value lives in `#[derive(Component)] struct MutableCell<T>(T)` on its own entity. API: `get(cx)`, `get_clone`, `as_ref`, `set(cx, v)`, `set_clone`, `update(w, |Mut<T>| ..)`, `signal()` — [mutable.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/mutable.rs)
  ```rust
  pub fn create_mutable<T: Send + Sync + 'static>(world: &mut World, owner: Entity, init: T) -> Mutable<T> {
      let cell = world.spawn(MutableCell::<T>(init)).id();
      world.entity_mut(owner).add_one_related::<OwnedBy>(cell);
      ...
  ```
- `Signal<T>` is an enum that erases the source type: `Signal::Mutable(Mutable<T>) | Signal::Derived(Derived<T>) | Signal::Constant(T)`, with `get`, `get_clone` and `map(rc, f)`. `IntoSignal<T>` is implemented for `T` (giving a Constant), `Mutable<T>`, `Derived<T>` and `Signal<T>`, so widget props can accept either a literal or a reactive value — [signal.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/signal.rs)
- `Derived<R>` stores a boxed `Fn(&mut Cx) -> R` in a `DerivedCell<R>` component on its own entity, created via `create_derived(...)` — [derived.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/derived.rs)
- `Cx<'p,'w>` wraps `&World` plus `&mut TrackingScope`. It provides `owner()`, `entity(e)` (returns `EntityRefTracked`), `resource::<T>()`, `component::<C>(e)`, `get_inherited_component::<C>()` (a context-like lookup up the hierarchy) and `on_cleanup(..)`. The `Lens<D>` trait (renamed from `call` to `read` in Aug 2026) is implemented for any `Fn(&Cx) -> D` and for `Signal<D>` — [cx.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/cx.rs)
- `Effect<D, DepsFn: Lens<D>, EffectFn: Fn(&mut EntityWorldMut, D)>` is a BSN `Template`. It has two phases: a tracked deps phase, then an untracked apply phase on the target entity. Because Bevy has a blanket `impl<T: Clone + Unpin> Template for T`, the crate makes its own template types `!Unpin` to avoid a conflicting impl (the same trick Bevy uses for `EntityTemplate`) — [effect.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/effect.rs)
- Control flow is used inside `bsn!` through `@` scene functions. Example from `examples/control_flow.rs` — [control_flow.rs](https://github.com/viridia/bevy_reactor/blob/main/examples/control_flow.rs):
  ```rust
  commands.spawn_scene(bsn!{
      Node { ... }
      Children [
          Text("State: ")
          --
          @switch(|cx: &Cx| *cx.resource::<State<GameState>>().get(), |cases| {
              cases
                  .case(GameState::Play, || bsn_list!{Text("Playing")})
                  .fallback(|| bsn_list!{Text("Not Playing")});
          })
          --
          @if_then_else(
              |cx: &Cx| *cx.resource::<State<GameState>>().get() == GameState::Play,
              || bsn_list!{Text("Yes: Playing")},
              || bsn_list!{Text("No: Not Playing")}
          )
      ]
  });
  ```
- `IfThenElse` inserts `(ReactionCell, TrackingScope, GhostNode)` on the parent and keeps an `IfState { Unset, True, False }`. It respawns a branch only when the boolean flips. A branch is any `SceneListFn` (`Fn() -> impl SceneList`) applied via `queue_apply_scene(bsn!{ @bundle_template(|ctx| { ctx.entity.despawn_children(); Ok(()) }) Children [ {(self)()} ] })` — [conditional.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/conditional.rs), [lib.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/lib.rs)
- `ForEach` takes an items lens `Lens<Vec<Item>>`, a comparator `CmpFn: Fn(&Item,&Item)->bool` (which acts as the key), a body `Fn(EntityWorldMut, &Item, usize) -> impl SceneList`, and an optional fallback. It diffs with a recursive longest-common-subsequence (LCS) pass. Matched rows are kept; unmatched rows are despawned or spawned, each wrapped in a `GhostNode` child — [foreach.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/foreach.rs)
- viridia's own example of a "reactive component" (described in the Bevy discussion): an `insert_dyn` closure in a BSN template reads `InteractionDisabled`, `Pressed` and `Hovered` through `Cx` and returns a `BackgroundColor` — [Discussion #21654](https://github.com/bevyengine/bevy/discussions/21654)

**bevy_reactor, earlier design (blog series "Reactivity in Bevy: From the Bottom Up")**
- Primitives: Mutables, Deriveds, signals/getters, Effects, Callbacks (stored closures taking `Props`), and two contexts: `Cx` (read/write/setup) and `Rcx` (read-only). The design follows Solid: setup runs once and local variables hold signals, not values — [Part 2](https://machinewords.hashnode.dev/reactivity-in-bevy-from-the-bottom-up-part-2)
  ```rust
  let counter = cx.create_mutable(0);
  cx.create_effect(move |cx| {
      println!("The count is: {}", counter.get(cx));
  });
  let focus = cx.use_resource::<Focus>();
  ```

**bevy_quill (viridia): stale**
- Last pushed 2025-08-27, 202 stars (GitHub API). The README calls it "an experimental library" inspired by React, Solid, Dioxus and Xilem, and says it needs the unstable `impl_trait_in_assoc_type` feature — [quill README](https://github.com/viridia/quill)
- Primitives: `ViewTemplate` (`create(cx) -> impl View`, which re-runs when dependencies change, like a React component), `View` (`build/rebuild/raze`), `Cx` hooks (`create_mutable`, `create_effect`, `create_memo`, `create_entity`, `create_callback`, `use_resource`, `use_component`), `Element`, `Cond`, `Switch` (works with Bevy States), `For::each / each_cmp / index`, and `Dynamic` (handles a child whose view type changes). Hooks are memoized by call order, so they must not be called conditionally — [quill README](https://github.com/viridia/quill)
  ```rust
  Cond::new(counter.count & 1 == 0, "even", "odd"),
  Switch::new(state)
      .case(EditorState::Realm, EditModeRealmControls)
      .case(EditorState::Terrain, EditModeTerrainControls)
  commands.add(TriggerReaction(owner));
  ```

**jonmo (databasedav): active, FRP signals as Bevy systems**
- Description "ergonomic Bevy-native reactivity powered by FRP signals". Last pushed 2026-08-24. Its API is ported from futures-signals, MoonZoon and Dominator. It provides the `SignalExt`/`SignalVecExt`/`SignalMapExt` combinators, `MutableVec`/`MutableBTreeMap` for "diff-less constant-time" collection updates, and a `jonmo::Builder`. It supports `no_std` — [jonmo README](https://github.com/databasedav/jonmo)
- Compatibility table: Bevy 0.18 → jonmo 0.7, 0.17 → 0.6, 0.16 → 0.3, 0.15 → 0.1. The `main` Cargo.toml is at version 0.7.0 but depends on `bevy_ecs = "0.19"`, so main already tracks 0.19 — [README](https://github.com/databasedav/jonmo), [Cargo.toml](https://github.com/databasedav/jonmo/blob/main/Cargo.toml)
- Counter example (abridged):
  ```rust
  let counter_holder = LazyEntity::new();
  jonmo::Builder::from(Node { .. })
      .insert(Counter(0))
      .lazy_entity(counter_holder.clone())
      .child(jonmo::Builder::from((Node::default(), TextFont::from_font_size(25.)))
          .component_signal(
              signal::from_component_changed::<Counter>(counter_holder.clone())
                  .map_in(deref_copied).map_in_ref(ToString::to_string)
                  .map_in(Text).map_in(Some)))
  // buttons:
  .observe(move |_: On<Pointer<Click>>, mut counters: Query<&mut Counter>| {
      **counters.get_mut(*counter_holder).unwrap() += step;
  })
  ```
  — [jonmo README](https://github.com/databasedav/jonmo)

**haalka (databasedav): active, UI layer on jonmo**
- Described as an "ergonomic reactive Bevy UI library powered by FRP signals of jonmo". Element types include `El::<Node>`, `Row::<Node>` and `El::<Text>`. Methods include `with_node`, `child`, `item`, `lazy_entity`, `on_click`, `text_signal` and `background_color_signal`. Signal sources include `signal::from_component_changed`, `signal::from_entity(..).has_component::<T>()`, `.dedupe()`, `.map_bool_in`, `.map_in`. A `futures_signals_ui` feature lets futures-signals and jonmo interoperate. Extras: pointer events, cursor, scroll, text input, grid — [haalka README](https://github.com/databasedav/haalka)
- Compatibility: haalka 0.7 → Bevy 0.18 (`Cargo.toml` main at 0.7.1 depends on bevy 0.18). Last pushed 2026-08-24, 196 stars — [Cargo.toml](https://github.com/databasedav/haalka/blob/main/Cargo.toml); listed on Bevy Assets with `^0.18` — [Bevy Assets](https://bevy.org/assets/)

**bevy_cobweb (UkoeHB): reactivity via triggers and systems; stalled at Bevy 0.17**
- Primitives: `ReactRes<R>`/`ReactResMut<R>` (reactive resources; mutation needs `Commands`) and `React<C>` components. Reactors are registered with `c.react().on(trigger, system)`. Triggers: `resource_mutation::<R>()`, `insertion::<C>()`, `mutation::<C>()`/`entity_mutation::<C>(e)`, `despawn(e)`, `broadcast::<E>()`, `entity_event::<E>(e)`. Triggers can be combined in tuples — [REACT.md](https://github.com/UkoeHB/bevy_cobweb/blob/master/src/react/REACT.md)
  ```rust
  fn increment(mut c: Commands, mut counter: ReactResMut<Counter>) {
      counter.get_mut(&mut c).0 += 1;
  }
  c.react().on(resource_mutation::<Counter>(), |counter: ReactRes<Counter>| { /* ... */ });
  c.react().on((resource_mutation::<A>(), entity_insertion::<B>(entity)), ...);
  let token = c.react().on_revokable(resource_mutation::<A>(), || { /* ... */ });
  c.react().revoke(token);
  ```
- Compatibility: bevy 0.17 → bevy_cobweb 0.18 (latest). Last pushed 2025-11-06 — [README](https://github.com/UkoeHB/bevy_cobweb)

**bevy_cobweb_ui: unmaintained**
- The README says "No longer maintained as of 01/13/2026". It used the COB asset format with hot reload, and its last supported Bevy is 0.17 — [bevy_cobweb_ui README](https://github.com/UkoeHB/bevy_cobweb_ui)

**sickle_ui: obsolete**
- The original repo is marked obsolete because of Bevy 0.15 changes; its last release (v0.4.0) targets Bevy 0.14.2. The README points to a fork at UkoeHB/sickle_ui — [sickle_ui mirror](https://gitee.com/star_mark/sickle_ui). That fork was last pushed 2024-10-04 (GitHub API), so it is also stale.

**bevy_immediate (PPakalns): active, Bevy 0.20**
- Immediate-mode UI on retained ECS. The README says it "only visits each entity once per tick and does minimal amount of changes". Entities are matched by hierarchical ids: `ui.tch()` (call-site id via `#[track_caller]`), `ui.ch_id("id")`, `lch!(ui, idx)`. Capabilities such as `.clicked()`, `.hovered()` and `.selected(bool)` are extension traits. It interops with BSN via `.on_spawn_apply_scene`/`.on_spawn_queue_apply_scene` and with `bevy_feathers`/`bevy_ui_widgets`. bevy_immediate 0.9 → Bevy 0.20 — [bevy_immediate README](https://github.com/PPakalns/bevy_immediate)
  ```rust
  for idx in 0..count {
      ui.ch_id(("my_loop", idx));
      lch!(ui, idx);
  }
  ```

**bevy_hui (Lommix): markup templates, Bevy 0.20**
- Pseudo-HTML templates with hot reload in the style of Web Components. Any value can be a dynamic `<property>` injected at runtime (recursively). Events bind to registered Bevy systems by name (`on_press="start_game"`). Bevy 0.20 → bevy_hui 0.8 — [bevy_hui README](https://github.com/Lommix/bevy_hui). It has no signal or reactive graph.

**woodpecker_ui (StarArawn)**
- An ECS-first UI rendered with vello, laid out with Taffy, with Parley text and dx hot-patching. Its Cargo.toml on main still depends on Bevy 0.16, though the repo was last pushed 2026-07-21 — [woodpecker_ui README/Cargo.toml](https://github.com/StarArawn/woodpecker_ui). Behind on Bevy versions.

**bevy_lunex, bevy_flair, others**
- bevy_lunex: "Blazingly fast retained layout engine for Bevy ECS", path-based and ECS-first. Listed for Bevy `^0.19`; 971 stars; last pushed 2026-10-09. It is a layout engine, not a reactivity framework — [Bevy Assets](https://bevy.org/assets/), [repo](https://github.com/bytestring-net/bevy_lunex)
- bevy_flair: "Bevy UI styling using CSS", listed for `^0.20.0`; last pushed 2026-10-23 — [repo](https://github.com/eckz/bevy_flair), [Bevy Assets](https://bevy.org/assets/)
- Newer Bevy Assets UI entries: bevy_superui ("reactive web-style UI framework using TSX", hot reload, ^0.20), bevy_pf (XAML with data binding and templates, ^0.19), bevy_pages (XML, ^0.19), bevy_extended_ui (HTML/CSS, ^0.19) — [Bevy Assets](https://bevy.org/assets/)
- actuate (matthunz): a "borrow-checker friendly framework for declarative programming". A `Compose` trait with `compose(cx: Scope<Self>)`, hooks such as `use_mut`, `SignalMut::update`, and composables that can borrow from ancestors (`Cow<'a, _>`) — [actuate README](https://github.com/matthunz/actuate)
  ```rust
  impl Compose for Counter {
      fn compose(cx: Scope<Self>) -> impl Compose {
          let count = use_mut(&cx, || cx.me().start);
          material_ui((
              text::headline(format!("High five count: {}", count)),
              button(text::label("Up high")).on_click(move || SignalMut::update(count, |x| *x += 1)),
              if *count == 0 { Some(text::label("Gimme five!")) } else { None },
          ))
      }
  }
  ```
- i-cant-believe-its-not-bsn (Leafwing): `WithChild<B>`/`WithChildren` components consumed by hooks to spawn children (usable from `#[require]`), and a `template!()` macro that supports incremental rebuilds. Archived 2025-07-13; last supported Bevy is 0.16.1 — [repo](https://github.com/Leafwing-Studios/i-cant-believe-its-not-bsn)
  ```rust
  #[derive(Component)]
  #[require(Transform, WithChild<PlayerHead>, WithChild<PlayerBody>)]
  struct Player;
  ```
- Other names that turned up: rx_bevy (Reactive Extensions observables and operators for Bevy, v0.3.1) — [rust-digger](https://rust-digger.code-maven.com/crates/rx_bevy); ui4 ("vdom-less UI library which uses fine-grained reactivity", self-described as very young) — [ui4](https://github.com/RustWorks/ui4_bevy_engine); bevy_kot (ReactCommands plus bevy_lunex, an older UkoeHB crate) — [docs.rs](https://docs.rs/bevy_kot)

### Inferences
- Only bevy_reactor has a full Solid-style primitive set (Mutable/Derived/Signal/Effect plus If/Switch/For with LCS diffing) running natively on Bevy 0.20 BSN `Template`s. It is the closest prior art for a BSN-based library.
- jonmo/haalka are the only maintained push-based FRP signal graphs with first-class reactive collections (`MutableVec`, `SignalVec`), which bevy_reactor lacks: its `ForEach` re-diffs a whole `Vec` with LCS.
- `IntoSignal<T>` for constants (`Signal::Constant`) is a small but useful idea: a widget prop can take a literal or a signal without separate APIs.

### Gaps
- No crate named `bevy_mod_reactivity`, `bevy_ecs_reactive` or "bevy-reactive" could be found; searches returned unrelated crates. They may not exist under those names.
- bevy_quill's exact last supported Bevy version was not confirmed (the README does not state it).
- Woodpecker's widget update model (render systems or change-driven diffing) was not verified in source.
- Whether bevy_reactor has a `Memo` distinct from `Derived` in the current code was not confirmed. Quill had `create_memo`; the bevy_reactor file list shows only `derived.rs`.

## Q2. Which crates store reactive state as entities/components (ECS-native), and how do they hook into observers, hooks or schedules?

### Takeaway
Both major signal libraries are entity-native. bevy_reactor stores every Mutable, Derived and reaction on its own entity (`MutableCell<T>`, `DerivedCell<R>`, `ReactionCell` + `TrackingScope` components), owned through an `OwnedBy`/`OwnerOf` relationship with `linked_spawn`. It runs reactions from one exclusive `Update` system that polls change ticks. jonmo stores every signal node as an entity holding a Bevy `System`, wired into a DAG of upstream/downstream components and processed level by level per schedule. bevy_cobweb stores reactors as system commands and drives them inline through `Commands`.

### Cited Findings
- **bevy_reactor ownership relationship:** `#[relationship(relationship_target = OwnerOf)] pub struct OwnedBy(pub Entity);` and `#[relationship_target(relationship = OwnedBy, linked_spawn)] pub struct OwnerOf(Vec<Entity>);` — documented as "a hidden satellite entity that is lifetime scoped to its owning entity" — [owner.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/owner.rs)
- **bevy_reactor effects as entities:** `Effect::build_template` spawns `(ReactionCell::new(EffectReaction{target, deps, effect}), TrackingScope::new(world.change_tick()))`, attaches it to the target with `add_one_related::<OwnedBy>`, and queues `InitialReactionCommand` so the first run happens after creation — [effect.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/effect.rs)
- **Control-flow nodes are not separate entities:** `IfThenElse` and `ForEach` insert the `ReactionCell` + `TrackingScope` directly on the parent entity, together with `GhostNode` (a layout-transparent UI node; this requires the `ghost_nodes` feature) — [conditional.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/conditional.rs)
- **`ReactionCell(pub Arc<Mutex<dyn Reaction + Send + Sync>>)`** with `trait Reaction { fn react(&mut self, owner: Entity, world: &mut World, tracking: &mut TrackingScope); }` — [reaction.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/reaction.rs)
- **Schedule hook:** `ReactorPlugin` registers an `on_remove` component hook for `TrackingScope` (to run cleanups) and adds the system `app.add_systems(Update, run_reactions)` — [lib.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/lib.rs), [tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)
- **Older blog design:** "an effect creates an entity with a tracking scope and a `Reaction` component" that lives until its owner is despawned. During a run the TrackingScope is a temporary copy that is written back as a component afterwards. The tracking scope sits in a `RefCell` so immutable contexts can record dependencies — [Part 2](https://machinewords.hashnode.dev/reactivity-in-bevy-from-the-bottom-up-part-2)
- **quill:** "Mutable … a handle which refers to a piece of mutable data stored within the Bevy World". Mutables are components; ones created via `Cx` are despawned with the view, while ones created on `World` must be deleted manually — [quill README](https://github.com/viridia/quill)
- **jonmo graph:** "The signal graph is a directed acyclic graph (DAG) where each node is backed by a Bevy `System`. … Cycles are detected and rejected at edge-creation time." `pub struct SignalSystem(pub Entity)` is a "Newtype wrapper for Entitys that hold systems in the signal graph". `ScheduleTag`/`ScheduleHint` components assign a node to a schedule (`Update`, `PostUpdate`, …), and downstream nodes inherit upstream schedules. The graph is partitioned per schedule — [graph.rs](https://github.com/databasedav/jonmo/blob/main/src/graph.rs)
- **jonmo builder:** "automated system lifecycle management when using the builder API, simple component on-remove hook when not". The builder attaches `.observe(..)` observers directly — [jonmo README](https://github.com/databasedav/jonmo)
- **bevy_cobweb:** reactors run "inside the Bevy `Commands` flow". A reaction triggered from within another reactor runs inside it (reactions telescope). The custom runner garbage collects `AutoDespawner` entities, removes the command from the world, runs it, applies deferred commands, and reinserts it — [REACT.md](https://github.com/UkoeHB/bevy_cobweb/blob/master/src/react/REACT.md)

### Inferences
- Signal-as-entity is the convergent design: it makes signals inspectable (as in a world inspector), lets relationship despawn handle lifetimes, and keeps handles `Copy` (`Entity` + `ComponentId` + `PhantomData`). The trade-off is entity count; see Q4.
- bevy_reactor's choice to put control-flow reactions on the parent `GhostNode` (rather than a satellite entity) saves entities and makes "this region rebuilds itself" structural.
- jonmo's "signal = registered system" design gets Bevy dependency injection (`Query`, `Res` as signal inputs) for free. bevy_reactor's `Cx` deliberately gives that up in exchange for automatic dependency tracking.

### Gaps
- I did not verify whether jonmo's per-signal systems run inside one exclusive driver system (likely, given topological level processing) or as separate schedule systems.

## Q3. How is dependency tracking done, and which API ideas are clever and Bevy-idiomatic?

### Takeaway
bevy_reactor does automatic, fine-grained tracking: each read through `Cx` records `(Entity, ComponentId, present)` or a resource `ComponentId` into a `TrackingScope`, and each frame it compares Bevy change ticks. This piggybacks on ECS change detection, so any ordinary system mutation triggers reactions with no special wrapper types. jonmo uses explicit FRP wiring: combinators build edges and values are pushed. cobweb uses explicit triggers and requires mutation through `React*` wrappers plus `Commands`.

### Cited Findings
- **TrackingScope fields:** `component_deps: HashSet<(Entity, ComponentId, bool)>` (the bool records whether the component was present), `resource_deps: HashSet<ComponentId>`, `changed: AtomicBool` (manual invalidation), `tick: Tick` (last run), `cleanups: Vec<Box<dyn FnOnce(&mut DeferredWorld)>>` — [tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)
- **Change test:** `entity.get_change_ticks_by_id(c).map(|ct| ct.is_changed(self.tick, tick) || !present).unwrap_or(*present)`. A missing entity yields `present`, so removals and insertions of tracked components are detected — [tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)
- **Run-to-convergence loop:** "running a reaction may trigger other reactions, so we loop until there are no more reactions left to run … We count the number of 'divergences' … and impose a strict limit", with `MAX_DIVERGENCE_CT = 32`. Each iteration after the first calls `world.increment_change_tick()`. The loop queries `(Entity, &mut TrackingScope, &ReactionCell)` and collects changed scopes, runs their cleanups, then runs the reactions. An optional `TrackingScopeTracing` resource records which reactions ran, for debugging — [tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)
- **Two-phase effects:** the `deps` lens is tracked; the `effect` closure receives `&mut EntityWorldMut` untracked. This separates reading from writing and avoids self-triggering — [effect.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/effect.rs)
- **Inherited context lookup:** `Cx::get_inherited_component::<C>()` provides a context/provider pattern over the entity hierarchy — [cx.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/cx.rs)
- **Untracked event handlers:** event handlers use direct `World` access, which skips tracking; the author considers this fine because writes still trigger change detection — [Part 2](https://machinewords.hashnode.dev/reactivity-in-bevy-from-the-bottom-up-part-2)
- **jonmo processing semantics:** levels are assigned (1 + max upstream level) and processed in topological order, sorted by entity index for determinism. A `Some` output is forwarded and `None` stops propagation. A node with multiple upstreams "runs once per upstream that fires", but only the final output is forwarded. Signals registered during processing run in the same frame, which avoids one-frame delays. `poll_signal` offers a synchronous pull — [graph.rs](https://github.com/databasedav/jonmo/blob/main/src/graph.rs)
- **jonmo query-based signal sources:** `signal::from_component_changed::<Counter>(entity)`, `signal::from_entity(e).has_component::<T>()`, and `.component_signal(...)`, which drives a component on the built entity (returning `None` removes it) — [jonmo README](https://github.com/databasedav/jonmo), [haalka README](https://github.com/databasedav/haalka)
- **cobweb typed triggers:** tuples of triggers such as `(resource_mutation::<A>(), entity_insertion::<B>(e))`. `ReactorMode::{Persistent, Cleanup, Revokable}` controls lifetime, and `once` creates a one-shot reactor — [REACT.md](https://github.com/UkoeHB/bevy_cobweb/blob/master/src/react/REACT.md)
- **bevy_immediate:** location-based identity (`#[track_caller]` ids) replaces dependency tracking; the UI is re-evaluated every frame with minimal ECS writes — [bevy_immediate README](https://github.com/PPakalns/bevy_immediate)
- **Hook-driven declarative children (i-cant-believe-its-not-bsn):** a `WithChild<B>` component's hook moves the bundle into a child entity and removes itself, which makes children composable through `#[require]` — [repo](https://github.com/Leafwing-Studios/i-cant-believe-its-not-bsn)

### Inferences
Ideas worth borrowing for a BSN library:
- Tracking `(Entity, ComponentId, present)` means "has component" and removal are reactive inputs at no extra cost.
- An `AtomicBool` "dirty" escape hatch for non-ECS invalidation.
- `Lens<D>` implemented for closures and for `Signal<D>`, so control-flow nodes accept either.
- jonmo's `component_signal`, where `Option<C>` inserts or removes a component, is an elegant "reactive component" shape.
- jonmo's per-node schedule assignment.
- cobweb's trigger tuples for multi-source subscriptions.
- bevy_reactor's `get_inherited_component` for context.

A hybrid seems plausible: auto-tracked `Cx` reads for ergonomics, with observer-driven invalidation (setting `changed`) instead of per-frame polling.

### Gaps
- I found no crate that implements system-param-based derived values (a derived computed from `Query`/`Res` system params with automatic tracking) other than jonmo's explicit system-backed signals.
- I found no crate that uses Bevy relationships as the dependency-graph edges themselves (as opposed to ownership). bevy_reactor uses relationships only for ownership.

## Q4. What are the known complaints and limitations (borrowing, World access, runaway recomputation, glitch-freedom, despawn leaks, performance)?

### Takeaway
The main costs of the entity-native, auto-tracked approach (stated by viridia himself, Oct 2025) are: a hidden entity per effect (about 3 per button), a single exclusive non-parallel system, per-frame polling of every dependency, and a `Cx` that is not idiomatic Bevy (no queries or DI). Observer-based approaches fire once per component change, which causes double recomputation. jonmo explicitly re-runs multi-upstream nodes once per firing upstream. Lifecycle footguns appear in quill (`despawn_recursive` panics, manual deletion of World-created mutables) and in jonmo (registration counts must be balanced with `SignalHandle::cleanup`).

### Cited Findings
- **viridia's Bevy discussion "Reactive ergonomics: a comparison of approaches"** (Oct 25, 2025):
  - bevy_reactor effects: "Each effect adds a hidden entity (about 3 per button)", all reactions run in one exclusive system, dependencies are polled every frame, `Cx` is not idiomatic (no queries or dependency injection), and the crate relies on a "poor woman's trait queries" workaround via component hooks.
  - Pros: colocation with the template, one plugin, batched updates (shared dependencies are computed once), mutation and removal detection, no over-reaction.
  - Observers: "Each component change fires a separate callback, so a style can be recalculated twice when two states change together"; mutations are hard to detect.
  - ECS systems: more than 100 lines of boilerplate for buttons; `RemovedComponents` requires a separate code path. Feathers needs 14 plugins.
  — [Discussion #21654](https://github.com/bevyengine/bevy/discussions/21654)
- **Replies to that discussion:** alice-i-cecile dislikes `RemovedComponents` (it "can miss events"), suggests a `Removed<C>` filter, and says colocation is "fundamentally" not fixable in the systems design. viridia replied that installed widget plugins add runtime overhead (observers on every pointer event) and binary size — [Discussion #21654](https://github.com/bevyengine/bevy/discussions/21654)
- **Runaway recomputation guard:** bevy_reactor caps divergent convergence cycles at 32 — [tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs). quill warns "Don't invoke `TriggerReaction` from within a reactive callback, as this can cause an infinite loop" — [quill README](https://github.com/viridia/quill)
- **Unused cleanup API:** a TODO in TrackingScope says of cleanups: "This is a concept taken from Solid, but I don't actually use it anywhere" — [tracking_scope.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/tracking_scope.rs)
- **quill despawn and type pitfalls:** "Calling `.despawn_recursive()` will panic"; not compatible with `StateScoped`; `.into_view_child()` can panic on rebuild when the view type changes (wrap in `Dynamic`); hooks must not be called conditionally — [quill README](https://github.com/viridia/quill)
- **jonmo lifecycle:** "each call to `.register` increments a registration count on the signal and its upstreams. To release a signal, `SignalHandle::cleanup` must be called, either explicitly or implicitly by storing handles in a `SignalHandles` component (which calls cleanup when the entity is despawned)". When the count reaches zero and no downstreams remain, the backing system is despawned — [graph.rs](https://github.com/databasedav/jonmo/blob/main/src/graph.rs)
- **jonmo glitch semantics:** a node with multiple upstreams "runs once per upstream that fires in a given frame … only the final output … is forwarded". Topological level ordering prevents downstream nodes from seeing stale upstream values within a frame — [graph.rs](https://github.com/databasedav/jonmo/blob/main/src/graph.rs)
- **cobweb control flow:** recursive commands are queued rather than run inline, and the docs advise against heavy recursion because "control flow becomes hard to follow". Mutating reactive state requires `Commands` (`get_mut(&mut c)`) — [REACT.md](https://github.com/UkoeHB/bevy_cobweb/blob/master/src/react/REACT.md)
- **bevy_immediate limitations:** id discipline (duplicate sibling ids map to the same entity; `.ch()` is unsafe for appearing or disappearing children), queries need `Without<ImmMarker<Caps>>` filters to avoid conflicts, and defining capability sets is unsolved — [bevy_immediate README](https://github.com/PPakalns/bevy_immediate)
- **Structural reactivity is hard:** Bevy's BSN discussion notes that top-level reactivity is easy while branching, looping and adding or removing children is the hard part, and floats observer-style fine-grained reactivity built on change detection — [Discussion #14437](https://github.com/bevyengine/bevy/discussions/14437)
- **Callback wiring with observers:** observers force a receiver to `.observe()` each sender and track sender ids. A workaround is pre-spawning blank entities to capture ids, which BSN scene functions could reduce — [Discussion #21044](https://github.com/bevyengine/bevy/discussions/21044)
- **Partitioning conflict:** ECS partitions state, while signal dependencies cut across those boundaries (2023) — [Discussion #10212](https://github.com/bevyengine/bevy/discussions/10212)
- **Borrowing:** the earlier reactor design wraps the tracking scope in a `RefCell` so `&Cx` can record dependencies — [Part 2](https://machinewords.hashnode.dev/reactivity-in-bevy-from-the-bottom-up-part-2). The current `ReactionCell` uses `Arc<Mutex<dyn Reaction>>` so a reaction can be taken out and run with `&mut World` — [reaction.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/reaction.rs). actuate advertises "borrow-checker friendly" state with composables borrowing from ancestors — [actuate README](https://github.com/matthunz/actuate)

### Inferences
- Polling cost grows with total dependencies, not with changes. A BSN library could keep auto-tracking but use observers or component hooks on tracked `MutableCell<T>` writes to mark scopes dirty (push invalidation, pull evaluation). That would remove per-frame scans for idle UIs.
- Glitch freedom: bevy_reactor's convergence loop can run a derived-consuming reaction more than once per frame. jonmo's topological levels are more principled, but multi-upstream nodes run repeatedly. Neither offers a true batched, glitch-free combineLatest-style join with single evaluation, which is a potential differentiator.
- Despawn leaks are mostly solved by `OwnedBy`/`OwnerOf` with `linked_spawn` (bevy_reactor) or `SignalHandles` on-despawn (jonmo). The remaining risk is handles created outside an owner (quill's World-created mutables, jonmo's raw `.register`).

### Gaps
- I found no published benchmarks comparing these crates.
- I found no Reddit or Discord complaint threads with concrete numbers; complaints here come from maintainer-authored docs and GitHub discussions.

## Q5. Which crates have updated to Bevy 0.18–0.20, and how did they adapt to BSN?

### Takeaway
On Bevy 0.20: bevy_reactor (0.20.0-rc.1, built natively on BSN `Template`/`Scene`/`bsn!`), bevy_immediate 0.9 (BSN interop via `on_spawn_apply_scene`), bevy_hui 0.8, bevy_flair, and bevy_superui. On 0.18–0.19: haalka 0.7 (0.18), jonmo 0.7 (0.18 per its README, with main on 0.19), bevy_lunex (0.19). Stalled at 0.17 or earlier: bevy_cobweb, bevy_cobweb_ui (unmaintained), woodpecker_ui (0.16 on main), i-cant-believe-its-not-bsn (archived, 0.16), sickle_ui (0.14, obsolete), quill (stale since Aug 2025).

### Cited Findings
- **bevy_reactor on 0.20:** depends on `bevy = "0.20.0-rc.1"` with `ghost_nodes` and `bevy_feathers`. Commented-out lines show it tracked cart's `next-generation-scenes` branch (PR #24474) and ickshonpe's `incremental-ui-updates` branch (PR #25653) during development — [Cargo.toml](https://github.com/viridia/bevy_reactor/blob/main/Cargo.toml). Commit history: "Migrate to bevy main (0.20-dev)" on 2026-08-21 — [commits](https://github.com/viridia/bevy_reactor/commits/main)
- **How bevy_reactor adapted to BSN:**
  - Reactive nodes are `bevy::ecs::template::Template` impls (`build_template(&self, ctx: &mut TemplateContext)`, `clone_template`) invoked inside `bsn!` with `@if_then_else(...)` and `@switch(...)`.
  - Branches are closures returning `bsn_list!{...}` (`SceneListFn`).
  - Rebuilds go through `EntityCommands::queue_apply_scene(bsn!{...})`.
  - A helper `bundle_template` wraps a closure as a `SceneFunction` that calls `push_bundle_template(template(func))`.
  - Template types are made `!Unpin` to sidestep Bevy's blanket `impl<T: Clone + Unpin> Template for T`.
  — [lib.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/lib.rs), [effect.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/effect.rs), [conditional.rs](https://github.com/viridia/bevy_reactor/blob/main/crates/bevy_reactor/src/conditional.rs)
- **bevy_immediate:** 0.9 → Bevy 0.20, 0.7/0.8 → 0.19, 0.5/0.6 → 0.18. Uses `.on_spawn_apply_scene` / `.on_spawn_queue_apply_scene` to apply BSN to immediate-mode entities — [bevy_immediate README](https://github.com/PPakalns/bevy_immediate)
- **bevy_hui:** 0.8 → Bevy 0.20, 0.7 → 0.19, 0.6 → 0.18 — [bevy_hui README](https://github.com/Lommix/bevy_hui)
- **Bevy Assets versions:** bevy_flair ^0.20.0, bevy_superui ^0.20, bevy_immediate ^0.20.0, bevy_hui ^0.20, bevy_lunex ^0.19, bevy_pf ^0.19, haalka ^0.18, bevy_cobweb_ui 0.17 — [Bevy Assets](https://bevy.org/assets/)
- **haalka / jonmo:** haalka 0.7 → 0.18; jonmo 0.7 → 0.18 (README), and jonmo main's Cargo.toml depends on bevy_ecs 0.19. They use the builder pattern (`jonmo::Builder`), not BSN — [jonmo](https://github.com/databasedav/jonmo), [haalka](https://github.com/databasedav/haalka)
- **bevy_cobweb:** latest is 0.18, for Bevy 0.17 (last push Nov 2025). bevy_cobweb_ui has been unmaintained since 2026-01-13 — [bevy_cobweb](https://github.com/UkoeHB/bevy_cobweb), [bevy_cobweb_ui](https://github.com/UkoeHB/bevy_cobweb_ui)
- **i-cant-believe-its-not-bsn:** archived 2025-07-13; its stated purpose was to bridge until BSN arrived — [repo](https://github.com/Leafwing-Studios/i-cant-believe-its-not-bsn)
- **woodpecker_ui:** Cargo.toml on main depends on Bevy 0.16 — [repo](https://github.com/StarArawn/woodpecker_ui)

### Inferences
- bevy_reactor is the only signal library that has re-architected around BSN. Its patterns (reactive `Template`s, `GhostNode` regions, `queue_apply_scene` rebuilds, the `!Unpin` trick) are the most direct references for a Bevy 0.20 BSN reactive library. jonmo/haalka would need a BSN adapter layer; none was found.
- The 0.17 → 0.18+ transition (and BSN landing) appears to have been where several crates stalled: cobweb, cobweb_ui, woodpecker, and quill (superseded by bevy_reactor).

### Gaps
- I found no evidence that jonmo or haalka plan BSN integration, and no released crates.io version of bevy_reactor for 0.20; the workspace may be unpublished. docs.rs and lib.rs returned 404 for bevy_reactor.
- I did not verify how bevy_flair and bevy_superui integrate with BSN.
