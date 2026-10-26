# `bevy_radioactive`

`bevy_radioactive` connects Bevy 0.20 state and entity events to local observers in
code-authored BSN scenes. A UI node can own the handler that keeps it synchronized
without a global system querying every possible view.

It has four layers:

- **Watching and binding** (`watch`, `bind`, `bind_or_else`, `watch_removed`): source-routed
  change notifications for tracked components and resources.
- **Subscriptions** (`subscribe_to`): routing of any entity event from a source to
  listeners.
- **`webby`**: React-style `#[component]` functions, used as `@name(props)`, with `use_state`, `use_memo`,
  `use_effect`, context, and keyed children.
- **`coat!`**: Topcoat-style templates — BSN plus reactive `if`/`for`/`match` and
  `$signal` reads.

## Compatibility

| `bevy_radioactive` | Bevy | Rust |
| --- | --- | --- |
| 0.5 | 0.20 | 1.97.1 or newer |
| 0.4 | 0.20 | 1.97.1 or newer |

```toml
[dependencies]
bevy = "0.20"
bevy_radioactive = { path = "../bevy_radioactive" }
```

The default `ui` feature orders the reactive runtime before Bevy UI layout and makes
`coat!` regions `GhostNode`s. The optional `widgets` feature adds two-way binding for
`bevy_ui_widgets` controls. See [MIGRATION.md](MIGRATION.md) for upgrading from 0.4.

## Quick start

Resources are singleton components in Bevy 0.20, so one API covers both. Add
`RadioactivePlugin`; `watch`, `bind` and `watch_removed` start tracking their type the
first time a scene uses them. Call `app.track::<T>()` only for types you observe
globally or want registered up front.

```rust,no_run
use bevy::{prelude::*, scene::CommandsSceneExt};
use bevy_radioactive::prelude::*;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counter(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Health(u32);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, RadioactivePlugin))
        .init_resource::<Counter>()
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands.spawn_scene(hud());
}

fn hud() -> impl Scene {
    bsn! {
        Node
        Children [
            #Player Health(100)
            --
            // resource source
            Text("0")
            bind(Singleton, |counter: &Counter, text: &mut Text| **text = counter.0.to_string())
            --
            // component source, with a removal fallback
            Text("")
            bind_or_else(
                #Player,
                |hp: &Health, text: &mut Text| **text = format!("HP {}", hp.0),
                |text: &mut Text| **text = "DEAD".into(),
            )
        ]
    }
}
```

## Sources

Every helper takes a source as its first argument:

| Source | Meaning |
| --- | --- |
| `#Name` | A named entity in the same BSN scene |
| `entity` | An `Entity` value |
| `Singleton` | The singleton entity of resource `T` (must exist when the scene resolves) |
| `[#A, #B]` / `vec![...]` | Several sources; the handler runs once per changed source |

One entity may declare any number of watches, bindings, and subscriptions, for the
same or different sources and types. Use `source()` on the event to tell sources apart.

## Helpers

| Helper | Runs when | Handler |
| --- | --- | --- |
| `watch(source, handler)` | `T` is inserted or mutated on a source | observer with `Reaction<T>` input |
| `watch_immediate(source, handler)` | same, plus once on spawn with the current value | same |
| `bind(source, \|&T, &mut C\|)` | on spawn with the current value, then like `watch` | writes component `C` on this entity |
| `bind_deferred(source, write)` | like `watch` (no initial write) | same |
| `bind_or_else(source, write, \|&mut C\|)` | like `bind`, plus removal/despawn | writes `C` for both cases |
| `watch_removed(source, handler)` | `T` is removed or the source despawns | observer with `Removal<T>` input |
| `subscribe_to(source, handler)` | source receives entity event `E` | observer with `Subscription<E>` input |
| `computed(\|r: &Reads\| C)` | anything it read changed | keeps component `C` on this entity equal to the result |
| `computed_opt(\|r: &Reads\| Option<C>)` | same | inserts `C`, or removes it on `None` |
| `layout_effect(\|r: &Reads, commands\|)` | after UI layout, when anything it read changed | side effects that read layout output |

