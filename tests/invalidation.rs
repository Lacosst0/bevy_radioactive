//! Integration coverage for push-based invalidation: idle frames do no
//! work, memo diamonds compute once, and writes made outside the runtime
//! are still seen.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, ReactiveDiagnostics, ReactiveSet, Reads,
    coat::{Signal, effect, memo},
    computed,
    testing::{settle, settle_n},
};

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Level(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[component(immutable)]
struct Fixed(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shown(u32);

#[derive(Component, Clone, Copy, Debug, Default)]
struct Marker;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Score(u32);

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

fn counter() -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    (runs.clone(), runs)
}

fn diagnostics(app: &App) -> ReactiveDiagnostics {
    *app.world().resource::<ReactiveDiagnostics>()
}

fn shown(app: &App, entity: Entity) -> u32 {
    app.world()
        .get::<Shown>(entity)
        .map_or(u32::MAX, |shown| shown.0)
}

#[test]
fn idle_frames_check_no_hosts_and_run_no_effects() {
    let mut app = app();
    app.insert_resource(Score(1));
    let signal = Signal::new(0_u32);
    let level = app.world_mut().spawn(Level(1)).id();
    let fixed = app.world_mut().spawn(Fixed(1)).id();
    let marked = app.world_mut().spawn(Marker).id();
    let (runs, probe) = counter();
    for _ in 0..50 {
        let (signal, runs) = (signal.clone(), runs.clone());
        let parent = app
            .world_mut()
            .spawn_scene(bsn! {
                effect(move |r: &Reads, _: &mut Commands| {
                    runs.fetch_add(1, Ordering::Relaxed);
                    let _ = signal.get();
                    let _ = r.get::<Level>(level);
                    let _ = r.get::<Fixed>(fixed);
                    let _ = r.has::<Marker>(marked);
                    let _ = r.resource::<Score>();
                })
            })
            .unwrap()
            .id();
        app.world_mut()
            .spawn_scene(
                bsn! { computed(|r: &Reads| Shown(r.inherited::<Level>().map_or(0, |l| l.0))) },
            )
            .unwrap()
            .insert(ChildOf(parent));
    }
    settle_n(&mut app, 2);
    assert_eq!(probe.load(Ordering::Relaxed), 50);

    for _ in 0..3 {
        settle(&mut app);
        let frame = diagnostics(&app);
        assert_eq!(frame.effects_run, 0);
        assert_eq!(frame.hosts_checked, 0, "an idle frame checked hosts");
        assert_eq!(frame.passes, 1);
    }

    // One write wakes only the hosts that read it.
    signal.set(1);
    settle(&mut app);
    assert_eq!(probe.load(Ordering::Relaxed), 100);
    assert_eq!(diagnostics(&app).hosts_checked, 50);
    settle(&mut app);
    assert_eq!(diagnostics(&app).hosts_checked, 0);
}

#[test]
fn memo_diamond_computes_once_per_change() {
    let mut app = app();
    let source = Signal::new(1_u32);
    let (left_runs, left_probe) = counter();
    let (bottom_runs, bottom_probe) = counter();
    let left = memo({
        let source = source.clone();
        move || {
            left_runs.fetch_add(1, Ordering::Relaxed);
            source.get() + 1
        }
    });
    let right = memo({
        let source = source.clone();
        move || source.get() * 10
    });
    let bottom = memo({
        let (left, right) = (left.clone(), right.clone());
        move || {
            bottom_runs.fetch_add(1, Ordering::Relaxed);
            left.get() + right.get()
        }
    });
    let (effect_runs, effect_probe) = counter();
    let (pair_runs, pair_probe) = counter();
    let seen = Signal::new(0_u32);
    let entity = {
        let (bottom, seen) = (bottom.clone(), seen.clone());
        let (left, right) = (left.clone(), right.clone());
        app.world_mut()
            .spawn_scene(bsn! {
                effect(move |_: &Reads, _: &mut Commands| {
                    effect_runs.fetch_add(1, Ordering::Relaxed);
                    seen.set(bottom.get());
                })
                computed(move |_: &Reads| {
                    pair_runs.fetch_add(1, Ordering::Relaxed);
                    Shown(left.get() + right.get())
                })
            })
            .unwrap()
            .id()
    };
    settle(&mut app);
    assert_eq!(seen.get_untracked(), 12);
    assert_eq!(shown(&app, entity), 12);
    let before = (
        left_probe.load(Ordering::Relaxed),
        bottom_probe.load(Ordering::Relaxed),
        effect_probe.load(Ordering::Relaxed),
        pair_probe.load(Ordering::Relaxed),
    );
    assert_eq!(before, (1, 1, 1, 1));

    source.set(2);
    settle(&mut app);
    assert_eq!(seen.get_untracked(), 23);
    assert_eq!(shown(&app, entity), 23);
    assert_eq!(
        left_probe.load(Ordering::Relaxed),
        2,
        "left memo recomputed twice"
    );
    assert_eq!(
        bottom_probe.load(Ordering::Relaxed),
        2,
        "bottom memo recomputed twice"
    );
    assert_eq!(
        effect_probe.load(Ordering::Relaxed),
        2,
        "diamond effect ran twice"
    );
    assert_eq!(
        pair_probe.load(Ordering::Relaxed),
        2,
        "two-memo reader ran twice"
    );

    // An equal result stops propagation: the reader of the bottom memo does
    // not run, though the memos recompute.
    let flip = memo({
        let source = source.clone();
        move || source.get() % 2
    });
    let (parity_runs, parity_probe) = counter();
    app.world_mut()
        .spawn_scene(bsn! {
            effect(move |_: &Reads, _: &mut Commands| {
                parity_runs.fetch_add(1, Ordering::Relaxed);
                let _ = flip.get();
            })
        })
        .unwrap();
    settle(&mut app);
    source.set(4);
    settle(&mut app);
    assert_eq!(parity_probe.load(Ordering::Relaxed), 1);
}

#[test]
fn system_writes_through_mut_are_seen() {
    let mut app = app();
    app.insert_resource(Score(0));
    let source = app.world_mut().spawn(Level(1)).id();
    let reader = app
        .world_mut()
        .spawn_scene(
            bsn! { computed(move |r: &Reads| Shown(r.get::<Level>(source).map_or(0, |l| l.0))) },
        )
        .unwrap()
        .id();
    let scorer = app
        .world_mut()
        .spawn_scene(bsn! { computed(|r: &Reads| Shown(r.resource::<Score>().map_or(0, |s| s.0))) })
        .unwrap()
        .id();
    app.add_systems(
        Update,
        |mut levels: Query<&mut Level>, mut score: ResMut<Score>, mut frame: Local<u32>| {
            *frame += 1;
            if frame.is_multiple_of(2) {
                for mut level in &mut levels {
                    level.0 += 1;
                }
                score.0 += 10;
            }
        },
    );
    settle(&mut app);
    assert_eq!(shown(&app, reader), 1);
    for frame in 2..8_u32 {
        settle(&mut app);
        assert_eq!(shown(&app, reader), 1 + frame / 2, "frame {frame}");
        assert_eq!(shown(&app, scorer), 10 * (frame / 2), "frame {frame}");
    }

    // Writes ordered just before the runtime in `PostUpdate` land the same
    // frame.
    app.add_systems(
        PostUpdate,
        (move |mut levels: Query<&mut Level>| {
            if let Ok(mut level) = levels.get_mut(source) {
                level.0 = 100;
            }
        })
        .before(ReactiveSet::Publish),
    );
    settle(&mut app);
    assert_eq!(shown(&app, reader), 100);
}

#[test]
fn presence_and_immutable_changes_from_systems_are_seen() {
    let mut app = app();
    let target = app.world_mut().spawn(Fixed(1)).id();
    let reader = app
        .world_mut()
        .spawn_scene(bsn! {
            computed(move |r: &Reads| Shown(
                r.get::<Fixed>(target).map_or(0, |f| f.0) + if r.has::<Marker>(target) { 1000 } else { 0 }
            ))
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 1);

    app.world_mut().entity_mut(target).insert(Fixed(2));
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);

    app.world_mut().entity_mut(target).insert(Marker);
    settle(&mut app);
    assert_eq!(shown(&app, reader), 1002);

    app.world_mut()
        .entity_mut(target)
        .remove::<(Marker, Fixed)>();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 0);

    app.world_mut().entity_mut(target).insert(Fixed(7));
    settle(&mut app);
    assert_eq!(shown(&app, reader), 7);

    app.world_mut().despawn(target);
    settle(&mut app);
    assert_eq!(shown(&app, reader), 0);
}

