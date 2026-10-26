//! Regression tests for defects found by the 0.4 API review.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use bevy::{asset::AssetPlugin, ecs::observer::Observer, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, Reaction, ReactiveAppExt, ReactiveDiagnostics, SubscriptionOf, ValueChanged,
    ValueRemoved,
    coat::{Reads, Signal, coat, memo, signal},
    subscription::{Subscription, subscribe_to},
    watch,
    webby::{Cx, component},
};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ));
    app
}

fn count<T: Component>(app: &mut App) -> usize {
    app.world_mut().query::<&T>().iter(app.world()).count()
}

#[derive(Component, Clone, Default)]
struct Label(u32);

#[derive(Component, Clone, Default)]
struct Meter(u32);

// ---------------------------------------------------------------------------
// Webby instance trees are freed when their host despawns.
// ---------------------------------------------------------------------------

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[component]
fn leaf(cx: &Cx, probe: Arc<AtomicUsize>) -> impl Scene {
    let _probe = cx.use_ref(|| DropProbe(probe.clone()));
    bsn! { Label(1) }
}

#[component]
fn parent(cx: &Cx, probe: Arc<AtomicUsize>) -> impl Scene {
    let _probe = cx.use_ref(|| DropProbe(probe.clone()));
    bsn! { Node Children [ @leaf(probe.clone()) ] }
}

#[test]
fn instance_tree_is_freed_on_despawn() {
    let mut app = app();
    let probe = Arc::new(AtomicUsize::new(0));
    let root = app
        .world_mut()
        .spawn_scene(bsn! { @parent(probe.clone()) })
        .unwrap()
        .id();
    app.update();
    assert_eq!(count::<Label>(&mut app), 1);
    app.world_mut().despawn(root);
    app.update();
    assert_eq!(
        probe.load(Ordering::SeqCst),
        2,
        "instance hook state leaked"
    );
}

// ---------------------------------------------------------------------------
// Duplicate keys in a keyed `for` do not leak rows.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Row {
    id: u32,
    n: u32,
}

#[test]
fn duplicate_keys_do_not_leak_rows() {
    let rows = signal(|| vec![Row { id: 1, n: 1 }, Row { id: 1, n: 2 }]);
    let mut app = app();
    let scene = {
        let rows = rows.clone();
        coat! {
            Node
            Children [
                #[key(row.id)]
                for row in $rows { Label({row.n}) }
            ]
        }
    };
    let root = app.world_mut().spawn_scene(scene).unwrap().id();
    app.update();
    assert_eq!(
        count::<Label>(&mut app),
        1,
        "the duplicate row replaces the first"
    );
    rows.set(vec![]);
    app.update();
    assert_eq!(
        count::<Label>(&mut app),
        0,
        "rows with duplicate keys leaked"
    );
    let orphans = app
        .world_mut()
        .query_filtered::<Entity, (Without<ChildOf>, With<Label>)>()
        .iter(app.world())
        .count();
    assert_eq!(orphans, 0);
    app.world_mut().despawn(root);
}

// ---------------------------------------------------------------------------
// Deep trees settle in one frame, independent of `max_passes`.
// ---------------------------------------------------------------------------

#[component]
fn n0() -> impl Scene {
    bsn! { Label(99) }
}

macro_rules! level {
    ($name:ident, $inner:ident) => {
        #[component]
        fn $name() -> impl Scene {
            bsn! { Node Children [ @$inner() ] }
        }
    };
}

level!(n1, n0);
level!(n2, n1);
level!(n3, n2);
level!(n4, n3);
level!(n5, n4);
level!(n6, n5);
level!(n7, n6);
level!(n8, n7);
level!(n9, n8);
level!(n10, n9);
level!(n11, n10);
level!(n12, n11);

