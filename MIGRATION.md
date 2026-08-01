# Migrating from 0.1 to 0.2

This revision replaces the original lazy, world-wide broadcast prototype with
explicit app-time tracking and source-specific routing. The change is intentionally
breaking: watched types are easier to reason about, component listeners cannot
receive changes from the wrong entity, and subscriptions preserve their original
event.

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
| `RetargetEvent`, `SubscribableEvent`, and retarget helpers | Removed; the original cloned event is wrapped unchanged |

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

After, `Subscription<E>` wraps an unchanged shared clone:

```rust
subscribe_to(#Slider, |event: Subscription<ValueChange<f32>>| {
    let producer = event.source();
    let listener = event.target();
    let value = event.value;
})
```

The original event fields remain intact. Forwarding uses the distinct
`SubscriptionEvent<E>` wrapper, so global `On<E>` observers do not receive one
duplicate `E` for every subscriber. `Subscription<E>` exposes `target()`,
`source()`, `event()`, and `observer()`, and dereferences to `E`.

The relationship on the listener is now `SubscribedTo<E>`. The producer still stores
the automatically maintained `Subscribers<E>` relationship target. Custom events no
longer implement `RetargetEvent`; they only need to implement `EntityEvent + Clone`.

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
   `ConnectorDistortion`, and `Pseudo3dSettings` with `track_resource` during
   plugin construction. An `observe_resource` call also registers its inferred type.
2. Replace the camera's `add_reactive_observer` call with `observe_resource`, and
   change that handler to `On<ValueChanged<PuzzleSettings>>`.
3. Replace each BSN `react(update_...)` entry with
   `watch_resource(update_...)`.
4. Change scene-local handler inputs from `React<Changed<T>>` to `Reaction<T>`.
5. Keep passing current resource values into scene functions for the initial UI.
6. Keep `set_if_neq` or explicit equality guards at mutation sites to avoid no-op
   publications.