#[test]
fn effect_writes_reach_later_readers_in_the_same_pass() {
    // A chain of computeds across sibling entities, longer than
    // `max_passes`: each writes the component the next one reads.
    let mut app = app();
    let first = app.world_mut().spawn(Level(1)).id();
    let mut previous = first;
    let mut chain = Vec::new();
    for _ in 0..12 {
        let source = previous;
        let entity = app
            .world_mut()
            .spawn_scene(bsn! {
                computed(move |r: &Reads| Level(r.get::<Level>(source).map_or(0, |l| l.0) + 1))
            })
            .unwrap()
            .id();
        chain.push(entity);
        previous = entity;
    }
    let last = previous;
    settle(&mut app);
    assert_eq!(app.world().get::<Level>(last), Some(&Level(13)));

    app.world_mut().get_mut::<Level>(first).unwrap().0 = 10;
    settle(&mut app);
    assert_eq!(app.world().get::<Level>(last), Some(&Level(22)));
    assert_eq!(diagnostics(&app).exhausted_frames, 0);
    assert_eq!(diagnostics(&app).effects_run, 12);
}

#[test]
fn signals_set_from_other_threads_are_seen() {
    let mut app = app();
    let signal = Signal::new(0_u32);
    let reader = {
        let signal = signal.clone();
        app.world_mut()
            .spawn_scene(bsn! { computed(move |_: &Reads| Shown(signal.get())) })
            .unwrap()
            .id()
    };
    settle(&mut app);
    let remote = signal.clone();
    std::thread::spawn(move || remote.set(5)).join().unwrap();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 5);
}

