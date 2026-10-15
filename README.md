# `bevy_radioactive`

Reactive UI for Bevy 0.20 BSN scenes. A node declares what it depends on, right
where it is spawned, instead of a global system querying for it.

There are four styles. Each one is a separate module behind its own Cargo feature,
with its own plugin and runtime: they share no code, so use any one of them alone
or several side by side.

| Style | Feature | Plugin | Use it to |
| --- | --- | --- | --- |
| [`watch`](#watch) | `watch` | `WatchPlugin` | re-run a system when the components and resources it reads change |
| [`subscribe`](#subscribe) | `subscribe` | none | react to entity events fired on other entities |
| [`hooks`](#hooks) | `hooks` | `HooksPlugin` | write React-style function components |
| [`view`](#view) | `view` | `ViewPlugin` | write templates with typed state and reactive `if`/`for`/`match` |

## Install

| `bevy_radioactive` | Bevy | Rust |
| --- | --- | --- |
| 0.6 | 0.20 | 1.97.1 or newer |

```toml
[dependencies]
bevy = "0.20"
# All four styles:
bevy_radioactive = { path = "../bevy_radioactive" }
# Or only the ones you use:
# bevy_radioactive = { path = "../bevy_radioactive", default-features = false, features = ["view", "ui"] }
```

The default `ui` feature runs updates before Bevy UI layout, so changes show in the
same frame, and makes component hosts and `view!` regions `GhostNode`s so they don't
affect layout. See [MIGRATION.md](MIGRATION.md) for upgrading from 0.5.

## `watch`

`watch` takes an ordinary Bevy system and attaches it to the current entity. The
system runs once when the scene spawns, and again whenever something it reads
changes. Dependencies come from the system's parameters, so there are no sources to
name and nothing to register: `Res<T>`, `&T` in queries, and query filters such as
`With<T>`, `Without<T>` and `Changed<T>` all count. Resources are singleton
components in Bevy 0.20, so they work the same way as components.

```rust,ignore
use bevy::{prelude::*, scene::CommandsSceneExt};
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Default)]
struct Score(u32);

#[derive(Component, Clone, Default)]
struct Player;

#[derive(Component, Clone)]
struct Health(u32);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, WatchPlugin))
        .init_resource::<Score>()
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
            commands.spawn_scene(hud());
        })
        .run();
}

fn hud() -> impl Scene {
    bsn! {
        Node
        Children [
            Player Health(100)
            --
            Text("0")
            watch(|score: Res<Score>, mut text: This<&mut Text>| {
                if let Some(mut text) = text.get_mut() {
                    **text = score.0.to_string();
                }
            })
            --
            Text("")
            watch(|player: Query<&Health, With<Player>>, mut text: This<&mut Text>| {
                if let Some(mut text) = text.get_mut() {
                    **text = match player.single() {
                        Ok(hp) => format!("HP {}", hp.0),
                        Err(_) => "DEAD".into(),
                    };
                }
            })
        ]
    }
}
```

| System reads | Re-runs when |
| --- | --- |
| `Res<T>` | resource `T` is inserted or changed |
| `Option<Res<T>>` | same, or `T` is removed |
| `Query<&T>`, `Option<&T>`, `Has<T>` | `T` is inserted, changed or removed on any entity |
| `With<T>`, `Without<T>`, `Changed<T>`, `Added<T>` | same, for the filtered `T` |
| `ResMut<T>`, `&mut T` | never: values a system writes are not dependencies |
| `Commands`, `Local<T>`, `This<..>` | never |

`This<D, F = ()>` queries the entity that declared the watch: `get()`, `get_mut()`
and `entity()`. One entity can declare any number of watches, and they despawn with
it.

- Insertion and mutable access count as changes; use `set_if_neq` to skip no-op
  writes. Several writes in one frame re-run a watch once.
- A watch that writes a value read by another watch re-runs it in the same frame.
  Avoid two watches that write each other's dependencies, and writes through
  `Commands` to values the same watch reads: they re-run every frame.
- A missing `Res<T>` fails like in any Bevy system; use `Option<Res<T>>` to render
  before the resource exists.
- Systems that read the whole world (`&World`, `Query<EntityRef>`) report a scene
  error, since there is nothing to infer.
- A system reading `Res<Time>` re-runs every frame. Changes are found by scanning
  the change ticks of each dependency, so prefer plain systems for values that change
  on thousands of entities every frame.

Examples: `watch_component`, `watch_resource`.

## `subscribe`

BSN's `on(...)` observes events on the entity that declares it. `subscribe_to`
observes events fired on another entity, and `bind_event` writes a component of the
declaring entity in response. No plugin is needed.

```rust,ignore
use bevy::{prelude::*, ui_widgets::Activate};
use bevy_radioactive::subscribe::*;

#[derive(Component, Clone, Default)]
struct Clicks(u32);

fn toolbar() -> impl Scene {
    bsn! {
        Node
        Children [
            #Save Button
            --
            Clicks(0)
            bind_event(#Save, |_: &Activate, clicks: &mut Clicks| clicks.0 += 1)
            --
            Text("")
            subscribe_to(#Save, |click: Subscription<Activate>, mut texts: Query<&mut Text>| {
                if let Ok(mut text) = texts.get_mut(click.target()) {
                    **text = "Saved".into();
                }
            })
        ]
    }
}
```

`Subscription<E>` dereferences to the event; `target()` is the declaring entity and
`source()` the entity the event fired on. A source is a `#Name`, an `Entity`, or a list
such as `[#A, #B]`. Subscriptions despawn with the entity that declared them.

Example: `subscribe`.

## `hooks`

Mark a function with `#[component]` and use it like a scene function: `@counter(5)`.
The optional `cx` argument is supplied by the runtime; the others are props. The
component re-renders when its state, or ECS data it read through `cx`, changes.

```rust,ignore
use bevy::prelude::*;
use bevy_radioactive::hooks::*;

#[derive(Component, Clone)]
struct Health(u32);

#[component]
fn counter(cx: &Cx, start: i32) -> impl Scene {
    let (count, set_count) = cx.use_state(|| start);
    bsn! {
        Button
        on(move |_: On<Pointer<Click>>| set_count(count + 1))
        Children [ Text({count.to_string()}) ]
    }
}

#[component]
fn health_bar(cx: &Cx, player: Entity) -> impl Scene {
    let hp = cx.use_component::<Health>(player).map_or(0, |hp| hp.0);
    bsn! { Text({format!("HP {hp}")}) }
}

fn hud() -> impl Scene {
    bsn! {
        Node
        Children [
            #Player Health(100)
            --
            @counter(5)            // <Counter start={5} />
            --
            @health_bar(#Player)   // `Entity` props accept `#Name`
        ]
    }
}

// app.add_plugins(HooksPlugin)
```

| React | `hooks` |
| --- | --- |
| `<Counter start={5} />` | `@counter(5)` |
| `<Row key={id} />` | `row(..).key(id)` or `@keyed(id, row(..))` |
| inline component | `component(\|cx\| bsn! { .. })` |
| `useState` / `useReducer` | `cx.use_state(init)` / `cx.use_reducer(init, reducer)` |
| `useMemo` / `useEffect` / `useRef` | `cx.use_memo` / `cx.use_effect` / `cx.use_ref` |
| `createContext` / `useContext` | `cx.provide_context(value)` / `cx.use_context::<T>()` |
| store/selector hooks | `cx.use_resource::<R>()` / `cx.use_component::<C>(entity)` |

The rendered scene is a child of the component's entity. A render replaces that
subtree; child components keep their state, matched by position or key. Call hooks
in the same order on every render. Props other than `Entity` must be
`Clone + Send + Sync + 'static`. Keep components small: a render respawns the
subtree.

Examples: `hooks_counter`, `hooks_todos`.

## `view`

`view!` is `bsn!` plus reactive control flow, modelled on
[Topcoat](https://github.com/tokio-rs/topcoat)'s `view!`. A `#[component]` function
declares what its template reacts to in the types of its parameters, and the runtime
supplies them. `$name` reads one. Control flow that reads one becomes a region that
rebuilds only itself, and a component with `$` in its fields is patched in place.

```rust,ignore
use bevy::prelude::*;
use bevy_radioactive::view::*;

#[derive(Clone, PartialEq)]
struct Todo { id: u32, title: &'static str }

#[derive(Resource, Clone, Default)]
struct Todos(Vec<Todo>);

#[component]
fn todo_list(todos: Watch<Res<Todos>>, #[init(true)] open: Watch<Local<bool>>) -> impl Scene {
    view! {
        Node
        Children [
            Button
            on(move |_: On<Pointer<Click>>| open.set(!$open))
            --
            Text($(format!("{} todos", $todos.0.len())))   // patched when `Todos` changes
            --
            if $open {
                #[key(todo.id)]
                for todo in $todos.0 { Text({todo.title}) }
            } else {
                Text("hidden")
            }
        ]
    }
}

// app.add_plugins(ViewPlugin)
// bsn! { @todo_list() }
```

| Parameter | Value |
| --- | --- |
| `Watch<Local<T>>` | state of this instance, starting at `T::default()`; `set`, `update`, `set_if_neq` |
| `#[init(expr)] state: Watch<Local<T>>` | same, starting at `expr`; props are in scope |
| `Watch<Res<T>>` | resource `T`, read-only; updates when the resource changes |
| `Watch<Option<Res<T>>>` | resource `T`, or `None` while it doesn't exist |
| anything else | a prop: `@row(todo)` passes `todo` |

| Item in a list | Meaning |
| --- | --- |
| `if c { .. } else if c { .. } else { .. }` | conditional entities |
| `#[key(expr)] for pat in iter { .. }` | entities per item; unchanged rows keep their entities |
| `for pat in iter { .. }` | entities per item, identified by position |
| `match value { pat => .., }` | entities for the matching arm |
| `let pat = expr;` | binding for the following items |

Without `$`, control flow is evaluated once. Watches have `get` and `with`, and
`effect(|commands| ..)` re-runs a side effect when a watch it read changes. Only
`Local` state can be written from the view; change resources through the ECS as usual,
and every view watching them follows. Each instance of a component has its own
`Local` state. Resource watches are bound when the scene spawns, so read them with `$`
in fields, regions and handlers, not in `{ .. }` values evaluated while the scene is
built.

Examples: `view_todos`, `view_tabs`.

## Comparison with plain Bevy

`examples/comparison` builds the same Feathers counter (−1, count, +1, and a Reset
button shown while the count is not zero) five times: once in plain Bevy and once per
style. Each file stands alone; none shares code with another.

| Example | Built with |
| --- | --- |
| `comparison_bevy` | plain Bevy |
| `comparison_watch` | `watch` |
| `comparison_subscribe` | `subscribe` |
| `comparison_hooks` | `hooks` |
| `comparison_view` | `view` |

```sh
cargo run --example comparison_bevy --features comparison
cargo run --example comparison_view --features comparison
```

The snippets below show only the parts that differ; layout and button styling are the
same in every file.

### Plain Bevy

The count is a `Count` resource and the buttons write it with `ResMut<Count>`. The
nodes get marker components, and global systems find them when `Count` changes:

```rust,ignore
app.add_systems(
    Update,
    (sync_count_label, sync_reset_button).run_if(resource_changed::<Count>),
);

fn sync_count_label(count: Res<Count>, mut labels: Query<&mut Text, With<CountLabel>>) {
    for mut text in &mut labels {
        **text = count.0.to_string();
    }
}

fn sync_reset_button(count: Res<Count>, mut buttons: Query<&mut Node, With<ResetButton>>) {
    for mut node in &mut buttons {
        node.display = if count.0 != 0 { Display::Flex } else { Display::None };
    }
}
```

### `watch`

Same resource and buttons, but each node runs a system that reads `Res<Count>`, so
it re-runs whenever the count changes:

```rust,ignore
Text("0")
watch(|count: Res<Count>, mut text: This<&mut Text>| {
    if let Some(mut text) = text.get_mut() {
        **text = count.0.to_string();
    }
})
--
@FeathersButton
watch(|count: Res<Count>, mut node: This<&mut Node>| {
    if let Some(mut node) = node.get_mut() {
        node.display = if count.0 != 0 { Display::Flex } else { Display::None };
    }
})
```

### `subscribe`

There is no shared state: the label owns a `LabelCount` component and announces each
new value with a `CountChanged` entity event. The label subscribes to the buttons by
name, and the Reset button subscribes to the label:

```rust,ignore
#Label
LabelCount(0)
subscribe_to(#Plus, |click: Subscription<Activate>, labels: CountLabels, commands: Commands| {
    update_count(click.target(), labels, commands, |n| n + 1);
})
--
#Reset @FeathersButton
subscribe_to(#Label, |changed: Subscription<CountChanged>, mut nodes: Query<&mut Node>| {
    if let Ok(mut node) = nodes.get_mut(changed.target()) {
        node.display = if changed.count != 0 { Display::Flex } else { Display::None };
    }
})
```

### `hooks`

The count is component state, and every change re-renders the counter:

```rust,ignore
#[component]
fn counter(cx: &Cx) -> impl Scene {
    let (count, set_count) = cx.use_state(|| 0);
    let (decrement, increment, reset) = (set_count.clone(), set_count.clone(), set_count);
    bsn! {
        // ...
        on(move |_: On<Activate>| increment(count + 1))
        // ...
        {(count != 0).then(|| bsn_list! { /* Reset button */ })}
    }
}
```

### `view`

Only the label's `Text` is patched, and the Reset button exists only while the count
is not zero: a `$` read patches the field and `if` owns the button:

```rust,ignore
#[component]
fn counter(count: Watch<Local<i32>>) -> impl Scene {
    view! {
        // ...
        on(move |_: On<Activate>| count.set($count + 1))
        --
        Text($(format!("{}", $count)))
        // ...
        if $count != 0 {
            @FeathersButton
            on(move |_: On<Activate>| count.set(0))
            Children [ @caption("Reset") ]
        }
    }
}
```

## Scheduling and performance

Each plugin runs in its own system set in `PostUpdate` — `WatchSystems`,
`HooksSystems` and `ViewSystems` — before UI layout with the `ui` feature. Order
systems that write watched values `.before(...)` the set to show them the same frame.
Each runtime repeats until nothing changes, up to 8 rounds per frame.

- `watch` checks the change ticks of every component and resource its systems
  depend on once per round, and runs only the watches whose dependencies changed.
- `subscribe` adds one observer per event type and one small entity per
  subscription.
- `hooks` components and `view` regions check what they read each round and do
  nothing when it didn't change. Prefer `view` regions or `watch` for hot paths, and
  plain systems for thousands of entities changing every frame.

## Official Bevy references

- [Bevy 0.20: resources as components and next-generation scenes](https://bevy.org/news/bevy-0-20/)
- [BSN and scene observers](https://docs.rs/bevy/0.20.0/bevy/scene/)
- [Bevy observer semantics](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/observer/)
- [Bevy change detection](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/change_detection/)
