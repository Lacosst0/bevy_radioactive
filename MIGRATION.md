# Migrating from 0.5 to 0.6

## `watch`

`watch` now takes a plain Bevy system and works out what it depends on from the
system's parameters. The system runs once when the scene spawns, then again whenever
a component or resource it reads (or filters on) changes, is inserted or is removed.
There are no sources to name and no types to register.

| 0.5 | 0.6 |
| --- | --- |
| `watch(Singleton, \|r: Reaction<T>\| ..)` | `watch(\|t: Res<T>\| ..)` |
| `watch(#Source, \|r: Reaction<T>\| ..)` | `watch(\|q: Query<&T, With<Source>>\| ..)` with a marker component |
| `watch([#A, #B], handler)` | one query matching both entities |
| `bind(source, \|t: &T, c: &mut C\| ..)` | `watch(\|.., mut this: This<&mut C>\| ..)` |
| `bind_or(source, write, removed)` | one `watch`; handle the query not matching |
| `watch_removed(source, handler)` | a `watch` re-runs on removal; check whether the query still matches |
| `Reaction<T>`, `Removal<T>` | read the value through `Res<T>` or a `Query` |
| `reaction.target()` | `This::entity()` |
| `reaction.source()` | `Query<(Entity, &T)>` |
| `Singleton`, `ValueSource` | removed |
| `WatchAppExt`, `app.track::<T>()` | removed; nothing needs registering |
| `ValueChanged<T>`, `ValueRemoved<T>` observers | a normal system with `Changed<T>`, or `On<Remove, T>` |

Before:

```rust,ignore
#Player Health(100)
--
Text("")
bind_or(
    #Player,
    |hp: &Health, text: &mut Text| **text = hp.0.to_string(),
    |text: &mut Text| **text = "DEAD".into(),
)
--
Text("")
bind(Singleton, |score: &Score, text: &mut Text| **text = score.0.to_string())
```

After:

```rust,ignore
Player Health(100)
--
Text("")
watch(|player: Query<&Health, With<Player>>, mut text: This<&mut Text>| {
    if let Some(mut text) = text.get_mut() {
        **text = player.single().map_or("DEAD".into(), |hp| hp.0.to_string());
    }
})
--
Text("")
watch(|score: Res<Score>, mut text: This<&mut Text>| {
    if let Some(mut text) = text.get_mut() {
        **text = score.0.to_string();
    }
})
```

Behavior changes:

- A watch runs once when its scene spawns, so nodes no longer need initializing from
  the current value by hand.
- Values a watch only writes (`ResMut<T>`, `&mut T`, `This<&mut T>`) are not
  dependencies, so a watch doesn't re-run from its own writes. Writes through
  `Commands` are not visible in the system's parameters and do re-run it if it reads
  the same value.
- A missing `Res<T>` is a system error, as in any Bevy system; use `Option<Res<T>>`
  instead of the old "resource must exist" scene error.
- Systems with whole-world access (`&World`, `Query<EntityRef>`) are rejected with a
  scene error.

## `view`

Signals are gone. A `#[component]` function declares its reactive values as
`Watch<..>` parameters, and the type says where each comes from.

| 0.5 | 0.6 |
| --- | --- |
| `fn counter() -> impl Scene { let count = signal(\|\| 0); .. }` | `#[component] fn counter(count: Watch<Local<i32>>) -> impl Scene { .. }` |
| `signal(\|\| value)` | `#[init(value)] name: Watch<Local<T>>` |
| `Signal<T>` | `Watch<Local<T>>` |
| a signal set from systems | a resource, read with `Watch<Res<T>>` (or `Watch<Option<Res<T>>>`) |
| `memo(\|\| ..)` | compute inline: `$(format!("{:?}", $tab))` |
| `signal.get()`, `with`, `set`, `update`, `set_if_neq` | same methods on `Watch`; writes only on `Watch<Local<T>>` |
| calling `counter()` | `@counter()` in `bsn!`/`view!`, as before |

Before:

```rust,ignore
fn counter() -> impl Scene {
    let count = signal(|| 0);
    view! {
        Button
        on(move |_: On<Pointer<Click>>| count.set($count + 1))
        Children [ Text($(format!("{}", $count))) ]
    }
}
```