#[test]
fn hosts_marked_while_disabled_run_when_enabled() {
    let mut app = app();
    let signal = Signal::new(1_u32);
    let reader = {
        let signal = signal.clone();
        app.world_mut()
            .spawn_scene(bsn! { computed(move |_: &Reads| Shown(signal.get())) })
            .unwrap()
            .id()
    };
    settle(&mut app);
    app.world_mut()
        .entity_mut(reader)
        .insert(bevy::ecs::entity_disabling::Disabled);
    signal.set(2);
    settle_n(&mut app, 2);
    assert_eq!(shown(&app, reader), 1, "a disabled host ran");
    app.world_mut()
        .entity_mut(reader)
        .remove::<bevy::ecs::entity_disabling::Disabled>();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);
}

#[test]
fn rerun_hosts_unsubscribe_from_what_they_stopped_reading() {
    let mut app = app();
    let use_a = Signal::new(true);
    let a = Signal::new(0_u32);
    let b = Signal::new(0_u32);
    let (runs, probe) = counter();
    {
        let (use_a, a, b) = (use_a.clone(), a.clone(), b.clone());
        app.world_mut()
            .spawn_scene(bsn! {
                effect(move |_: &Reads, _: &mut Commands| {
                    runs.fetch_add(1, Ordering::Relaxed);
                    let _ = if use_a.get() { a.get() } else { b.get() };
                })
            })
            .unwrap();
    }
    settle(&mut app);
    use_a.set(false);
    settle(&mut app);
    assert_eq!(probe.load(Ordering::Relaxed), 2);
    a.set(1);
    settle(&mut app);
    assert_eq!(probe.load(Ordering::Relaxed), 2);
    assert_eq!(
        diagnostics(&app).hosts_checked,
        0,
        "a stale subscription woke the host"
    );
    b.set(1);
    settle(&mut app);
    assert_eq!(probe.load(Ordering::Relaxed), 3);
}
