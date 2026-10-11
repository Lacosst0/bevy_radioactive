use bevy::{
    asset::AssetPlugin,
    ecs::{entity_disabling::Disabled, resource::IsResource},
    prelude::*,
    scene::ScenePlugin,
};
use bevy_radioactive::{Reaction, ReactiveAppExt, ValueChanged, watch_component, watch_resource};

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
struct Counter(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Meter(u32);

#[derive(Component, Clone, Copy, Default)]
struct SourceA;

#[derive(Component, Clone, Copy, Default)]
struct SourceB;

#[derive(Component, Clone, Copy, Default)]
struct Listener;

#[derive(Resource, Default)]
struct Log {
    global: Vec<(Entity, u32)>,
    local: Vec<(Entity, Entity, u32)>,
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
        .init_resource::<Log>();
    app
}

fn settle(app: &mut App) {
    app.update();
    app.world_mut().flush();
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer systems receive SystemInput parameters by value"
)]
fn record_counter(event: Reaction<Counter>, mut log: ResMut<Log>) {
    log.local
        .push((event.target(), event.source(), event.event().0));
}

#[test]
fn resource_change_runs_global_once_and_each_local_watcher_once() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).observe_resource(
        |event: On<ValueChanged<Counter>>, mut log: ResMut<Log>| {
            log.global.push((event.source(), event.0));
        },
    );

    let first = app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch_resource(record_counter)
        })
        .unwrap()
        .id();
    let second = app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch_resource(record_counter)
        })
        .unwrap()
        .id();

    settle(&mut app);

    let source = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, (With<Counter>, With<IsResource>)>();
        query.single(world).unwrap()
    };
    assert_eq!(app.world().resource::<Log>().global, vec![(source, 1)]);
    let mut local = app.world().resource::<Log>().local.clone();
    local.sort_unstable_by_key(|entry| entry.0);
    let mut expected = vec![(first, source, 1), (second, source, 1)];
    expected.sort_unstable_by_key(|entry| entry.0);
    assert_eq!(local, expected);

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(1));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 1);
    assert_eq!(app.world().resource::<Log>().local.len(), 2);

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(2));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 2);
    assert_eq!(app.world().resource::<Log>().local.len(), 4);

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(3));
    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(4));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 3);
    assert_eq!(app.world().resource::<Log>().global[2].1, 4);
    assert_eq!(app.world().resource::<Log>().local.len(), 6);
    assert!(
        app.world().resource::<Log>().local[4..]
            .iter()
            .all(|entry| entry.2 == 4)
    );
}

#[test]
fn duplicate_registration_is_idempotent_and_reinsertion_is_published() {
    let mut app = test_app();
    app.insert_resource(Counter(1))
        .track_resource::<Counter>()
        .track_resource::<Counter>()
        .add_observer(|event: On<ValueChanged<Counter>>, mut log: ResMut<Log>| {
            log.global.push((event.source(), event.0));
        });

    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch_resource(record_counter)
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 1);
    assert_eq!(app.world().resource::<Log>().local.len(), 1);

    app.world_mut().remove_resource::<Counter>();
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 1);
    assert_eq!(app.world().resource::<Log>().local.len(), 1);

    app.world_mut().insert_resource(Counter(2));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 2);
    assert_eq!(app.world().resource::<Log>().global[1].1, 2);
    assert_eq!(app.world().resource::<Log>().local.len(), 2);
    assert_eq!(app.world().resource::<Log>().local[1].2, 2);
}

#[test]
fn app_update_flushes_scheduled_publication_to_local_watchers() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).track_resource::<Counter>();
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch_resource(record_counter)
        })
        .unwrap();

    // Resolve the scene commands before the frame, then rely only on App::update
    // for publication and nested subscription delivery.
    app.world_mut().flush();
    app.update();

    assert_eq!(app.world().resource::<Log>().local.len(), 1);
    assert_eq!(app.world().resource::<Log>().local[0].2, 1);
}