After:

```rust,ignore
#[component]
fn counter(count: Watch<Local<i32>>) -> impl Scene {
    view! {
        Button
        on(move |_: On<Pointer<Click>>| count.set($count + 1))
        Children [ Text($(format!("{}", $count))) ]
    }
}
```

Behavior changes:

- `Local` state belongs to one spawn of the component: two `@counter()` calls count
  separately.
- Resource watches are bound when the scene spawns. Read them with `$` in component
  fields, regions and handlers; reading one in a `{ .. }` value, which runs while the
  scene is built, panics.
- A component whose fields are all `$` reads, such as `Text($x)`, starts at its
  default value and is filled in by the first update, before UI layout.
- `view` and `hooks` both export a `#[component]` attribute. When you glob-import
  both styles, write `#[view::component]` or `#[hooks::component]`.

# Migrating from 0.4 to 0.5

The four styles are now independent modules, each behind its own Cargo feature
(all on by default) with its own plugin and runtime. Import the style you use with a
glob: `use bevy_radioactive::view::*;`.

| 0.4 | 0.5 |
| --- | --- |
| `bevy_radioactive::prelude`, root re-exports | `bevy_radioactive::{watch, subscribe, hooks, view}::*` |
| `RadioactivePlugin` | `WatchPlugin`, `HooksPlugin` or `ViewPlugin` for each style you use; `subscribe` needs none |
| `ReactiveSet::Publish` | `WatchSystems`, `HooksSystems`, `ViewSystems` |
| `ReactiveSettings { max_passes }` | removed; each runtime settles in up to 8 rounds per frame |
| `ReactiveAppExt` | `WatchAppExt` (`app.track::<T>()`) |
| `app.observe_value(o)` | `app.track::<T>().add_observer(o)` |
| `on_removed(source, handler)` | `watch_removed(source, handler)` |
| `reaction.event().0`, `reaction.value()` | `reaction.0` (`Reaction<T>` dereferences to `T`) |
| `Watch`, `WatchRemoval`, `Subscribe` return types | `impl Template` |
| `subscription` module | `subscribe` module |
| `SubscribedTo`, `Subscribers`, `SubscriptionOf`, `Subscriptions`, `SubscriptionAppExt::register_subscription` | removed; use `subscribe_to` |
| `Subscription::observer()` | removed |
| `webby` module | `hooks` module |
| `Mount` | `Element` |
| `component_with`, `Deps`, `Render`, `Effects`, `Reads` | internal |
| `cx.get`, `cx.this`, `cx.resource` (via `Reads`) | `cx.use_component(entity)`, `cx.use_resource()` |
| `cx.untracked()` | `cx.world()` |
| `cx.use_signal(init)` | `cx.use_state(init)`, or a `view` signal outside `hooks` |
| `cx.use_ref(init) -> Ref<T>` | `cx.use_ref(init) -> Arc<Mutex<T>>` |
| `coat` module, `coat!`, `coat_list!` | `view` module, `view!`, `view_list!` |
| `Signal::increment` / `decrement` | `signal.update(\|n\| *n += 1)` |
| `Signal::notify`, `Signal::ptr_eq`, `Step` | removed |
| `shard(args, render)` | removed; use a `match`/`if` region or `effect` |
| `cloned!` | removed; `view!` clones signals for you, clone by hand elsewhere |
| `Region` | internal |
| `COAT_DEBUG` | `VIEW_DEBUG` |

New: `bind_event(source, |&E, &mut C|)` writes a component of the declaring entity when
an event fires on a source.

Behavior changes:

- Each style runs its own loop in its own system set. A `watch` reaction that writes a
  value read by a `hooks` component or a `view` signal is still picked up in the same
  frame if that style's set runs later, otherwise on the next frame. Order the sets
  with `configure_sets` if you mix styles and need a specific order.
- `hooks` state no longer uses signals: `use_state` and `use_reducer` setters
  re-render their own component. To share changing values through context, provide
  them from a parent that re-renders.
- `use_context` values are plain values; a provider re-render reaches consumers
  because its subtree renders again.

# Migrating from 0.3 to 0.4

