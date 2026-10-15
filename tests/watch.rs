#![cfg(feature = "watch")]
use bevy::{asset::AssetPlugin, ecs::entity_disabling::Disabled, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
struct Counter(u32);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Unrelated(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Meter(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Echo(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Label(u32);

#[derive(Component, Clone, Copy, Default)]
struct SourceA;

#[derive(Component, Clone, Copy, Default)]
struct Selected;

#[derive(Component, Clone, Copy, Default)]
struct Listener;

#[derive(Resource, Default)]
struct Log(Vec<(Entity, u32)>);

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ))
    .init_resource::<Log>()
    .init_resource::<Unrelated>();
    app
}

fn settle(app: &mut App) {
    app.world_mut().flush();
    app.update();
}

fn runs(app: &App) -> usize {
    app.world().resource::<Log>().0.len()
}

fn label(app: &mut App) -> u32 {
    app.world_mut()
        .query_filtered::<&Label, With<Listener>>()
        .single(app.world())
        .unwrap()
        .0
}

fn listener(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<Listener>>()
        .single(app.world())
        .unwrap()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy systems receive parameters by value"
)]
fn record_counter(counter: Res<Counter>, this: This<Entity>, mut log: ResMut<Log>) {
    log.0.push((this.entity(), counter.0));
}

#[test]
fn runs_on_spawn_and_when_a_resource_changes() {
    let mut app = test_app();
    app.insert_resource(Counter(1));
    let first = app
        .world_mut()
        .spawn_scene(bsn! { Listener watch(record_counter) })
        .unwrap()
        .id();
    let second = app
        .world_mut()
        .spawn_scene(bsn! { watch(record_counter) })
        .unwrap()
        .id();
    settle(&mut app);
    let mut log = app.world().resource::<Log>().0.clone();
    log.sort_unstable();
    let mut expected = vec![(first, 1), (second, 1)];
    expected.sort_unstable();
    assert_eq!(log, expected);

    // Nothing changed: no re-run.
    settle(&mut app);
    assert_eq!(runs(&app), 2);

    // An equal write via `set_if_neq` is not a change.
    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(1));
    settle(&mut app);
    assert_eq!(runs(&app), 2);

    // A resource the system doesn't read is not a dependency.
    app.world_mut().resource_mut::<Unrelated>().0 += 1;
    settle(&mut app);
    assert_eq!(runs(&app), 2);

    // Several writes in one frame run each watch once, with the latest value.
    app.world_mut().resource_mut::<Counter>().0 = 2;
    app.world_mut().resource_mut::<Counter>().0 = 3;
    settle(&mut app);
    assert_eq!(runs(&app), 4);
    assert!(
        app.world().resource::<Log>().0[2..]
            .iter()
            .all(|e| e.1 == 3)
    );
}

#[test]
fn an_optional_resource_reruns_when_it_is_inserted() {
    let mut app = test_app();
    app.world_mut()
        .spawn_scene(bsn! {
            watch(|counter: Option<Res<Counter>>, mut log: ResMut<Log>| {
                log.0.push((Entity::PLACEHOLDER, counter.map_or(0, |counter| counter.0)));
            })
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(runs(&app), 1);

    app.insert_resource(Counter(5));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().0.last().unwrap().1, 5);
}

#[test]
fn writes_to_this_entity_update_it() {
    let mut app = test_app();
    app.insert_resource(Counter(1));
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            Label(0)
            watch(|counter: Res<Counter>, mut this: This<&mut Label>| {
                if let Some(mut label) = this.get_mut() {
                    label.0 = counter.0 * 10;
                }
            })
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(label(&mut app), 10);

    app.world_mut().resource_mut::<Counter>().0 = 2;
    settle(&mut app);
    assert_eq!(label(&mut app), 20);
}

#[test]
fn component_queries_react_to_change_insert_and_despawn() {
    let mut app = test_app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                SourceA Meter(3)
                --
                Listener
                Label(0)
                watch(|meters: Query<&Meter, With<SourceA>>, mut this: This<&mut Label>| {
                    let Some(mut label) = this.get_mut() else { return };
                    label.0 = meters.single().map_or(999, |meter| meter.0);
                })
            ]
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(label(&mut app), 3);

    let source = app
        .world_mut()
        .query_filtered::<Entity, With<SourceA>>()
        .single(app.world())
        .unwrap();
    app.world_mut().get_mut::<Meter>(source).unwrap().0 = 4;
    settle(&mut app);
    assert_eq!(label(&mut app), 4);

    app.world_mut().despawn(source);
    settle(&mut app);
    assert_eq!(label(&mut app), 999);

    app.world_mut().spawn((SourceA, Meter(7)));
    settle(&mut app);
    assert_eq!(label(&mut app), 7);
}

#[test]
fn filters_are_dependencies() {
    let mut app = test_app();
    let a = app.world_mut().spawn(Meter(1)).id();
    let b = app.world_mut().spawn(Meter(2)).id();
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            Label(0)
            watch(|selected: Query<Entity, With<Selected>>, mut this: This<&mut Label>| {
                if let Some(mut label) = this.get_mut() {
                    label.0 = u32::try_from(selected.iter().count()).unwrap();
                }
            })
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(label(&mut app), 0);

    app.world_mut().entity_mut(a).insert(Selected);
    app.world_mut().entity_mut(b).insert(Selected);
    settle(&mut app);
    assert_eq!(label(&mut app), 2);

    app.world_mut().entity_mut(a).remove::<Selected>();
    settle(&mut app);
    assert_eq!(label(&mut app), 1);
}

