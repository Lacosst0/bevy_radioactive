//! Integration coverage for `Reads::count` and `Reads::any`.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, ecs::entity_disabling::Disabled, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, Reads,
    coat::effect,
    computed,
    monitor::{Monitor, MonitorAppExt},
    testing::settle,
};

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Alive(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Health(u32);

#[derive(Component, Clone, Copy, Debug, Default)]
struct Dead;

#[derive(Component, Clone, Copy, Debug, Default)]
struct Living;

#[derive(Component, Clone, Copy, Debug, Default)]
struct Late;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shown(usize);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Flag(bool);

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

fn shown(app: &App, entity: Entity) -> usize {
    app.world().get::<Shown>(entity).map_or(usize::MAX, |s| s.0)
}

#[test]
fn count_follows_additions_removals_and_despawns_only() {
    let mut app = app();
    let first = app.world_mut().spawn(Alive(1)).id();
    app.world_mut().spawn(Alive(2));
    let runs = Arc::new(AtomicUsize::new(0));
    let probe = runs.clone();
    let reader = app
        .world_mut()
        .spawn_scene(bsn! {
            computed(move |r: &Reads| {
                runs.fetch_add(1, Ordering::Relaxed);
                Shown(r.count::<Alive>())
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);
    assert_eq!(probe.load(Ordering::Relaxed), 1);

    let third = app.world_mut().spawn(Alive(3)).id();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 3);

    // Value changes and same-count replacements do not re-run it.
    app.world_mut().get_mut::<Alive>(first).unwrap().0 = 10;
    app.world_mut().entity_mut(third).insert(Alive(30));
    settle(&mut app);
    assert_eq!(probe.load(Ordering::Relaxed), 2);

    app.world_mut().entity_mut(first).remove::<Alive>();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);

    app.world_mut().despawn(third);
    settle(&mut app);
    assert_eq!(shown(&app, reader), 1);
    assert_eq!(probe.load(Ordering::Relaxed), 4);
}

#[test]
fn count_leaves_out_disabled_entities() {
    let mut app = app();
    let entity = app.world_mut().spawn(Alive(1)).id();
    app.world_mut().spawn(Alive(2));
    let reader = app
        .world_mut()
        .spawn_scene(bsn! { computed(|r: &Reads| Shown(r.count::<Alive>())) })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);

    app.world_mut().entity_mut(entity).insert(Disabled);
    settle(&mut app);
    assert_eq!(shown(&app, reader), 1);

    app.world_mut().entity_mut(entity).remove::<Disabled>();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);
}

#[test]
fn any_reruns_only_when_it_flips() {
    let mut app = app();
    let runs = Arc::new(AtomicUsize::new(0));
    let probe = runs.clone();
    let reader = app
        .world_mut()
        .spawn_scene(bsn! {
            computed(move |r: &Reads| {
                runs.fetch_add(1, Ordering::Relaxed);
                Flag(r.any::<Alive>())
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    // `Alive` is not registered yet.
    assert_eq!(app.world().get::<Flag>(reader), Some(&Flag(false)));

    let a = app.world_mut().spawn(Alive(1)).id();
    settle(&mut app);
    assert_eq!(app.world().get::<Flag>(reader), Some(&Flag(true)));
    let after_first = probe.load(Ordering::Relaxed);

    let b = app.world_mut().spawn(Alive(2)).id();
    settle(&mut app);
    app.world_mut().despawn(a);
    settle(&mut app);
    assert_eq!(
        probe.load(Ordering::Relaxed),
        after_first,
        "re-ran without a flip"
    );

    app.world_mut().despawn(b);
    settle(&mut app);
    assert_eq!(app.world().get::<Flag>(reader), Some(&Flag(false)));
}

#[test]
fn count_of_an_unregistered_marker_starts_at_zero() {
    let mut app = app();
    let reader = app
        .world_mut()
        .spawn_scene(bsn! { computed(|r: &Reads| Shown(r.count::<Late>())) })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 0);
    app.world_mut().spawn(Late);
    app.world_mut().spawn(Late);
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);
}

#[test]
fn count_of_a_monitor_marker_counts_matches() {
    let mut app = app();
    app.add_monitor::<Living>(Monitor::new().with::<Health>().without::<Dead>());
    let a = app.world_mut().spawn(Health(3)).id();
    app.world_mut().spawn(Health(5));
    let reader = app
        .world_mut()
        .spawn_scene(bsn! { computed(|r: &Reads| Shown(r.count::<Living>())) })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(shown(&app, reader), 2);

    app.world_mut().entity_mut(a).insert(Dead);
    settle(&mut app);
    assert_eq!(shown(&app, reader), 1);
}

#[test]
fn count_in_an_effect_is_counted_in_the_same_frame_as_spawns() {
    let mut app = app();
    let seen = Arc::new(AtomicUsize::new(usize::MAX));
    let log = seen.clone();
    app.world_mut()
        .spawn_scene(bsn! {
            effect(move |r: &Reads, commands: &mut Commands| {
                let count = r.count::<Alive>();
                log.store(count, Ordering::Relaxed);
                if count < 3 {
                    commands.spawn(Alive(0));
                }
            })
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(seen.load(Ordering::Relaxed), 3);
}
