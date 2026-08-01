# `bevy_radioactive`

`bevy_radioactive` connects Bevy 0.19 state and entity events to local observers in
code-authored BSN scenes. A UI node can own the handler that keeps it synchronized
without a global system querying every possible view.

This crate is deliberately small. It provides source-routed change notifications and
event subscriptions; it is not a virtual DOM, a scene reconciler, or a general-purpose
functional-reactive state graph.

## Compatibility

| `bevy_radioactive` | Bevy | Rust |
| --- | --- | --- |
| 0.2 | 0.19 | 1.95 or newer |

For a workspace checkout:

```toml
[dependencies]
bevy = "0.19"
bevy_radioactive = { path = "../bevy_radioactive" }
```

The API in this revision is intentionally incompatible with the earlier broadcast
prototype. See [MIGRATION.md](MIGRATION.md) for a complete mapping.

## Quick start: watch a resource

Register every watched type while building the app. Scene templates only attach
listeners; they never modify a running schedule.

```rust,no_run
use bevy::{prelude::*, scene::CommandsSceneExt};
use bevy_radioactive::prelude::*;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counter(u32);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .init_resource::<Counter>()
        .track_resource::<Counter>()
        .add_systems(Startup, setup)
        .add_systems(Update, increment)
        .run();
}

fn setup(mut commands: Commands, counter: Res<Counter>) {
    commands.spawn(Camera2d);
    commands.spawn_scene(counter_label(*counter));
}

fn counter_label(initial: Counter) -> impl Scene {
    bsn! {
        Text({initial.0.to_string()})
        watch_resource(update_counter_label)
    }
}

fn update_counter_label(change: Reaction<Counter>, mut labels: Query<&mut Text>) {
    let Ok(mut label) = labels.get_mut(change.target()) else {
        return;
    };
    **label = change.0.to_string();
}

fn increment(keys: Res<ButtonInput<KeyCode>>, mut counter: ResMut<Counter>) {
    if keys.just_pressed(KeyCode::Space) {
        let next = Counter(counter.0 + 1);
        counter.set_if_neq(next);
    }
}
```

`Reaction<T>` dereferences to the shared `T` snapshot. Its two entity accessors have
distinct meanings:

- `source()` is the component owner, or Bevy's singleton resource entity.
- `target()` is the BSN entity whose local observer is running.

The scene receives the current value explicitly and uses it for its initial render.
`watch_resource` observes future publications; it does not replay a value merely
because a new listener was added. Resolving the scene returns an error if `Counter`
was not registered or does not currently exist as a resource.

## Watch one component source

Component changes are routed by source entity. A listener watching `#Player` is not
notified when the same component type changes on another entity.

```rust
use bevy::prelude::*;
use bevy_radioactive::prelude::*;

#[derive(Component, Clone, Copy, Debug, Default)]
struct Score(u32);

fn scoreboard(initial: Score) -> impl Scene {
    bsn! {
        Children [
            (#Player Score({initial.0})),
            (
                Text({initial.0.to_string()})
                watch_component(#Player, update_score)
            )
        ]
    }
}

fn update_score(change: Reaction<Score>, mut labels: Query<&mut Text>) {
    let Ok(mut label) = labels.get_mut(change.target()) else {
        return;
    };
    **label = change.0.to_string();
}
```

Register the type once during app construction:

```rust
# use bevy::prelude::*;
# use bevy_radioactive::prelude::*;
# #[derive(Component, Clone)]
# struct Score(u32);
# let mut app = App::new();
app.track_component::<Score>();
```

`watch_component` accepts an `EntityTemplate`, so named BSN references such as
`#Player` resolve normally. Bevy relationships associate the listener with that
specific source. Resolving the scene returns an error when `Score` was not registered.

## App-wide observers

Use the `ReactiveAppExt` convenience methods when behavior is global rather than
owned by a scene node:

```rust
# use bevy::prelude::*;
# use bevy_radioactive::prelude::*;
# #[derive(Resource, Clone)]
# struct PuzzleSettings;
# #[derive(Component, Clone)]
# struct Health;
fn plugin(app: &mut App) {
    app.observe_resource(update_camera_from_settings)
        .observe_component(record_health_change);
}

fn update_camera_from_settings(change: On<ValueChanged<PuzzleSettings>>) {
    // `change.source()` is the PuzzleSettings resource entity.
}

fn record_health_change(change: On<ValueChanged<Health>>) {
    // Runs for every tracked Health source.
}
```

`observe_resource` and `observe_component` register tracking and add a native
app-wide `On<ValueChanged<T>>` observer. Use `track_resource::<T>` or
`track_component::<T>` when registration and observer attachment are separate. All
four methods must be called while constructing the app, before it starts running.
A Rust type can be tracked in exactly one storage role: resource or ordinary
component. Use distinct newtype wrappers if the same payload needs both roles.

## Subscribe to another entity's event

State reactions and event subscriptions solve different problems. Use
`subscribe_to(source, handler)` for a transient entity event emitted by a particular
widget or peer:

```rust
use bevy::{prelude::*, ui_widgets::ValueChange};
use bevy_radioactive::prelude::*;

fn slider_and_label() -> impl Scene {
    bsn! {
        Children [
            (#Slider Name::new("volume slider")),
            (
                Text("0")
                subscribe_to(#Slider, update_slider_label)
            )
        ]
    }
}

fn update_slider_label(
    event: Subscription<ValueChange<f32>>,
    mut labels: Query<&mut Text>,
) {
    let Ok(mut label) = labels.get_mut(event.target()) else {
        return;
    };
    **label = event.value.to_string();
}
```

`Subscription<E>` dereferences to a clone of the event as the forwarding observer saw
it. `source()` reports the entity whose relationship routed the event and `target()`
reports the receiving entity. Forwarding does not rewrite fields such as
`ValueChange::source`, and it does not retrigger `E` for global `On<E>` observers.

For propagating events, `source()` is the current propagation entity. If a handler
also needs the pre-propagation origin, store that entity in a separate immutable field
on `E`. The snapshot may include mutations made by an earlier observer because Bevy
does not specify ordering between unrelated observers; `bevy_radioactive` itself never
retargets the clone.

The public `SubscribedTo<E>` relationship is stored on the listener and its
`Subscribers<E>` relationship target is stored on the producer. Bevy maintains the
relationship target; treat it as read-only application state.
`SubscriptionEvent<E>` is the underlying targeted wrapper for integrations that
need a regular `On<SubscriptionEvent<E>>` observer.

A listener has one source per event type. Put handlers on separate listener entities
when the same `E` must be received from multiple sources. `subscribe_to` installs the
shared forwarder automatically. Code that inserts `SubscribedTo<E>` directly must
first call `app.register_subscription::<E>()` during app construction; otherwise the
insertion fails immediately instead of silently losing the first event.

Subscriptions carry events, not current state. They do not replay an event to a late
subscriber.

Forwarded deliveries are queued through Bevy's deferred `Commands`. Events emitted
inside schedules are delivered at the next normal command flush. If application code
calls `World::trigger` directly outside a schedule, call `World::flush` afterward;
starting `App::update` is not itself a guaranteed flush for commands that were already
queued before its schedules began.

## Choosing an API

| Need | Use |
| --- | --- |
| A BSN node mirrors a resource | `track_resource::<T>` + `watch_resource` |
| A BSN node mirrors one entity's component | `track_component::<T>` + `watch_component` |
| App-wide behavior reacts to a resource | `observe_resource` |
| App-wide behavior reacts to all component sources | `observe_component` |
| One entity listens to another entity's transient event | `subscribe_to` |
| Component/resource add, insert, discard, or removal | Bevy's native lifecycle observers |
| Bulk or per-frame processing over many entities | A normal scheduled Bevy system |

Bevy 0.19 stores resources as components on singleton entities. Consequently, native
lifecycle observers work for both components and resources and this crate does not
wrap them:

```rust
use bevy::{ecs::lifecycle::{Add, Discard, Insert, Remove}, prelude::*};

# #[derive(Resource)]
# struct MyState;
# let mut app = App::new();

app.add_observer(|event: On<Add, MyState>| { /* added */ });
app.add_observer(|event: On<Insert, MyState>| { /* inserted or replaced */ });
app.add_observer(|event: On<Discard, MyState>| { /* old value is leaving */ });
app.add_observer(|event: On<Remove, MyState>| { /* removed or despawned */ });
```