`bind` writes the source's current value when the scene spawns (it is delivered at the
start of the next reactive pass, so a component in the same scene already exists).
`bind_or_else` calls the fallback instead when the source lacks `T`.

`computed` is a derived component: it reads through `Reads` (`r.resource::<R>()`,
`r.get::<C>(entity)`, `r.this::<C>()`), re-runs when any
of those change, and writes with `set_if_neq` so equal results don't republish.

```rust
use bevy::prelude::*;
use bevy_radioactive::prelude::*;

#[derive(Resource, Clone, Copy, Default)]
struct Score(u32);

#[derive(Component, Clone, Copy, Default, PartialEq)]
struct Medal(u8);

fn medal() -> impl Scene {
    bsn! {
        computed(|r: &Reads| Medal(match r.resource::<Score>().map_or(0, |s| s.0) {
            0..10 => 0,
            10..100 => 1,
            _ => 2,
        }))
    }
}
```

### Presence, inherited context and layout effects

`Reads` has two more tracked reads:

- `r.has::<C>(entity)` / `r.this_has::<C>()` return whether the component is there and
  depend only on that: inserting or removing `C` re-runs the reader, changing its value
  doesn't. Use them for markers such as `Pressed`, `Checked` or a selection marker.
- `r.inherited::<C>()` returns `C` from the reader's own entity or its nearest
  `ChildOf` ancestor. It is context provided through the ECS: an ancestor provides a
  value by holding the component, and the nearest one wins, so a subtree can override
  it. Changing the provider's value, adding an override in between, or reparenting
  re-runs the reader. `webby` components get it as `cx.inherited::<C>()`.

`layout_effect(|r: &Reads, commands| ..)` is an `effect` that runs after UI layout and
transform propagation, in `ReactiveSet::PostLayout`, so it can read this frame's
`ComputedNode` or `GlobalTransform`. Values it writes are laid out on the next frame.

```rust
use bevy::{prelude::*, ui::Pressed};
use bevy_radioactive::prelude::*;

#[derive(Component, Clone, Copy, Default, PartialEq)]
struct Theme { accent: Color }

fn panel() -> impl Scene {
    bsn! {
        Node Theme { accent: Color::WHITE }
        Children [
            Node
            computed(|r: &Reads| BackgroundColor(if r.this_has::<Pressed>() {
                Color::BLACK
            } else {
                r.inherited::<Theme>().map_or(Color::BLACK, |t| t.accent)
            }))
            layout_effect(|r: &Reads, _: &mut Commands| {
                if let Some(node) = r.this::<ComputedNode>() {
                    debug!("panel is {:?}", node.size());
                }
            })
        ]
    }
}
```

### Selection by marker move

