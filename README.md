# `bevy_radioactive`

`bevy_radioactive` connects Bevy 0.20 state and entity events to local observers in
code-authored BSN scenes. A UI node can own the handler that keeps it synchronized
without a global system querying every possible view.

It has three layers:

- **Watching and binding** (`watch`, `bind`, `bind_or`, `on_removed`): source-routed
  change notifications for tracked components and resources.
- **Subscriptions** (`subscribe_to`): routing of any entity event from a source to
  listeners.
- **`webby`**: Solid/React-style effects that track what they read automatically and
  can render text, props, conditional children, and keyed lists.

## Compatibility

| `bevy_radioactive` | Bevy | Rust |
| --- | --- | --- |
| 0.4 | 0.20 | 1.97.1 or newer |

```toml
[dependencies]
bevy = "0.20"
bevy_radioactive = { path = "../bevy_radioactive" }
```

The default `ui` feature orders the reactive runtime before Bevy UI layout and enables
`webby::text` and `webby::show`. See [MIGRATION.md](MIGRATION.md) for upgrading from 0.3.

## Quick start

Resources are singleton components in Bevy 0.20, so one API covers both. Register each
watched type with `track` while building the app.

```rust,no_run
use bevy::{prelude::*, scene::CommandsSceneExt};
use bevy_radioactive::prelude::*;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counter(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Health(u32);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .init_resource::<Counter>()
        .track::<Counter>()
        .track::<Health>()
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

## `webby`

`webby` effects read state through a tracking context `cx` and re-run automatically
when anything they read changes. No `track` registration is needed: they work on any
component or resource and render once immediately when spawned.

```rust,ignore
use bevy_radioactive::webby::*;

fn hud() -> impl Scene {
    bsn! {
        Node
        Children [
            #Player Health(100)
            --
            Text("")
            text(#Player, |cx, player| format!("HP {}", cx.get::<Health>(player).map_or(0, |h| h.0)))
            prop(#Player, |cx, player| {
                let low = cx.get::<Health>(player).is_some_and(|h| h.0 < 30);
                TextColor(if low { RED.into() } else { WHITE.into() })
            })
            --
            Node
            each(
                (),
                |cx, ()| cx.resource::<Inventory>().map(|i| i.items.clone()).unwrap_or_default(),
                |item: &Item| item.id,
                |item, ()| { let name = item.name.clone(); bsn! { Text(name) } },
            )
            --
            Node
            when(#Player, |cx, player| cx.get::<Health>(player).is_none(), |_| bsn! { Text("GAME OVER") })
        ]
    }
}
```

| JS frameworks | `webby` |
| --- | --- |
| `createEffect` / `useEffect` | `effect(deps, \|cx, deps, commands\| ...)` |
| `createMemo`, JSX props | `prop(deps, \|cx, deps\| Component)` |
| `{value}` interpolation | `text(deps, \|cx, deps\| String)` |
| `style={{ display }}` | `show(deps, \|cx, deps\| bool)` |
| `<Show when>` | `when(deps, cond, render)` |
| `<Switch>` | `switch(deps, key, render)` |
| `<For each>` | `each(deps, items, key, render)` |
| `createSignal` / `useState` | any component or resource |

The first argument lists the entities the closure needs, since `#Name` cannot be
written inside a closure body: `()`, `#Name`, `[#A, #B]`, or a tuple. Which values the
effect depends on is tracked from its `cx.get`, `cx.this`, and `cx.resource` reads.
`when`, `switch`, and `each` own all children of their entity.

## Change and delivery semantics

- Tracking uses Bevy change ticks: insertion and mutable dereferencing count as
  changes. Use `set_if_neq` to avoid publishing no-op writes.
- Multiple writes between runs are coalesced into one snapshot per source.
- The runtime runs in `PostUpdate` in `ReactiveSet::Publish`, before
  `UiSystems::Prepare` with the `ui` feature, so reaction output is laid out in the
  same frame. Writes made later in the frame are published on the next frame.
- Inside the runtime, publishing and `webby` effects repeat until nothing changes, up
  to `ReactiveSettings::max_passes` (default 8). A reaction that writes another
  tracked value is therefore delivered in the same frame.
- `watch` and `bind` do not replay the current value when a listener attaches. Values
  inserted by the same scene are published as changes, so same-scene sources render
  on spawn; for an existing resource, initialize the node from it or use `webby`.
- Removal is delivered synchronously, while the source still exists.
- Ordering between different local observers is unspecified.
- Sources with Bevy's `Disabled` marker still publish and route events.

## Performance model

`track::<T>` adds one publisher that uses Bevy's `Changed<T>` filter, clones one `T`
snapshot per changed source, and shares it through `Arc` with every listener of that
source. Each `watch`/`bind` creates one small subscription entity per source, owned
by (and despawned with) its listener.

`webby` effects check the change ticks of their recorded dependencies each pass;
effects with no changed dependency do no work. Keep effects to presentation-scale
work and prefer normal systems for thousands of entities changing every frame.

## Official Bevy references

- [Bevy 0.20: resources as components and next-generation scenes](https://bevy.org/news/bevy-0-20/)
- [BSN and scene observers](https://docs.rs/bevy/0.20.0/bevy/scene/)
- [Bevy observer semantics](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/observer/)
- [Bevy change detection](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/change_detection/)
- [Bevy ECS relationships](https://docs.rs/bevy_ecs/0.20.0/bevy_ecs/relationship/)