Resources are singleton components in Bevy 0.20, so resource and component APIs are
unified. Event subscriptions now support multiple sources per listener.

| 0.3 | 0.4 |
| --- | --- |
| `app.track_resource::<T>()` / `app.track_component::<T>()` | `app.track::<T>()` |
| `app.observe_resource(o)` / `app.observe_component(o)` | `app.observe_value(o)` |
| `watch_resource(handler)` | `watch(Singleton, handler)` |
| `watch_component(#Source, handler)` | `watch(#Source, handler)` |
| one source per event type per listener | any number; also `[#A, #B]` |
| publishers in `Last` | runtime in `PostUpdate`, before UI layout |

New: `bind`, `bind_or`, `on_removed`, `ValueRemoved<T>`, `Removal<T>`,
`RadioactivePlugin`, `ReactiveSettings`, the `webby` module (React-style components and
hooks), and the `coat` module (`coat!` templates and signals, in the new
`bevy_radioactive_macros` crate).

Behavior changes:

- `ReactiveSet::Publish` moved from `Last` to `PostUpdate`. Systems in `Last` that
  mutated watched values before publication must move earlier, or accept next-frame
  delivery.
- The same type may no longer be rejected for being both a resource and a component;
  a resource is just the singleton entity holding `T`, and `Singleton` selects it.
- BSN helpers place each subscription on an internal entity linked to the listener by
  `SubscriptionOf`. `SubscriptionEvent::target()` still reports the listener;
  `Subscription::observer()` now reports the internal subscription entity's observer.
- `app.track::<T>()` is optional: `watch`, `bind` and `on_removed` track their type on
  first use. Without `RadioactivePlugin` they report a scene error naming the plugin.

# Migrating from 0.1 to 0.2

This revision replaces the original lazy, world-wide broadcast prototype with
explicit app-time tracking and source-specific routing. The change is intentionally
breaking: watched types are easier to reason about, component listeners cannot
receive changes from the wrong entity, and subscriptions no longer retarget source
events.

## Migration summary

| Previous API | Replacement |
| --- | --- |
| `install_reactive::<T>(world)` | `app.track_resource::<T>()` or `app.track_component::<T>()` |
| `ReactiveAppExt::add_reactive_observer(handler)` | `observe_resource(handler)` or `observe_component(handler)` |
| `react(handler)` | `watch_resource(handler)` or `watch_component(source, handler)` |
| `React<Changed<T>>` | `Reaction<T>` for BSN listeners |
| `Changed<T>` | `ValueChanged<T>` for app-wide/native observers |
| `Added<T>` | Bevy `On<Add, T>` |
| `Insert<T>` / `Inserted<T>` | Bevy `On<Insert, T>` |
| `Discarded<T>` | Bevy `On<Discard, T>` |
| `Removed<T>` | Bevy `On<Remove, T>` |
| `subscribe_on(source, handler)` | `subscribe_to(source, handler)` |
| `Subscribe<E>` | `SubscribedTo<E>` |
| Forwarded `On<E>` | `Subscription<E>` |
| `RetargetEvent`, `SubscribableEvent`, and retarget helpers | Removed; an observed snapshot is wrapped without retargeting |

`Subscribers<E>` remains the inverse relationship target, now paired with
`SubscribedTo<E>`. The old `ReactiveEvent`, `ReactiveValue`,
`AddReactiveObserver`, and `BroadcastEntityTrigger` implementation types have no
replacement in application code.

## 1. Register watched types during app construction

The old `react(...)` template installed dispatch the first time a scene was resolved.
The new templates never mutate schedules. Register every state type before the app
starts:

```rust
use bevy::prelude::*;
use bevy_radioactive::prelude::*;

app.track_resource::<PuzzleSettings>()
    .track_resource::<PuzzleImage>()
    .track_component::<Health>();
```

Choose the method by storage semantics, even though Bevy 0.19 implements `Resource`
as a singleton `Component`:

- `track_resource::<T>` publishes changes from the one resource entity.
- `track_component::<T>` publishes changes from every entity carrying `T`.

One Rust type cannot be registered in both roles because both routes use
`ValueChanged<T>`. Introduce distinct resource and component newtypes when both are
needed. `watch_resource` also requires the resource to exist when its scene resolves;
both watch helpers now report missing app-time registration as a scene error.

