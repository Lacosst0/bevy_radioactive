#![cfg(feature = "watch")]
//! Integration coverage for `watch`: `Watch<..>`/`Removed<..>` handlers,
//! selectors and `bind`.

use bevy::{
    asset::AssetPlugin,
    ecs::{entity_disabling::Disabled, query::Allow},
    prelude::*,
    scene::ScenePlugin,
};
use bevy_radioactive::watch::*;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Track)]
struct Health(u32);

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Track)]
struct Score(u32);

#[derive(Component, Clone, Copy, Default)]
struct Enemy;

#[derive(Component, Clone, Copy, Default)]
struct Listener;

#[derive(Component, Clone, Default, Debug, PartialEq, Eq)]
struct Label(u32);

/// `(watch target, changed entity, value)` per delivery.
#[derive(Resource, Default)]
struct Log(Vec<(Entity, Entity, u32)>);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ))
    .init_resource::<Log>();
    app
}

fn log(app: &App) -> Vec<(Entity, Entity, u32)> {
    app.world().resource::<Log>().0.clone()
}

fn listener(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<Listener>>()
        .single(app.world())
        .unwrap()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn record_health(hp: Watch<Query<&Health>>, mut log: ResMut<Log>) {
    log.0.push((hp.target(), hp.entity(), hp.0));
}

#[test]
fn watch_query_sees_every_entity_once_per_frame_with_the_latest_value() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { Listener watch(record_health) })
        .unwrap();
    let a = app.world_mut().spawn(Health(1)).id();
    let b = app.world_mut().spawn(Health(2)).id();
    app.update();
    let target = listener(&mut app);
    let mut seen = log(&app);
    seen.sort();
    let mut expected = vec![(target, a, 1), (target, b, 2)];
    expected.sort();
    assert_eq!(seen, expected);

    app.world_mut().resource_mut::<Log>().0.clear();
    app.world_mut().get_mut::<Health>(a).unwrap().0 = 5;
    app.world_mut().get_mut::<Health>(a).unwrap().0 = 6;
    app.update();
    assert_eq!(log(&app), [(target, a, 6)]);

    app.update();
    assert_eq!(log(&app), [(target, a, 6)]);
}

#[test]
fn query_filters_select_entities() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(|hp: Watch<Query<&Health, With<Enemy>>>, mut log: ResMut<Log>| {
                log.0.push((hp.target(), hp.entity(), hp.0));
            })
        })
        .unwrap();
    app.world_mut().spawn(Health(1));
    let enemy = app.world_mut().spawn((Health(2), Enemy)).id();
    app.update();
    assert_eq!(log(&app), [(listener(&mut app), enemy, 2)]);
}

#[test]
fn disabled_entities_follow_query_default_filters() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                Label(1)
                watch(|hp: Watch<Query<&Health>>, mut log: ResMut<Log>| log.0.push((hp.target(), hp.entity(), 1)))
                --
                Label(2)
                watch(|hp: Watch<Query<&Health, Allow<Disabled>>>, mut log: ResMut<Log>| log.0.push((hp.target(), hp.entity(), 2)))
            ]
        })
        .unwrap();
    app.world_mut().spawn((Health(1), Disabled));
    app.update();
    let kinds: Vec<u32> = log(&app).into_iter().map(|(_, _, kind)| kind).collect();
    assert_eq!(kinds, [2]);
}

#[test]
fn watch_res_sees_the_resource_including_its_first_insertion() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(|score: Watch<Res<Score>>, mut log: ResMut<Log>| {
                log.0.push((score.target(), score.entity(), score.0));
            })
        })
        .unwrap();
    app.update();
    assert!(log(&app).is_empty());

    app.insert_resource(Score(3));
    app.update();
    let resource = app.world().resource_entity::<Score>().unwrap();
    assert_eq!(log(&app), [(listener(&mut app), resource, 3)]);

    app.world_mut().resource_mut::<Score>().0 = 4;
    app.update();
    assert_eq!(log(&app).last(), Some(&(listener(&mut app), resource, 4)));
}

#[test]
fn removed_runs_on_removal_and_despawn() {
    let mut app = app();
    app.insert_resource(Score(1));
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(|gone: Removed<Query<&Health>>, mut log: ResMut<Log>| {
                log.0.push((gone.target(), gone.entity(), 0));
            })
            watch(|gone: Removed<Res<Score>>, mut log: ResMut<Log>| {
                log.0.push((gone.target(), gone.entity(), 1));
            })
        })
        .unwrap();
    let a = app.world_mut().spawn(Health(1)).id();
    let b = app.world_mut().spawn(Health(2)).id();
    app.update();
    let target = listener(&mut app);
    let resource = app.world().resource_entity::<Score>().unwrap();

    app.world_mut().entity_mut(a).remove::<Health>();
    app.world_mut().despawn(b);
    app.world_mut().remove_resource::<Score>();
    app.world_mut().flush();
    assert_eq!(
        log(&app),
        [(target, a, 0), (target, b, 0), (target, resource, 1)]
    );
}

#[test]
fn bind_writes_the_current_value_on_spawn_and_follows_changes() {
    let mut app = app();
    app.insert_resource(Score(7));
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Label(0)
            bind(|score: &Score, label: &mut Label| label.0 = score.0)
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(app.world().get::<Label>(entity), Some(&Label(7)));

    app.world_mut().resource_mut::<Score>().0 = 8;
    app.update();
    assert_eq!(app.world().get::<Label>(entity), Some(&Label(8)));
}

#[test]
fn a_watch_stops_with_its_entity() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! { Listener watch(record_health) })
        .unwrap()
        .id();
    app.update();
    app.world_mut().despawn(root);
    app.world_mut().spawn(Health(1));
    app.update();
    assert!(log(&app).is_empty());
}

#[test]
fn app_watch_runs_without_a_scene() {
    let mut app = app();
    app.watch(|hp: Watch<Query<&Health>>, mut log: ResMut<Log>| {
        log.0.push((hp.target(), hp.entity(), hp.0));
    });
    let entity = app.world_mut().spawn(Health(9)).id();
    app.update();
    let seen = log(&app);
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].1, seen[0].2), (entity, 9));
}

#[test]
fn watch_without_the_plugin_reports_it() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    let Err(error) = app
        .world_mut()
        .spawn_scene(bsn! { Listener watch(record_health) })
    else {
        panic!("watch resolved without its plugin");
    };
    assert!(error.to_string().contains("WatchPlugin"));
}

#[test]
fn chained_handlers_settle_in_one_update() {
    let mut app = app();
    app.insert_resource(Score(0));
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            // Health changes bump the score; the score is shown on a label.
            watch(|hp: Watch<Query<&Health>>, mut score: ResMut<Score>| score.0 += hp.0)
            Label(0)
            bind(|score: &Score, label: &mut Label| label.0 = score.0)
        })
        .unwrap();
    app.update();
    app.world_mut().spawn(Health(5));
    app.update();
    let target = listener(&mut app);
    assert_eq!(app.world().get::<Label>(target), Some(&Label(5)));
}

#[test]
fn watchers_spawned_mid_frame_start_receiving() {
    let mut app = app();
    app.add_systems(PostUpdate, spawn_late.before(WatchSystems));
    app.update();
    app.world_mut().spawn(Health(3));
    app.update();
    assert_eq!(log(&app).len(), 1);
}

fn spawn_late(mut commands: Commands, mut done: Local<bool>) {
    if !std::mem::replace(&mut *done, true) {
        commands.spawn_scene(bsn! { Listener watch(record_health) });
    }
}