`Selection` on a group entity remembers which of its entities is selected, and that
entity holds the `Selected` marker. Selecting another entity moves the marker, so only
the old and the new row's `r.this_has::<Selected>()` readers re-run, however long the
list is (Solid's `createSelector`). `commands.select::<Selected>(list, Some(row))` sets
it, and `commands.entity(row).select::<Selected>()` / `.toggle_select::<Selected>()`
select in the nearest ancestor with a `Selection`. Despawning the selected entity clears
the selection; removing or despawning the group removes the marker. Use another marker
type with `Selection<Hovered>` for independent selections on the same group.

```rust
use bevy::{prelude::*, ui_widgets::{Activate, Button}};
use bevy_radioactive::prelude::*;

fn row(label: &'static str) -> impl Scene {
    bsn! {
        Button
        on(|activate: On<Activate>, mut commands: Commands| {
            commands.entity(activate.entity).select::<Selected>();
        })
        computed(|r: &Reads| BackgroundColor(if r.this_has::<Selected>() {
            Color::WHITE
        } else {
            Color::BLACK
        }))
        Children [ Text({label}) ]
    }
}

fn list() -> impl Scene {
    bsn! { Node Selection Children [ @row("a") -- @row("b") -- @row("c") ] }
}
```

### Query monitors

`app.add_monitor::<M>(Monitor::new().with::<A>().without::<B>())` keeps marker `M` on
exactly the entities that match the filter, after flecs monitors. Observers on each
term's `Add`/`Remove` re-check only the entity that changed, and entities that already
match when the monitor is added get the marker too. `On<Add<M>>` and `On<Remove<M>>`
observers are the "entered" and "left" callbacks, and `r.this_has::<M>()` makes the
membership a reactive input.

```rust
use bevy::prelude::*;
use bevy_radioactive::prelude::*;

#[derive(Component)]
struct Health(u32);
#[derive(Component)]
struct Dead;
#[derive(Component, Default)]
struct Alive;

fn plugin(app: &mut App) {
    app.add_monitor::<Alive>(Monitor::new().with::<Health>().without::<Dead>())
        .add_observer(|died: On<Remove<Alive>>| info!("{} is out", died.entity));
}
```

### Two-way widget binding (`widgets` feature)

`bevy_ui_widgets` widgets are controlled: a slider emits `ValueChange<f32>` and waits
for the app to write `SliderValue`. `bind_slider(model)`, `bind_checkbox(model)`,
`bind_tabs(model)` and `bind_radio(model)` do both halves on the widget's entity. The
widget's state follows the model (starting with its current value), and each
`ValueChange` writes the model. The model is a `Signal`, or any ECS state through
`bind_widget_with::<W, _, _>(read, write)`. `.commit_on_final()` writes the model only
when the interaction ends and lets the widget move by itself meanwhile. Implement
`ControlledWidget` to bind other widgets with `bind_widget::<W, _>(model)`. Don't also
add the widget's `*_self_update` observer.

```rust,ignore
use bevy::ui_widgets::{Checkbox, Slider};
use bevy_radioactive::prelude::*;

#[derive(Resource)]
struct Volume(f32);

fn settings(muted: Signal<bool>) -> impl Scene {
    bsn! {
        Node Children [
            Checkbox bind_checkbox(muted.clone())
            --
            Slider
            bind_widget_with::<Slider, _, _>(
                |r: &Reads| r.resource::<Volume>().map(|v| v.0),
                |world: &mut World, v| world.resource_mut::<Volume>().0 = v,
            ).commit_on_final()
        ]
    }
}
```

```rust
use bevy::prelude::*;
use bevy_radioactive::prelude::*;

#[derive(Component, Clone, Default)]
struct Health(u32);

fn duel_log() -> impl Scene {
    bsn! {
        Children [
            #Player Health(100)
            --
            #Enemy Health(50)
            --
            Text("")
            watch([#Player, #Enemy], |hit: Reaction<Health>, names: Query<&Name>, mut q: Query<&mut Text>| {
                if let (Ok(name), Ok(mut text)) = (names.get(hit.source()), q.get_mut(hit.target())) {
                    **text = format!("{name} now has {} HP", hit.0);
                }
            })
            watch_removed(#Enemy, |removal: Removal<Health>, mut q: Query<&mut Text>| {
                if let Ok(mut text) = q.get_mut(removal.target()) {
                    **text = "Enemy defeated".into();
                }
            })
        ]
    }
}
```

`Reaction<T>` dereferences to the shared `T` snapshot. `source()` is the value owner
(or the resource entity) and `target()` is the BSN entity that declared the helper.
For global observers use `app.observe_value::<T>(observer)` with `On<ValueChanged<T>>`,
or observe `On<ValueRemoved<T>>`.

## `webby`: React-style components

Mark a function with `#[component]` and use it in `bsn!` like a scene function:
`@counter(5)`. `cx` is supplied by the runtime; the other arguments are props. The
component re-renders when anything it read changes, and returns plain BSN.

`cx.use_effect(deps, |deps, commands| ..)` runs after the render's scene has been
spawned (children's effects before their parent's), and again when `deps` change. It
may return a cleanup, `move |commands: &mut Commands| ..`, which runs before the next
run and when the component unmounts. `cx.on_unmount(|commands| ..)` registers a
cleanup without an effect.

```rust,ignore
use bevy_radioactive::webby::*;

#[component]
fn counter(cx: &Cx, start: i32) -> impl Scene {
    let (count, set_count) = cx.use_state(|| start);
    let doubled = cx.use_memo(count, |count| count * 2);
    cx.use_effect(count, |count, _commands| info!("count is now {count}"));

    bsn! {
        Button
        on(move |_: On<Pointer<Click>>| set_count(count + 1))
        Children [ Text({format!("{count} (x2 = {doubled})")}) ]
    }
}

#[component]
fn health_bar(cx: &Cx, player: Entity) -> impl Scene {
    let hp = cx.use_component::<Health>(player).map_or(0, |hp| hp.0);
    bsn! { Text({format!("HP {hp}")}) }
}

#[component]
fn app(cx: &Cx) -> impl Scene {
    let todos = cx.use_resource::<Todos>().unwrap_or_default().0;
    let rows = todos.into_iter().map(|t| todo_row(t).key(t.id)).collect::<Vec<_>>();
    bsn! {
        Node
        Children [
            #Player Health(100)
            --
            @counter(5)               // <Counter start={5} />
            --
            @health_bar(#Player)      // `Entity` props accept `#Name`
            --
            Node Children [ {rows} ]  // {todos.map(t => <TodoRow key={t.id} />)}
        ]
    }
}