#[test]
fn deep_component_tree_settles_in_one_frame() {
    let mut app = app();
    app.world_mut().spawn_scene(bsn! { @n12() }).unwrap();
    app.update();
    assert_eq!(
        count::<Label>(&mut app),
        1,
        "leaf was not rendered this frame"
    );
    assert_eq!(
        app.world()
            .resource::<ReactiveDiagnostics>()
            .exhausted_frames,
        0
    );
}

// ---------------------------------------------------------------------------
// A conditional sibling component does not crash the app.
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Default)]
struct Show(bool);

#[component]
fn uses_bool(cx: &Cx) -> impl Scene {
    let (flag, _) = cx.use_state(|| true);
    bsn! { Label({u32::from(flag) + 1}) }
}

#[component]
fn uses_int(cx: &Cx) -> impl Scene {
    let (n, _) = cx.use_state(|| 5_u32);
    bsn! { Label({n}) }
}

#[component]
fn switcher(cx: &Cx) -> impl Scene {
    let show = cx.use_resource::<Show>().unwrap_or_default().0;
    bsn! {
        Node Children [
            {show.then(|| bsn_list! { @uses_bool() })}
            --
            @uses_int()
        ]
    }
}

#[test]
fn conditional_sibling_component_does_not_panic() {
    let mut app = app();
    app.init_resource::<Show>();
    app.world_mut().spawn_scene(bsn! { @switcher() }).unwrap();
    app.update();
    app.world_mut().resource_mut::<Show>().0 = true;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.update()));
    assert!(result.is_ok(), "a conditional sibling crashed the app");
    let mut labels: Vec<u32> = app
        .world_mut()
        .query::<&Label>()
        .iter(app.world())
        .map(|label| label.0)
        .collect();
    labels.sort_unstable();
    assert_eq!(labels, vec![2, 5]);
}

// ---------------------------------------------------------------------------
// Subscription entities are despawned with their sources.
// ---------------------------------------------------------------------------