Registration adds publishers to `Last` in `ReactiveSet::Publish` and lasts for the
lifetime of the app. Both methods require `T: Clone`; tracking itself no longer
requires `PartialEq`.

## 2. Replace scene-local `react`

Resource listener before:

```rust
use bevy_radioactive::{Changed as ReactiveChanged, React, react};

fn update_label(
    change: React<ReactiveChanged<PuzzleSettings>>,
    mut text: Query<&mut Text>,
) {
    let Ok(mut text) = text.get_mut(change.target()) else { return; };
    **text = change.columns.to_string();
}

bsn! {
    Text({initial_columns})
    react(update_label)
}
```

After:

```rust
use bevy_radioactive::prelude::*;

fn update_label(change: Reaction<PuzzleSettings>, mut text: Query<&mut Text>) {
    let Ok(mut text) = text.get_mut(change.target()) else { return; };
    **text = change.columns.to_string();
}

bsn! {
    Text({initial_columns})
    watch_resource(update_label)
}
```

`Reaction<T>` dereferences through `ValueChanged<T>` to the shared `T` snapshot,
so field access such as `change.columns` remains concise. `source()` identifies the
resource/component owner; `target()` identifies the local listener.

Component listeners must now name their source:

```rust
bsn! {
    Children [
        (#Player Health),
        (
            Text({initial_health})
            watch_component(#Player, update_health)
        )
    ]
}
```

This replaces the previous world-wide broadcast. A change to another entity's
`Health` no longer invokes this listener.

The old `React::query`, `get`, and `get_mut` conveniences are gone. Query the source
explicitly when the snapshot is insufficient:

```rust
fn inspect_source(change: Reaction<Health>, health: Query<&Health>) {
    let Ok(source_health) = health.get(change.source()) else {
        return;
    };
    // Use source_health with the reaction snapshot.
}
```

## 3. Replace app-wide observers

Before:

```rust
fn update_camera(change: React<Changed<PuzzleSettings>>) {
    // ...
}

app.add_reactive_observer(update_camera);
```

After, choose the source kind and use Bevy's native observer input:

```rust
fn update_camera(change: On<ValueChanged<PuzzleSettings>>) {
    // `ValueChanged<T>` dereferences to T.
}

fn record_health(change: On<ValueChanged<Health>>) {
    // Receives every tracked Health source.
}

app.observe_resource(update_camera)
    .observe_component(record_health);
```

`observe_resource` and `observe_component` both register the inferred value type
and attach the app-wide observer. Use `track_resource` or `track_component` instead
when registration and observer attachment are separate.

The raw `ValueChanged<T>` event exposes `source()`, `value()`, and `snapshot()`,
and dereferences to `T`. `snapshot()` clones its `Arc<T>`, not the underlying
value.

## 4. Use Bevy lifecycle observers directly

The crate no longer clones and rebroadcasts component lifecycle events. Bevy 0.19
supports lifecycle observers for resources as well as ordinary components:

```rust
use bevy::{ecs::lifecycle::{Add, Discard, Insert, Remove}, prelude::*};

app.add_observer(|event: On<Add, MyState>| { /* first presence or re-add */ });
app.add_observer(|event: On<Insert, MyState>| { /* initial insert or replacement */ });
app.add_observer(|event: On<Discard, MyState>| { /* replacement, removal, or despawn */ });
app.add_observer(|event: On<Remove, MyState>| { /* removal or despawn */ });
```

These native lifecycle observers run for the source entity; they are not broadcast to
unrelated BSN listener entities. If a view needs a lifecycle transition, translate it
into application state or a dedicated entity event.

Important lifecycle corrections from the old documentation:

- `Add` also occurs when a removed component is added again.
- `Discard` is not limited to replacement; it also precedes removal and despawn.
- `Remove` occurs for explicit removal and despawn.

## 5. Migrate event subscriptions

Before, forwarding retargeted the original `E`. For `ValueChange<T>`, this replaced
its `source` field with the subscriber entity:

```rust
subscribe_on(#Slider, |event: On<ValueChange<f32>>| {
    // event.source was the receiving entity, not the slider.
})
```

After, `Subscription<E>` wraps one shared clone without retargeting it:

```rust
subscribe_to(#Slider, |event: Subscription<ValueChange<f32>>| {
    let producer = event.source();
    let listener = event.target();
    let value = event.value;
})
```

The event fields are copied as the forwarding observer sees them. Forwarding uses the distinct
`SubscriptionEvent<E>` wrapper, so global `On<E>` observers do not receive one
duplicate `E` for every subscriber. `Subscription<E>` exposes `target()`,
`source()`, `event()`, and `observer()`, and dereferences to `E`. For propagating
events, `source()` is the current propagation entity; keep a separate immutable origin
field on `E` if the pre-propagation target is also required. An earlier observer can
mutate the event before the forwarder snapshots it because unrelated observer order
is unspecified.

The relationship on the listener is now `SubscribedTo<E>`. The producer still stores
the automatically maintained `Subscribers<E>` relationship target. Custom events no
longer implement `RetargetEvent`; they only need to implement `EntityEvent + Clone`.
Each listener supports one source per event type; use separate listener entities for
multiple same-typed sources. `subscribe_to` installs routing automatically. Before
inserting `SubscribedTo<E>` directly, register its forwarder during app construction:

```rust
app.register_subscription::<MyEntityEvent>();
```

Forwarding is deferred. Scheduled event producers reach the usual command flushes. If
you emit an event by calling `World::trigger` directly outside a schedule, follow it
with `World::flush` to deliver the queued `SubscriptionEvent` values; merely starting
`App::update` is not a guaranteed flush for commands queued before its schedules run.

## 6. Account for the new change semantics

The old `Changed<T>` cache required `T: PartialEq` and suppressed a publication when
the final value compared equal to its cached clone. The new tracker follows Bevy
change ticks and does not keep an equality cache.

Mutably dereferencing `ResMut<T>` or `Mut<T>` counts as a change. Use `set_if_neq`
when equal assignments should remain silent:

```rust
fn set_columns(mut settings: ResMut<PuzzleSettings>, columns: u32) {
    let mut next = *settings;
    next.columns = columns;
    settings.set_if_neq(next);
}
```

`T: PartialEq` is needed by `set_if_neq`, not by tracking itself. Tracked snapshots
still require `T: Clone`.

Multiple writes before `ReactiveSet::Publish` runs are coalesced into one publication
holding the final snapshot. Delivery uses deferred commands at the next flush, and
observer execution order is unspecified.

The built-in Bevy `Disabled` component does not suppress tracked publications or
subscription routing, and subscription routing bypasses custom disabling filters too.
Tracked resource/component publisher queries remain subject to a custom component
registered as a default query filter; use an application-specific query system if
those custom-disabled sources must remain reactive.

## 7. Initialize newly spawned views explicitly

Neither the old subscription API nor the new source-routed API is a replay channel.
A watcher added after a value was published does not immediately receive it.

Continue passing the current value into scene construction:

```rust
fn spawn_menu(mut commands: Commands, settings: Res<PuzzleSettings>) {
    commands.spawn_scene(menu_scene(*settings));
}

fn menu_scene(initial: PuzzleSettings) -> impl Scene {
    bsn! {
        Text({initial.columns.to_string()})
        watch_resource(update_columns)
    }
}
```

The first insertion of a tracked value counts as a change, but code should not depend
on listener construction happening before that publication.

## `jigsaw_fox` checklist

For the original consumer, the mechanical migration is:

1. Register `PuzzleSettings`, `PuzzleImage`, `LoadingFeedback`,
   `ConnectorDistortion`, `Pseudo3dSettings`, and the developer-only
   `DebugDisplaySettings` with `track_resource` during plugin construction. An
   `observe_resource` call also registers its inferred type.
2. Replace the camera's `add_reactive_observer` call with `observe_resource`, and
   change that handler to `On<ValueChanged<PuzzleSettings>>`.
3. Replace each BSN `react(update_...)` entry with
   `watch_resource(update_...)`.
4. Change scene-local handler inputs from `React<Changed<T>>` to `Reaction<T>`.
5. Keep passing current resource values into scene functions for the initial UI.
6. Keep `set_if_neq` or explicit equality guards at mutation sites to avoid no-op
   publications.
