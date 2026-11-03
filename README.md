# `bevy_radioactive`

Reactive UI for Bevy 0.20 BSN scenes. A node declares what it depends on, right
where it is spawned, instead of a global system querying for it.

There are four styles. Each one is a separate module behind its own Cargo feature,
with its own plugin and runtime, so use any one of them alone or several side by
side. [`reactive`](#reactive) is the newest and covers what the
others do in one ECS-native API: watching, `if`/`else`, `match`, keyed lists, event
handlers, context and cleanup.

| Style | Feature | Plugin | Use it to |
| --- | --- | --- | --- |
| [`reactive`](#reactive) | `reactive` | `ReactivePlugin` | do all of the below with components as state and entities as scope |
| [`subscribe`](#subscribe) | `subscribe` | none | react to entity events fired on other entities |
| [`hooks`](#hooks) | `hooks` | `HooksPlugin` | write React-style function components |
| [`view`](#view) | `view` | `ViewPlugin` | write templates with signals and reactive `if`/`for`/`match` |

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

## `reactive`

One API for reactivity and control flow where the ECS is the reactive graph:
state is a component or resource, context is an ancestor's component, a branch
is an entity, and cleanup is despawning.

Write scenes with `rsn!`: it is `bsn!` plus Rust `if`/`match`/`for` and `$` reads.
**One rule tells reactive from not: whatever contains a `$` re-runs when what it read
changes; everything else runs once, when the scene spawns.**

| `$` read | Reads | Example |
| --- | --- | --- |
| `$Ref<Type>(#Name)` | component `Type` on the nearest entity named `#Name` (this entity and its subtree, then each ancestor and its subtree) | `$Ref<Count>(#Counter).0` |
| `$Ref<Type>(entity)` | `Type` on that entity (skipped while it has none) | `$Ref<Health>(player).0` |
| `$Mut<Type>(#Name)`, `$Mut<Type>(entity)` | the same, writable; only in `on(..)` | `$Mut<Count>(#Counter).0 += 1` |
| `$Ref<Type>("Name")` | like `#Name`, for an entity named in another scene (not checked at compile time) | `$Ref<Theme>("App").0` |
| `$Res<T>`, `$ResMut<T>`, `$Local<T>`, `$Query<..>`, `$Single<..>`, any system param | that parameter, as in a Bevy system | `$Res<Score>.0`, `$Query<&Enemy>.iter().count()` |

A `#Name` must be declared in the same `rsn!`, so a typo is a compile error. In debug
builds, a closure whose name matches nothing (or whose entity lacks the component) logs
one warning; it is skipped either way.

```rust,ignore
use bevy::{prelude::*, ui_widgets::Activate};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Default)]
struct Count(i32);

#[derive(Component, Clone, PartialEq)]
struct Todo { title: String, done: bool }

#[derive(Resource, Clone, Copy, PartialEq, Default)]
enum Tab { #[default] Home, Settings }

fn counter() -> impl Scene {
    rsn! {
        #Counter
        Node
        Count(0)                                          // state; every @counter() has its own
        Children [
            Button
            on(|_: On<Activate>| $Mut<Count>(#Counter).0 += 1)  // writes this counter's Count
            --
            Text({ $Ref<Count>(#Counter).0.to_string() })        // re-renders when it changes
            --
            Text({ format!("{} enemies", $Query<(), With<Enemy>>.iter().count()) })
            --
            if $Ref<Count>(#Counter).0 > 10 {
                Text("That's a lot")
            } else if $Ref<Count>(#Counter).0 < 0 {
                Text("Negative")
            }
        ]
    }
}

fn app_body() -> impl Scene {
    rsn! {
        Node
        Children [
            match *$Res<Tab> {
                Tab::Home => @counter(),
                Tab::Settings => Text("Settings"),
            }
            --
            // One row per entity with a Todo, added and removed as they come and go.
            for todo in $Query<&Todo> {
                Text({ todo.title.clone() })
            }
            --
            // A plain loop: runs once.
            for i in 0..3 { Text({ i.to_string() }) }
        ]
    }
}

// app.add_plugins(ReactivePlugin)
```

| In `rsn!` | Becomes |
| --- | --- |
| `Comp(.. $ ..)` | a `watch` that rebuilds `Comp` |
| `Comp { a: .. $ .., b: .. }` | `Comp { b: .. }` plus a `watch` that patches `a` |
| `Type::new(.. $ ..)`, `func(.. $ ..)`, `Comp(..).with(.. $ ..)` | a `watch` that rebuilds the component the expression returns |
| `Comp { a: Inner { x: .. $ .. } }` | patches only `a.x`, as `bsn!` does for nested values |
| `on(\|e: On<E>\| .. $ ..)` | a handler; write with `$Mut<Type>(#Name)`, `$Mut<Type>(e)` or `$ResMut<T>` (`$Mut<Count>(#Counter).0 += 1`) |
| `if`/`else if`/`else` | a region showing one branch, rebuilt when the branch changes |
| `match value { pat => .., }` | a region per arm; arms are `pat => scene,` or `pat => { scene -- scene }` |
| `for pat in $Query<D, F> { .. }` | one row per matching entity; see below |
| `#[key(expr)] for pat in iter { .. }` | a keyed list; rows are kept, moved or rebuilt per key |
| `for pat in iter { .. }` | a list by position |
| anything else | passed to `bsn!` unchanged |

- `for pat in $Query<D, F>` keeps one row per entity. `pat` binds an owned copy of
  the item (`&T` becomes `T`), and the row is rebuilt when it changes, so `T` needs
  `Clone + PartialEq`. To update rows in place instead, loop over entities and read
  them: `for e in $Query<Entity, With<Todo>> { Text({ $Ref<Todo>(e).title.clone() }) }`.
- Other `for` loops need owned items (`$Res<Todos>.0.clone()`, a range, a `Vec`) that
  implement `PartialEq`. Loop and match bindings can be used any number of times.
- `rsn_list!` is the list form (`a -- b`), like `bsn_list!`.
- A control flow item becomes a region: an entity (a `GhostNode` with the `ui`
  feature) whose children are the content.

### The functions under `rsn!`

`rsn!` expands to functions you can use directly in `bsn!` or plain Rust. They
take closures with ordinary Bevy system parameters:

```rust,ignore
bsn! {
    Text("")
    watch(|enemies: Query<(), With<Enemy>>| Text(format!("{} enemies", enemies.iter().count())))
    --
    Button
    handle(|_: &Activate, count: &mut Count| count.0 += 1)
    --
    @when(|count: Ref<Count>| count.0 > 10, || bsn! { Text("That's a lot") })
}
```

| In a closure | Re-runs it? | Meaning |
| --- | --- | --- |
| `c: Ref<T>` / `Option<Ref<T>>` | yes | the nearest component `T`: this entity, its `Source`, then its ancestors |
| `Res<T>` / `Option<Res<T>>` | yes | resource `T` |
| `Query`, `Single`, `Populated` | yes | when a component they read or filter on changes, is added or removed |
| `c: &mut C` | no | the nearest `C`, written in place |
| `e: Entity` | no | the entity that declared the closure |
| `ResMut`, `Commands`, `Local`, messages | no | ordinary system parameters |
| `Untracked<P>` | no | any system parameter, without re-running; also for custom `SystemParam`s |

| Item | Kind | Use it to |
| --- | --- | --- |
| `watch(\|..\| ..)` | template | re-run on change; a returned component or bundle is inserted (a derived value) |
| `handle(\|e: &E, ..\| ..)` | template | run when entity event `E` reaches this entity |
| `@when(cond, \|\| ..)` | scene | show content while `cond` holds |
| `@if_else(cond, \|\| then, \|\| otherwise)` | scene | `if` / `else` |
| `@switch(\|..\| key, \|key\| ..)` | scene | `match`; arms are boxed with `.into()` |
| `@each(\|..\| items, \|item\| key, \|item\| ..)` | scene | a keyed list |
| `@each_indexed(\|..\| items, \|item\| ..)` | scene | a list by position |
| `Source(#Player)` | component | make this subtree read and write another entity's components |
| `Nearest<D>` | system param | the same lookup from ordinary systems and observers |

- Outside scenes, attach templates with `commands.entity(e).react(watch(..))` and
  spawn regions like any scene: `commands.spawn_scene(when(..))`.
- For list rows that update in place, list entities and give each row
  `Source({entity})`; `Ref<Todo>` in the row then reads that entity's `Todo`.
- Reactions are entities linked to the entity that declared them (`ReactionOf`), so
  they despawn with it, and replacing a branch despawns everything it declared. Use
  Bevy's `on(|_: On<Despawn>| ..)` for cleanup code.
- Content exists right after its scene spawns. Later updates run in
  `ReactiveSystems` in `PostUpdate`; outer regions run before the reactions inside
  them, and chained reactions settle in the same frame.

Examples: `reactive_counters`, `reactive_todos`, `comparison_reactive`.

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
| `comparison_reactive` | `reactive` |
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

### `reactive`

The count is a component on the counter's root, so each counter has its own.
`$Ref<Count>(#Counter)` reads it, and the `if` owns the Reset button:

```rust,ignore
rsn! {
    #Counter
    Count(0)
    // ...
    on(|_: On<Activate>| $Mut<Count>(#Counter).0 += 1)
    // ...
    Text({ $Ref<Count>(#Counter).0.to_string() })
    // ...
    if $Ref<Count>(#Counter).0 != 0 {
        @FeathersButton
        on(|_: On<Activate>| $Mut<Count>(#Counter).0 = 0)
        Children [ @caption("Reset") ]
    }
}
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

Each plugin runs in its own system set in `PostUpdate` — `ReactiveSystems`,
`HooksSystems` and `ViewSystems` — before UI layout with the `ui`
feature. Order systems that write watched values `.before(...)` the set to show them
the same frame. Each runtime repeats until nothing changes, up to 8 rounds per frame
(16 for `reactive`); if changes are still pending after that, it logs a warning and
continues next frame.

- `reactive` checks each reaction's inputs once per round by change tick (a few
  lookups each) and runs only those that changed. `Query` parameters scan the
  entities with the components they read once per round, so prefer `Ref<T>` and
  `Res<T>` on hot paths.
- `subscribe` adds one observer per event type and one small entity per
  subscription.
- `hooks` components and `view` regions check what they read each round and do
  nothing when it didn't change. Prefer `reactive` or `view` regions for hot paths, and
  plain systems for thousands of entities changing every frame.

## Official Bevy references

- [Bevy 0.20: resources as components and next-generation scenes](https://bevy.org/news/bevy-0-20/)
- [BSN and scene observers](https://docs.rs/bevy/0.20.0/bevy/scene/)
- [Bevy observer semantics](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/observer/)
- [Bevy change detection](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/change_detection/)