#[test]
fn disabled_sources_still_publish() {
    let mut component_app = test_app();
    component_app.track_component::<Meter>();
    component_app
        .world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(7) Disabled
                --
                Listener
                watch_component(#A, |event: Reaction<Meter>, mut log: ResMut<Log>| {
                    log.local.push((event.target(), event.source(), event.event().0));
                })
            ]
        })
        .unwrap();
    component_app.world_mut().flush();
    component_app.update();
    assert_eq!(component_app.world().resource::<Log>().local.len(), 1);
    assert_eq!(component_app.world().resource::<Log>().local[0].2, 7);

    let mut resource_app = test_app();
    resource_app
        .insert_resource(Counter(8))
        .track_resource::<Counter>();
    let source = {
        let world = resource_app.world_mut();
        let mut query = world.query_filtered::<Entity, (With<Counter>, With<IsResource>)>();
        query.single(world).unwrap()
    };
    resource_app.world_mut().entity_mut(source).insert(Disabled);
    resource_app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch_resource(record_counter)
        })
        .unwrap();
    resource_app.world_mut().flush();
    resource_app.update();
    assert_eq!(resource_app.world().resource::<Log>().local.len(), 1);
    assert_eq!(resource_app.world().resource::<Log>().local[0].2, 8);
}

#[test]
#[should_panic(expected = "cannot be tracked as both")]
fn one_type_cannot_be_tracked_as_both_a_resource_and_component() {
    let mut app = test_app();
    app.track_resource::<Counter>().track_component::<Counter>();
}

#[test]
fn component_watch_is_scoped_to_its_source() {
    let mut app = test_app();
    app.track_component::<Meter>();

    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(1)
                --
            #B SourceB Meter(10)
                --
                Listener
                watch_component(#A, |event: Reaction<Meter>, mut log: ResMut<Log>| {
                    log.local.push((event.target(), event.source(), event.event().0));
                })
            ]
        })
        .unwrap();
    settle(&mut app);

    let source_a = app
        .world_mut()
        .query_filtered::<Entity, With<SourceA>>()
        .single(app.world())
        .unwrap();
    let source_b = app
        .world_mut()
        .query_filtered::<Entity, With<SourceB>>()
        .single(app.world())
        .unwrap();
    assert_eq!(app.world().resource::<Log>().local.len(), 1);
    assert_eq!(app.world().resource::<Log>().local[0].1, source_a);

    app.world_mut()
        .entity_mut(source_b)
        .get_mut::<Meter>()
        .unwrap()
        .set_if_neq(Meter(11));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 1);

    app.world_mut()
        .entity_mut(source_a)
        .get_mut::<Meter>()
        .unwrap()
        .set_if_neq(Meter(2));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 2);
    assert_eq!(app.world().resource::<Log>().local[1].2, 2);
}

fn spawn_dynamic_listener(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Listener
        watch_resource(record_counter)
    });
}

#[test]
fn watcher_spawned_during_update_uses_pre_registered_publisher() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).track_resource::<Counter>();
    settle(&mut app);

    app.add_systems(Update, spawn_dynamic_listener.run_if(run_once));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 0);

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(2));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 1);
    assert_eq!(app.world().resource::<Log>().local[0].2, 2);
}

#[test]
fn despawned_watcher_stops_receiving_changes() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).track_resource::<Counter>();
    let listener = app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch_resource(record_counter)
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 1);

    app.world_mut().despawn(listener);
    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(2));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 1);
}

#[test]
fn watch_resource_requires_app_time_registration() {
    let mut app = test_app();
    app.insert_resource(Counter(1));

    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Listener
        watch_resource(record_counter)
    }) else {
        panic!("unregistered watch unexpectedly resolved");
    };

    assert!(error.to_string().contains("track_resource"));
}

#[test]
fn watch_resource_requires_a_current_resource_value() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).track_resource::<Counter>();
    app.world_mut().remove_resource::<Counter>();

    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Listener
        watch_resource(record_counter)
    }) else {
        panic!("watch unexpectedly resolved without its resource");
    };

    assert!(error.to_string().contains("must exist"));
}

#[test]
fn watch_component_requires_app_time_registration() {
    let mut app = test_app();

    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Children [
            #A SourceA Meter(1)
            --
            Listener
            watch_component(#A, |_: Reaction<Meter>| {})
        ]
    }) else {
        panic!("unregistered component watch unexpectedly resolved");
    };

    assert!(error.to_string().contains("track_component"));
}
