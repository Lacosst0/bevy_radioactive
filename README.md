# `bevy_radioactive`

`bevy_radioactive` connects Bevy 0.20 state and entity events to local observers in
code-authored BSN scenes. A UI node can own the handler that keeps it synchronized
without a global system querying every possible view.

It has four layers:

- **Watching and binding** (`watch`, `bind`, `bind_or`, `on_removed`): source-routed
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
| 0.4 | 0.20 | 1.97.1 or newer |

```toml
[dependencies]
bevy = "0.20"
bevy_radioactive = { path = "../bevy_radioactive" }
```

The default `ui` feature orders the reactive runtime before Bevy UI layout and makes
`coat!` regions `GhostNode`s. See [MIGRATION.md](MIGRATION.md) for upgrading from 0.3.

## Quick start

Resources are singleton components in Bevy 0.20, so one API covers both. Add
`RadioactivePlugin`; `watch`, `bind` and `on_removed` start tracking their type the
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
            bind_or(
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
| `bind(source, \|&T, &mut C\|)` | same | writes component `C` on this entity |
| `bind_or(source, write, \|&mut C\|)` | same, plus removal/despawn | writes `C` for both cases |
| `on_removed(source, handler)` | `T` is removed or the source despawns | observer with `Removal<T>` input |
| `subscribe_to(source, handler)` | source receives entity event `E` | observer with `Subscription<E>` input |

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
            on_removed(#Enemy, |_: Removal<Health>, mut q: Query<&mut Text>| {
                for mut text in &mut q {
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
| `useReducer` | `let (state, dispatch) = cx.use_reducer(init, reducer); dispatch(action)` |
| signals | `cx.use_signal(init) -> Signal` |
| `useMemo` / `useEffect` / `useRef` | `cx.use_memo` / `cx.use_effect` / `cx.use_ref` |
| `createContext` / `useContext` | `cx.provide_context(value)` / `cx.use_context::<T>()` |
| store/selector hooks | `cx.use_resource::<R>()`, `cx.use_component::<C>(entity)` |

A component's output is a child of its entity; an entity without a UI `Node` becomes a
`GhostNode`, so `@counter(5)` works as a list item without affecting layout. A render
replaces the subtree; child components keep their hook state, matched by position or
key. Props other than `Entity` must be `Clone + Send + Sync + 'static`.

## `coat!`: reactive templates

`coat!` is `bsn!` with a small Rust subset in scene lists, modelled on
[Topcoat](https://github.com/tokio-rs/topcoat)'s `view!`. `$name` reads a `Signal`;
control flow that reads one becomes a region that rebuilds only itself, and a
component with `$` in its fields is patched in place.

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
        Text($(format!("{} todos", $todos.len())))   // patched when `todos` changes
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
| `match value { pat => .., }` | entities for the matching arm |
| `let pat = expr;` | binding for the following items |

Without `$`, control flow is evaluated once. Regions are `GhostNode`s with the `ui`
feature, so they don't affect layout. Signals (`signal`, `memo`, `get`, `set`,
`update`, `increment`, ...) also work inside `webby` components and `coat::effect`;
`coat::shard(args, |args, world| scene)` re-renders a subtree when its arguments change.

## Comparison with plain Bevy

`examples/comparison` builds the same Feathers counter (−1, count, +1, and a Reset
button shown while the count is not zero) once per style. Each style has its own
folder with two standalone files: `radioactive.rs` uses `bevy_radioactive`, and
`bevy.rs` does the same thing with plain Bevy. No file shares code with another.

| Style | `bevy_radioactive` | Plain Bevy |
| --- | --- | --- |
| Reactivity | `reactivity_radioactive` | `reactivity_bevy` |
| Subscriptions | `subscription_radioactive` | `subscription_bevy` |
| `webby` | `webby_radioactive` | `webby_bevy` |
| `coat!` | `coat_radioactive` | `coat_bevy` |

```sh
cargo run --example reactivity_radioactive --features comparison
```

The snippets below show only the parts that differ; layout and button styling are the
same in every file.

### Reactivity: `bind` vs change-detection systems

The count is a `Count` resource and the buttons write it with `ResMut<Count>`.

With `bevy_radioactive`, each node binds itself to the resource:

```rust,ignore
Text("0")
bind(Singleton, |count: &Count, text: &mut Text| **text = count.0.to_string())
--
@FeathersButton
bind(Singleton, |count: &Count, node: &mut Node| {
    node.display = if count.0 != 0 { Display::Flex } else { Display::None };
})
```

With plain Bevy, the nodes get marker components and global systems find them:

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

### Subscriptions: `subscribe_to` vs marker lookups

There is no shared state: the label owns a `LabelCount` component and announces each
new value with a `CountChanged` entity event.

With `bevy_radioactive`, the label subscribes to the buttons it cares about by name,
and the Reset button subscribes to the label:

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

With plain Bevy, each button observes its own `Activate` and finds the label by a
marker; the label observes its own `CountChanged` and finds the Reset button the same
way:

```rust,ignore
@FeathersButton
on(|_: On<Activate>, labels: CountLabels, commands: Commands| {
    update_count(labels, commands, |n| n + 1);
})
--
CountLabel
LabelCount(0)
on(|changed: On<CountChanged>, mut resets: Query<&mut Node, With<ResetButton>>| {
    for mut node in &mut resets {
        node.display = if changed.count != 0 { Display::Flex } else { Display::None };
    }
})
```

### `webby`: component state vs a re-render system

The count is local to the counter, and every change rebuilds its subtree.

With `bevy_radioactive`, the state is a hook and the runtime re-renders:

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

With plain Bevy, the state is a component on the root, the buttons write it by
entity, and a system respawns the subtree when it changes:

```rust,ignore
#[derive(Component, Clone, Default)]
struct CounterState(i32);

fn render_counters(
    counters: Query<(Entity, &CounterState), Changed<CounterState>>,
    mut commands: Commands,
) {
    for (root, state) in &counters {
        commands.entity(root).despawn_children().apply_scene(counter(root, state.0));
    }
}

fn counter(root: Entity, count: i32) -> impl Scene {
    let set_count = move |states: &mut Query<&mut CounterState>, value: i32| {
        if let Ok(mut state) = states.get_mut(root) {
            state.0 = value;
        }
    };
    bsn! {
        // ...
        on(move |_: On<Activate>, mut states: Query<&mut CounterState>| set_count(&mut states, count + 1))
        // ...
        {(count != 0).then(|| bsn_list! { /* Reset button */ })}
    }
}
```

### `coat!`: signals vs patching and spawning by hand

Only the label's `Text` is patched, and the Reset button exists only while the count
is not zero.

With `bevy_radioactive`, a `$` read patches the field and `if` owns the button:

```rust,ignore
let count = signal(|| 0);
coat! {
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

With plain Bevy, one system patches the label and another spawns or despawns the
button:

```rust,ignore
fn patch_count_label(count: Res<Count>, mut labels: Query<&mut Text, With<CountLabel>>) {
    for mut text in &mut labels {
        **text = count.0.to_string();
    }
}

fn toggle_reset_button(
    count: Res<Count>,
    columns: Query<Entity, With<CounterColumn>>,
    buttons: Query<Entity, With<ResetButton>>,
    mut commands: Commands,
) {
    let shown = !buttons.is_empty();
    if count.0 != 0 && !shown {
        for column in &columns {
            commands.entity(column).queue_spawn_related_scenes::<Children>(reset_button());
        }
    } else if count.0 == 0 && shown {
        for button in &buttons {
            commands.entity(button).despawn();
        }
    }
}
```

## Change and delivery semantics

- Tracking uses Bevy change ticks: insertion and mutable dereferencing count as
  changes. Use `set_if_neq` to avoid publishing no-op writes.
- Multiple writes between runs are coalesced into one snapshot per source.
- The runtime runs in `PostUpdate` in `ReactiveSet::Publish`, before
  `UiSystems::Prepare` with the `ui` feature, so reaction output is laid out in the
  same frame. Writes made later in the frame are published on the next frame.
- Inside the runtime, publishing, components and regions repeat until nothing changes, up
  to `ReactiveSettings::max_passes` (default 8). A reaction that writes another
  tracked value is therefore delivered in the same frame.
- When a type is first tracked by a scene, its publisher's first run reports the current
  values once, so the first watcher of a resource also receives its current value.
- `watch` and `bind` do not replay the current value when a listener attaches. Values
  inserted by the same scene are published as changes, so same-scene sources render
  on spawn; for an existing resource, initialize the node from it or use `webby`/`coat!`.
- Removal is delivered synchronously, while the source still exists.
- Ordering between different local observers is unspecified.
- Sources with Bevy's `Disabled` marker still publish and route events.

## Performance model

`track::<T>` adds one publisher that uses Bevy's `Changed<T>` filter, clones one `T`
snapshot per changed source, and shares it through `Arc` with every listener of that
source. Each `watch`/`bind` creates one small subscription entity per source, owned
by (and despawned with) its listener.

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
