# `bevy_radioactive`

Reactive UI for Bevy 0.20 BSN scenes. A node declares what it depends on, right
where it is spawned, instead of a global system querying for it.

There are four styles. Each one is a separate module behind its own Cargo feature,
with its own plugin and runtime: they share no code, so use any one of them alone
or several side by side.

| Style | Feature | Plugin | Use it to |
| --- | --- | --- | --- |
| [`watch`](#watch) | `watch` | `WatchPlugin` | react to tracked components and resources changing |
| [`subscribe`](#subscribe) | `subscribe` | none | react to entity events fired on other entities |
| [`hooks`](#hooks) | `hooks` | `HooksPlugin` | write React-style function components |
| [`view`](#view) | `view` | `ViewPlugin` | write templates with signals and reactive `if`/`for`/`match` |

## Install

| `bevy_radioactive` | Bevy | Rust |
| --- | --- | --- |
| 0.5 | 0.20 | 1.97.1 or newer |

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
affect layout. See [MIGRATION.md](MIGRATION.md) for upgrading from 0.4.

## `watch`

Derive `Track` on a component or resource, then `watch` it. The handler's first
parameter says what to react to, written like the system parameter you'd read it
with: `Watch<Query<&T>>` for components, `Watch<Res<T>>` for a resource. `bind`
writes a component of the current entity from the latest value instead.

```rust,ignore
use bevy::{prelude::*, scene::CommandsSceneExt};
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Default, Track)]
struct Score(u32);

#[derive(Component, Clone, Track)]
struct Health(u32);

#[derive(Component, Clone)]
struct Enemy;

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
            Text("0")
            bind(|score: &Score, text: &mut Text| **text = score.0.to_string())
            --
            watch(|hp: Watch<Query<&Health>>, mut commands: Commands| {
                info!("{:?} has {} HP", hp.entity(), hp.0);
            })
            --
            watch(|hp: Watch<Query<&Health, With<Enemy>>>, mut score: ResMut<Score>| {
                if hp.0 == 0 { score.0 += 1; }
            })
            --
            watch(|gone: Removed<Query<&Health>>| info!("{:?} is gone", gone.entity()))
        ]
    }
}
```

| Handler input | Runs when |
| --- | --- |
| `Watch<Query<&T>>` | `T` is inserted or changed on an entity |
| `Watch<Query<&T, F>>` | the same, for entities matching filter `F` (`With<Enemy>`, `Allow<Disabled>`, ..) |
| `Watch<Res<T>>` | resource `T` is inserted or changed |
| `Removed<Query<&T, F>>` / `Removed<Res<T>>` | `T` is removed, or its entity despawns |

`Watch` dereferences to the new value; `entity()` is the entity whose `T` changed and
`target()` the entity that declared the watch. Other handler parameters are ordinary
system parameters. `bind(|&T, &mut C|)` writes `C` on its entity when the scene spawns
and whenever `T` changes, so it suits resources and components that exist once.
`app.watch(handler)` adds a watch that isn't tied to a scene.

- A `Query` selector matches exactly the entities that query would return, so
  disabled entities are skipped unless the filter allows them.
- Insertion and mutable access count as changes; use `set_if_neq` to skip no-op
  writes. Several writes in one frame are delivered once, with the latest value.
- A handler that writes another tracked value is delivered in the same frame.
- Watches despawn with the entity that declared them.

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
`source()` the entity the event fired on. A source is a `#Name`, an `Entity`, or a
list such as `[#A, #B]`. Subscriptions despawn with the entity that declared them.

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
| `use(promise)` with Suspense | `cx.use_task(deps, \|d\| async { .. })` returns `TaskState` |

The rendered scene is a child of the component's entity. A render replaces that
subtree; child components keep their state, matched by key, or else by the `@name(..)`
call that created them, so a conditional sibling doesn't shift their state. Call hooks
in the same order on every render. Props other than `Entity` must be
`Clone + Send + Sync + 'static`. Keep components small: a render respawns the
subtree.

Examples: `hooks_counter`, `hooks_todos`.

## `view`

`view!` is `bsn!` plus signals and reactive control flow, modelled on
[Topcoat](https://github.com/tokio-rs/topcoat)'s `view!`. `$name` reads a signal.
Control flow that reads one becomes a region that rebuilds only itself, and a
component with `$` in its fields is patched in place.

```rust,ignore
use bevy::prelude::*;
use bevy_radioactive::view::*;

#[derive(Clone, PartialEq)]
struct Todo { id: u32, title: &'static str }

fn todo_list() -> impl Scene {
    let todos = signal(|| vec![Todo { id: 1, title: "Write docs" }]);
    let open = signal(|| true);
    view! {
        Node
        Children [
            Button
            on(move |_: On<Pointer<Click>>| open.set(!$open))
            --
            Text($(format!("{} todos", $todos.len())))   // patched when `todos` changes
            --
            if $open {
                #[key(todo.id)]
                for todo in $todos { Text({todo.title}) }
            } else {
                Text("hidden")
            }
        ]
    }
}

// app.add_plugins(ViewPlugin)
```

| Item in a list | Meaning |
| --- | --- |
| `if c { .. } else if c { .. } else { .. }` | conditional entities |
| `#[key(expr)] for pat in iter { .. }` | entities per item; unchanged rows keep their entities |
| `for pat in iter { .. }` | entities per item, identified by position |
| `for $item in iter { .. }` (keyed or not) | each row gets its item as a signal and is patched in place instead of rebuilt |
| `match value { pat => .., }` | entities for the matching arm |
| `let pat = expr;` | binding for the following items |

Without `$`, control flow is evaluated once. Signals have `get`, `set`, `update`,
`set_if_neq` and `with`; `memo(|| ..)` derives a cached value from signals, and
`effect(|commands| ..)` re-runs a side effect when a signal it read changes. Signals
are `Send + Sync`, so they can also live in components or resources and be set from
systems. Writing a signal from inside its own `with`/`update` closure panics instead of
deadlocking.

`CopySignal<T>` is a signal behind a `Copy` handle, so closures capture it without
`clone`s. Its value lives on its own entity, linked to an owner and despawned with it:
create one with `commands.spawn_signal(owner, value)`, or with `copy_signal(|| value)`
plus `owns(signal)` on an entity of the scene. After the owner despawns, `try_get`
returns `None` and `get` panics.

`#[derive(Store)]` on a struct generates `{Name}Store` with one signal per field, so a
reader of one field doesn't update when another changes; `set` and `update` write the
whole value and notify only the fields that changed.

```rust,ignore
#[derive(Clone, PartialEq, Store)]
struct Player { name: String, health: u32 }

let player = PlayerStore::new(Player { name: "Ada".into(), health: 10 });
let health = player.health.clone();
view! { Text($(format!("HP {}", $health))) }   // not patched when `name` changes
```

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

Same resource and buttons, but each node binds itself to the resource:

```rust,ignore
Text("0")
bind(|count: &Count, text: &mut Text| **text = count.0.to_string())
--
@FeathersButton
bind(|count: &Count, node: &mut Node| {
    node.display = if count.0 != 0 { Display::Flex } else { Display::None };
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
let count = signal(|| 0);
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
```

## Scheduling and performance

Each plugin runs in its own system set in `PostUpdate` — `WatchSystems`,
`HooksSystems` and `ViewSystems` — before UI layout with the `ui` feature. Order
systems that write watched values `.before(...)` the set to show them the same frame.
Each runtime repeats until nothing changes, up to 8 rounds per frame; if changes are
still pending after that, it logs a warning and continues next frame.

- `watch` adds one publisher per tracked type using Bevy's `Changed<T>` filter. Each
  changed value is cloned once and shared with every watch that selects it.
- `subscribe` adds one observer per event type and one small entity per
  subscription.
- `hooks` components and `view` regions check what they read each round and do
  nothing when it didn't change. Prefer `view` regions or `bind` for hot paths, and
  plain systems for thousands of entities changing every frame.

## Official Bevy references

- [Bevy 0.20: resources as components and next-generation scenes](https://bevy.org/news/bevy-0-20/)
- [BSN and scene observers](https://docs.rs/bevy/0.20.0/bevy/scene/)
- [Bevy observer semantics](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/observer/)
- [Bevy change detection](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/change_detection/)