## Change and delivery semantics

Tracking uses Bevy's native change ticks:

- The first insertion counts as a change.
- Mutably dereferencing `ResMut<T>` or `Mut<T>` marks the value changed, even if the
  final value compares equal to the previous value.
- Use `set_if_neq` when `T: PartialEq` to avoid publishing no-op writes.
- Multiple writes between publisher runs are coalesced into one final snapshot per
  source.
- Publishers run in `Last` in `ReactiveSet::Publish`. Writes before that set can be
  published in the same frame; writes after it are observed on a later run.
- A `ValueChanged<T>` snapshot is delivered through deferred commands at the next
  command flush.
- Ordering between different local observers is unspecified. Do not make one
  reaction depend on another reaction running first.
- Adding a listener does not replay the current value. Read state while creating the
  scene and use it for the initial UI, as shown above.
- Sources carrying Bevy's built-in `Disabled` marker still publish and route events.
  Subscription routing also bypasses application-defined disabling filters. Tracked
  state publisher queries remain subject to custom default filters; use a tailored
  normal query system when custom-disabled resource or component sources must stay
  reactive.

`ValueChanged<T>` is the underlying source-preserving snapshot event. It dereferences
to `T` and also exposes `source()`, `value()`, and `snapshot()`.
`Reaction<T>` is the scene-listener input: an alias for
`Subscription<ValueChanged<T>>` that dereferences through to the same snapshot.

## Performance model

Registration adds one lightweight `Last` publisher for each resource/component
tracking registration. Resource tracking checks one singleton; component tracking
uses Bevy's non-archetypal `Changed<T>` filter, which scans matching `T` instances on
each publisher run and yields only changed ones. When a source changes, the crate
clones one `T` snapshot and routes it through relationships only to listeners of that
source. Each listener then runs its own observer system.

There is no value cache, equality comparison, or world-wide scan of entity observers.
The publisher remains installed for the app's lifetime, while listener relationships
follow their scene entities. Snapshot delivery shares `Arc` handles rather than
cloning `T` once per listener.

For good results:

- keep reactive values reasonably small or cheap to clone;
- split large state into independently changing components/resources;
- use `set_if_neq` to suppress accidental no-op publications;
- prefer a normal query system when thousands of entities change every frame; and
- prefer messages or ordinary events for streams that do not represent durable state.

`subscribe_to` clones `E` once when the source event is received, then shares it
between recipients and clones only a small `Arc` handle per subscription.

## The `jigsaw_fox` use case

This crate was extracted from a BSN-heavy puzzle UI. `jigsaw_fox` initializes each
scene from its current resources, then uses local resource watchers to keep the
following views synchronized:

- `PuzzleSettings` updates the camera, column count, puzzle summary, and preview-grid
  material.
- `PuzzleImage` updates the preview image, aspect ratio, image summary, feedback, and
  play-button availability.
- `LoadingFeedback` updates the menu feedback label.
- `ConnectorDistortion` and `Pseudo3dSettings` update settings status text.
- `DebugDisplaySettings` updates developer-only diagnostics status text.

This keeps update behavior next to the `Text`, `ImageNode`, or material-bearing node
that owns it. Adjacent uses include inspector fields, HUD labels, settings summaries,
tooltips, and direct slider-to-label or button-to-panel event wiring.

## Official Bevy references

- [Bevy 0.19: resources as components and next-generation scenes](https://bevy.org/news/bevy-0-19/)
- [BSN and scene observers](https://docs.rs/bevy/0.19.0/bevy/scene/)
- [Bevy observer semantics](https://docs.rs/bevy_ecs/0.19.0/bevy_ecs/observer/)
- [Bevy change detection](https://docs.rs/bevy_ecs/0.19.0/bevy_ecs/change_detection/)
- [Bevy ECS relationships](https://docs.rs/bevy_ecs/0.19.0/bevy_ecs/relationship/)