#[test]
fn subscription_entities_follow_their_sources() {
    let mut app = app();
    let sources: Vec<Entity> = (0..5)
        .map(|i| app.world_mut().spawn(Meter(i)).id())
        .collect();
    let watched = sources.clone();
    let listener = app
        .world_mut()
        .spawn_scene(bsn! {
            Label(0)
            watch(watched.clone(), |_: Reaction<Meter>| {})
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(count::<SubscriptionOf>(&mut app), 5);
    let observers = count::<Observer>(&mut app);

    for source in &sources {
        app.world_mut().despawn(*source);
    }
    app.update();
    assert_eq!(count::<SubscriptionOf>(&mut app), 0, "subscriptions leaked");
    assert_eq!(
        count::<Observer>(&mut app),
        observers - 5,
        "observers leaked"
    );
    assert!(app.world().get_entity(listener).is_ok());
}

// ---------------------------------------------------------------------------
// Raw value-event subscriptions deliver exactly once.
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
struct Hits {
    removed: usize,
    changed: usize,
}

#[test]
fn raw_value_event_subscriptions_deliver_once() {
    let mut app = app();
    app.init_resource::<Hits>();
    let source = app.world_mut().spawn(Meter(1)).id();
    app.world_mut()
        .spawn_scene(bsn! {
            Label(0)
            subscribe_to(source, |_: Subscription<ValueRemoved<Meter>>, mut hits: ResMut<Hits>| {
                hits.removed += 1;
            })
            subscribe_to(source, |_: Subscription<ValueChanged<Meter>>, mut hits: ResMut<Hits>| {
                hits.changed += 1;
            })
        })
        .unwrap();
    app.update();

    // Subscribing registers the event type before `Meter` is tracked.
    app.track::<Meter>();
    app.world_mut().get_mut::<Meter>(source).unwrap().0 = 3;
    app.update();
    assert_eq!(app.world().resource::<Hits>().changed, 1);

    app.world_mut().entity_mut(source).remove::<Meter>();
    app.world_mut().flush();
    app.update();
    assert_eq!(
        app.world().resource::<Hits>().removed,
        1,
        "removal delivered twice"
    );
}

// ---------------------------------------------------------------------------
// Exhausting `max_passes` is reported.
// ---------------------------------------------------------------------------

#[test]
fn max_passes_exhaustion_is_reported() {
    let value = signal(|| 0_u32);
    let mut app = app();
    let scene = {
        let value = value.clone();
        bsn! {
            Label(0)
            bevy_radioactive::coat::effect(move |_: &Reads, _: &mut Commands| {
                let current = value.get();
                if current < 1000 {
                    value.set(current + 1);
                }
            })
        }
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    assert_eq!(value.get_untracked(), 8);
    assert_eq!(
        app.world()
            .resource::<ReactiveDiagnostics>()
            .exhausted_frames,
        1
    );
}

// ---------------------------------------------------------------------------
// Latent hazards.
// ---------------------------------------------------------------------------

/// Runs `f` on another thread. Returns `Some(panicked)`, or `None` if it hung.
fn outcome(f: impl FnOnce() + Send + 'static) -> Option<bool> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = sender.send(result.is_err());
    });
    receiver.recv_timeout(Duration::from_secs(5)).ok()
}

#[test]
fn setting_a_signal_while_reading_it_panics_instead_of_deadlocking() {
    let value: Signal<i32> = Signal::new(1);
    let reader = value.clone();
    assert_eq!(outcome(move || reader.with(|_| reader.set(2))), Some(true));
    let updater = value.clone();
    assert_eq!(
        outcome(move || {
            let _ = updater.update(|_| updater.get());
        }),
        Some(true)
    );
    // Nested reads are fine, and the signal stays usable.
    assert_eq!(value.with(|a| value.with(|b| a + b)), 2);
    value.set(3);
    assert_eq!(value.get(), 3);
}

#[test]
fn memo_cycle_panics_instead_of_overflowing() {
    let cell: Arc<std::sync::OnceLock<bevy_radioactive::coat::Memo<i32>>> = Arc::default();
    let inner = cell.clone();
    let looped = memo(move || inner.get().map_or(0, |memo| memo.get() + 1));
    cell.set(looped.clone()).ok().unwrap();
    assert_eq!(
        outcome(move || {
            let _ = looped.get();
        }),
        Some(true)
    );
}

#[derive(Resource, Default)]
struct TextChanges(usize);

#[test]
fn patch_does_not_republish_unchanged_components() {
    let n = signal(|| 1_u32);
    let mut app = app();
    app.init_resource::<TextChanges>().observe_value(
        |_: On<ValueChanged<Text>>, mut changes: ResMut<TextChanges>| {
            changes.0 += 1;
        },
    );
    let scene = {
        let n = n.clone();
        coat! { Text($(if $n % 2 == 0 { "even" } else { "odd" }.to_string())) }
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    let initial = app.world().resource::<TextChanges>().0;
    n.set(3);
    app.update();
    assert_eq!(app.world().resource::<TextChanges>().0, initial);
    n.set(4);
    app.update();
    assert_eq!(app.world().resource::<TextChanges>().0, initial + 1);
}

#[derive(Resource, Default)]
struct Removals(usize);

#[test]
fn despawned_source_delivers_removal_before_its_subscription_goes() {
    let mut app = app();
    app.init_resource::<Removals>();
    let source = app.world_mut().spawn(Meter(1)).id();
    app.world_mut()
        .spawn_scene(bsn! {
            Label(0)
            bevy_radioactive::on_removed(source, |_: bevy_radioactive::Removal<Meter>, mut removals: ResMut<Removals>| {
                removals.0 += 1;
            })
        })
        .unwrap();
    app.update();
    app.world_mut().despawn(source);
    app.update();
    assert_eq!(app.world().resource::<Removals>().0, 1);
    assert_eq!(count::<SubscriptionOf>(&mut app), 0);
}