// mount: commands.spawn_scene(bsn! { @app() })
```

| React | `webby` |
| --- | --- |
| `<Counter start={5} />` | `@counter(5)` |
| `<Row key={id} />` | `row(..).key(id)` or `@keyed(id, row(..))` |
| inline component | `component(\|cx\| bsn! { .. })` |
| `useState` | `let (count, set_count) = cx.use_state(|| 0); set_count(count + 1)` |
| `useReducer` | `let (state, dispatch) = cx.use_reducer(reducer, init); dispatch(action)` |
| effect cleanup / unmount | return `move \|commands: &mut Commands\| ..` from `use_effect`, or `cx.on_unmount(..)` |
| signals | `cx.use_signal(init) -> Signal` |
| `useMemo` / `useEffect` / `useRef` | `cx.use_memo` / `cx.use_effect` / `cx.use_ref` |
| `createContext` / `useContext` | `cx.provide_context(value)` / `cx.use_context::<T>()` |
| store/selector hooks | `cx.use_resource::<R>()`, `cx.use_component::<C>(entity)` |

A component's output is a child of its entity; an entity without a UI `Node` becomes a
`GhostNode`, so `@counter(5)` works as a list item without affecting layout. A render
replaces the subtree; child components keep their hook state, matched by key or else
by the call site of `@name(..)`, so a conditional sibling doesn't shift another
component's state (key components created in loops). Props other than `Entity` must be
`Clone + Send + Sync + 'static`; mark other entity-valued props `#[entity]`, as in
`#[entity] target: Option<Entity>`. A component may return `Result<impl Scene>`: errors
go to Bevy's fallback error handler. `either(cond, || bsn! {..}, || bsn! {..})` picks
between two scenes of different types.

## `coat!`: reactive templates

