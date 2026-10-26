//! Integration coverage for query monitors.

use std::sync::{Arc, Mutex};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, Reads, computed,
    monitor::{Monitor, MonitorAppExt},
    testing::settle,
};

#[derive(Component, Clone, Copy, Debug, Default)]
struct Health(u32);

#[derive(Component, Clone, Copy, Debug, Default)]
struct Dead;

#[derive(Component, Clone, Copy, Debug, Default)]
struct Alive;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Status(bool);

#[derive(Resource, Clone, Default)]
struct Log(Arc<Mutex<Vec<(&'static str, Entity)>>>);

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

fn sorted<const N: usize>(mut entities: [Entity; N]) -> Vec<Entity> {
    entities.sort();
    entities.to_vec()
}

fn alive(app: &mut App) -> Vec<Entity> {
    let mut alive: Vec<Entity> = app
        .world_mut()
        .query_filtered::<Entity, With<Alive>>()
        .iter(app.world())
        .collect();
    alive.sort();
    alive
}

#[test]
fn monitor_tracks_entities_entering_and_leaving_the_filter() {
    let mut app = app();
    let existing = app.world_mut().spawn(Health(1)).id();
    let dead = app.world_mut().spawn((Health(1), Dead)).id();
    let log = Log::default();
    let (entered, left) = (log.0.clone(), log.0.clone());
    app.add_monitor::<Alive>(Monitor::new().with::<Health>().without::<Dead>())
        .add_observer(move |event: On<Add<Alive>>| {
            entered.lock().unwrap().push(("enter", event.entity));
        })
        .add_observer(move |event: On<Remove<Alive>>| {
            left.lock().unwrap().push(("leave", event.entity));
        });
    // Entities that matched before the monitor existed get the marker.
    assert_eq!(alive(&mut app), sorted([existing]));

    let spawned = app.world_mut().spawn(Health(5)).id();
    app.world_mut().flush();
    assert_eq!(alive(&mut app), sorted([existing, spawned]));

    // An excluded component makes it leave, removing it makes it enter.
    app.world_mut().entity_mut(spawned).insert(Dead);
    app.world_mut().flush();
    assert_eq!(alive(&mut app), sorted([existing]));
    app.world_mut().entity_mut(dead).remove::<Dead>();
    app.world_mut().flush();
    assert_eq!(alive(&mut app), sorted([existing, dead]));

    // Losing a required component makes it leave.
    app.world_mut().entity_mut(existing).remove::<Health>();
    app.world_mut().flush();
    assert_eq!(alive(&mut app), sorted([dead]));

    // Replacing a value is not a membership change.
    app.world_mut().entity_mut(dead).insert(Health(9));
    app.world_mut().flush();
    assert_eq!(alive(&mut app), sorted([dead]));

    assert_eq!(
        *log.0.lock().unwrap(),
        [
            ("enter", spawned),
            ("leave", spawned),
            ("enter", dead),
            ("leave", existing),
        ]
    );
}

#[test]
fn monitor_markers_drive_reactive_reads_and_despawn_cleanly() {
    let mut app = app();
    app.add_monitor::<Alive>(Monitor::new().with::<Health>().without::<Dead>());
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Health(3)
            computed(|r: &Reads| Status(r.this_has::<Alive>()))
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(app.world().get::<Status>(entity), Some(&Status(true)));

    app.world_mut().entity_mut(entity).insert(Dead);
    settle(&mut app);
    assert_eq!(app.world().get::<Status>(entity), Some(&Status(false)));

    app.world_mut().entity_mut(entity).remove::<Dead>();
    settle(&mut app);
    assert_eq!(app.world().get::<Status>(entity), Some(&Status(true)));

    // Despawn while matching: the queued re-check finds nothing to do.
    app.world_mut().despawn(entity);
    settle(&mut app);
    assert_eq!(alive(&mut app), Vec::<Entity>::new());
}

#[test]
fn monitors_with_several_terms_and_markers_are_independent() {
    #[derive(Component, Default)]
    struct Armed;
    #[derive(Component, Default)]
    struct Dangerous;

    let mut app = app();
    app.add_monitor::<Alive>(Monitor::new().with::<Health>())
        .add_monitor::<Dangerous>(
            Monitor::new()
                .with::<Health>()
                .with::<Armed>()
                .without::<Dead>(),
        );
    let entity = app.world_mut().spawn(Health(1)).id();
    app.world_mut().flush();
    assert!(app.world().entity(entity).contains::<Alive>());
    assert!(!app.world().entity(entity).contains::<Dangerous>());

    app.world_mut().entity_mut(entity).insert(Armed);
    app.world_mut().flush();
    assert!(app.world().entity(entity).contains::<Dangerous>());

    app.world_mut().entity_mut(entity).insert(Dead);
    app.world_mut().flush();
    assert!(app.world().entity(entity).contains::<Alive>());
    assert!(!app.world().entity(entity).contains::<Dangerous>());
}

#[test]
#[should_panic(expected = "at least one `with` term")]
fn monitor_without_required_terms_panics() {
    app().add_monitor::<Alive>(Monitor::new().without::<Dead>());
}