#[test]
fn own_writes_do_not_rerun_the_watch() {
    let mut app = test_app();
    app.insert_resource(Counter(0));
    app.world_mut()
        .spawn_scene(bsn! {
            watch(|mut counter: ResMut<Counter>, mut log: ResMut<Log>| {
                counter.0 += 1;
                log.0.push((Entity::PLACEHOLDER, counter.0));
            })
        })
        .unwrap();
    settle(&mut app);
    settle(&mut app);
    assert_eq!(runs(&app), 1);
    assert_eq!(app.world().resource::<Counter>().0, 1);
}

#[test]
fn chained_watches_settle_in_one_update() {
    let mut app = test_app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                SourceA Meter(1)
                --
                Echo(0)
                watch(|meters: Query<&Meter>, mut this: This<&mut Echo>| {
                    if let (Ok(meter), Some(mut echo)) = (meters.single(), this.get_mut()) {
                        echo.0 = meter.0 * 2;
                    }
                })
                --
                Listener
                Label(0)
                watch(|echoes: Query<&Echo>, mut this: This<&mut Label>| {
                    if let (Ok(echo), Some(mut label)) = (echoes.single(), this.get_mut()) {
                        label.0 = echo.0 + 1;
                    }
                })
            ]
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(label(&mut app), 3);

    let source = app
        .world_mut()
        .query_filtered::<Entity, With<SourceA>>()
        .single(app.world())
        .unwrap();
    app.world_mut().get_mut::<Meter>(source).unwrap().0 = 5;
    app.update();
    assert_eq!(label(&mut app), 11);
}

#[test]
fn one_entity_declares_several_watches() {
    let mut app = test_app();
    app.insert_resource(Counter(1));
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(record_counter)
            watch(|meters: Query<&Meter>, this: This<Entity>, mut log: ResMut<Log>| {
                for meter in &meters {
                    log.0.push((this.entity(), meter.0 + 100));
                }
            })
        })
        .unwrap();
    app.world_mut().spawn(Meter(1));
    settle(&mut app);
    assert_eq!(runs(&app), 2);

    app.world_mut().resource_mut::<Counter>().0 = 2;
    settle(&mut app);
    assert_eq!(runs(&app), 3);
    assert_eq!(app.world().resource::<Log>().0[2].1, 2);
}

#[test]
fn despawned_listener_stops_running() {
    let mut app = test_app();
    app.insert_resource(Counter(1));
    app.world_mut()
        .spawn_scene(bsn! { Listener watch(record_counter) })
        .unwrap();
    settle(&mut app);
    assert_eq!(runs(&app), 1);

    let listener = listener(&mut app);
    let before = app.world_mut().query::<Entity>().iter(app.world()).count();
    app.world_mut().despawn(listener);
    let after = app.world_mut().query::<Entity>().iter(app.world()).count();
    assert_eq!(before - after, 2, "the listener and its watcher despawn");

    app.world_mut().resource_mut::<Counter>().0 = 2;
    settle(&mut app);
    assert_eq!(runs(&app), 1);
}

fn spawn_dynamic_listener(mut commands: Commands) {
    commands.spawn_scene(bsn! { watch(record_counter) });
}

#[test]
fn watch_spawned_during_update_runs() {
    let mut app = test_app();
    app.insert_resource(Counter(1));
    app.add_systems(Update, spawn_dynamic_listener.run_if(run_once));
    settle(&mut app);
    assert_eq!(runs(&app), 1);

    app.world_mut().resource_mut::<Counter>().0 = 2;
    settle(&mut app);
    assert_eq!(runs(&app), 2);
}

#[test]
fn disabled_entities_are_seen_with_allow() {
    let mut app = test_app();
    let source = app.world_mut().spawn((Meter(1), Disabled)).id();
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            Label(0)
            watch(|meters: Query<&Meter, Allow<Disabled>>, mut this: This<&mut Label>| {
                if let (Ok(meter), Some(mut label)) = (meters.single(), this.get_mut()) {
                    label.0 = meter.0;
                }
            })
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(label(&mut app), 1);

    app.world_mut().get_mut::<Meter>(source).unwrap().0 = 2;
    settle(&mut app);
    assert_eq!(label(&mut app), 2);
}

#[test]
fn watch_without_plugin_reports_it() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
        .init_resource::<Log>();
    let Err(error) = app.world_mut().spawn_scene(bsn! { watch(record_counter) }) else {
        panic!("watch unexpectedly resolved without its plugin");
    };
    assert!(error.to_string().contains("WatchPlugin"));
}

#[test]
fn whole_world_access_is_rejected() {
    let mut app = test_app();
    let Err(error) = app
        .world_mut()
        .spawn_scene(bsn! { watch(|_: Query<EntityRef>| {}) })
    else {
        panic!("whole-world watch unexpectedly resolved");
    };
    assert!(error.to_string().contains("can't tell what"));
}