`coat!` is `bsn!` with a small Rust subset in scene lists, modelled on
[Topcoat](https://github.com/tokio-rs/topcoat)'s `view!`. `$name` reads a `Signal`
(a clone of its value); `$&name.method()` calls methods on the value in place without
cloning it. Control flow that reads a signal becomes a region that rebuilds only
itself, and a component with `$` in its fields is patched in place. Signals used with
`$` are cloned into the closures that need them, so no manual `let x = x.clone();` is
needed.

```rust,ignore
use bevy_radioactive::coat::*;

let todos = signal(|| vec![Todo { id: 1, title: "Write docs" }]);
let open = signal(|| true);

coat! {
    Node
    Children [
        Button
        on(move |_: On<Pointer<Click>>| open.set(!$open))
        --
        Text($(format!("{} todos", $&todos.len())))  // patched when `todos` changes
        --
        if $open {
            #[key(todo.id)]
            for todo in $todos { Text({todo.title}) }
        } else {
            Text("hidden")
        }
        --
        match $filter {
            Filter::All => Text("all"),
            Filter::Done => Text("done"),
        }
    ]
}
```

| Item in a list | Meaning |
| --- | --- |
| `if c { .. } else if c { .. } else { .. }` | conditional entities |
| `#[key(expr)] for pat in iter { .. }` | entities per item; unchanged rows keep their entities |
| `for pat in iter { .. }` | entities per item, identified by position |
| `match value { pat => .., }` | entities for the matching arm; rebuilt when the arm, or a binding it uses, changes |
| `let pat = expr;` | binding for the following items |

Without `$`, control flow is evaluated once. An `if` region rebuilds only when the
chosen branch changes; a `match` region rebuilds when the matched arm or a value its
pattern bound (and the arm uses) changes, and otherwise patches the arm in place.
Regions are `GhostNode`s with the `ui` feature, so they don't affect layout. Signals
(`signal`, `memo`, `get`, `set`, `update`, `increment`, ...) also work inside `webby`
components. `coat::shard(args, |args, world| scene)` re-renders a subtree when its
arguments change.

`coat::effect(|r: &Reads, commands| ..)` runs once when spawned and again when a
signal or ECS value it read changes. It may return a cleanup closure, which runs before
the next run and when its entity despawns.

## Change and delivery semantics

- Tracking uses Bevy change ticks: insertion and mutable dereferencing count as
  changes. Use `set_if_neq` to avoid publishing no-op writes.
- Multiple writes between runs are coalesced into one snapshot per source.
- The runtime runs in `PostUpdate` in `ReactiveSet::Publish`, before
  `UiSystems::Prepare` with the `ui` feature, so reaction output is laid out in the
  same frame. Writes made later in the frame are published on the next frame.
- Inside the runtime, publishing, components and regions repeat until nothing changes, up
  to `ReactiveSettings::max_passes` (default 8). A reaction that writes another
  tracked value is therefore delivered in the same frame. Nested components and regions
  settle within one pass. Hitting the limit logs a warning and increments
  `ReactiveDiagnostics::exhausted_frames`.
- When a type is first tracked by a scene, its publisher's first run reports the current
  values once, so the first watcher of a resource also receives its current value.
- `bind`, `bind_or_else` and `watch_immediate` replay the source's current value when the
  listener spawns; `watch` and `bind_deferred` only see later changes.
- A changed value is published only when something listens to it: a subscription, an
  entity observer on the source, or a global observer.
- Systems that read reaction output can run `.after(ReactiveSet::Settled)`. In tests,
  `bevy_radioactive::testing::settle(&mut app)` flushes commands and runs one frame.
- Removal is routed while the source still exists, so despawning a source reaches its
  listeners, but the handler runs at the next command flush, after the despawn has
  finished. Don't expect to read the source's components from a removal handler.
- Ordering between different local observers is unspecified.
- Sources with Bevy's `Disabled` marker still publish and route events.

## Performance model

`track::<T>` adds one publisher that uses Bevy's `Changed<T>` filter, clones one `T`
snapshot per changed source, and shares it through `Arc` with every listener of that
source. Each `watch`/`bind` creates one small subscription entity per source, owned
by (and despawned with) its listener, and despawned when its source is despawned.

`webby` components and `coat!` regions check the change ticks and signal versions of
their recorded dependencies each pass, and do no work when nothing changed. A component
re-render respawns its subtree, so keep components small; prefer `coat!` regions or
`bind` for hot paths and normal systems for thousands of entities changing every frame.

## Official Bevy references

- [Bevy 0.20: resources as components and next-generation scenes](https://bevy.org/news/bevy-0-20/)
- [BSN and scene observers](https://docs.rs/bevy/0.20.0/bevy/scene/)
- [Bevy observer semantics](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/observer/)
- [Bevy change detection](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/change_detection/)
- [Bevy ECS relationships](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/relationship/)
