use bevy::{
    asset::AssetPlugin,
    ecs::{entity_disabling::Disabled, resource::IsResource},
    prelude::*,
    scene::ScenePlugin,
};
use bevy_radioactive::{
    RadioactivePlugin, Reaction, ReactiveAppExt, Removal, Singleton, ValueChanged, bind,
    bind_or_else, testing::settle, watch, watch_immediate, watch_removed,
};

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
    app.insert_resource(Counter(1)).observe_value(
        |event: On<ValueChanged<Counter>>, mut log: ResMut<Log>| {
            log.global.push((event.source(), event.0));
        },
    );

    let first = app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(Singleton, record_counter)
        })
        .unwrap()
        .id();
    let second = app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(Singleton, record_counter)
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
        .track::<Counter>()
        .track::<Counter>()
        .add_observer(|event: On<ValueChanged<Counter>>, mut log: ResMut<Log>| {
            log.global.push((event.source(), event.0));
        });

    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(Singleton, record_counter)
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
    app.insert_resource(Counter(1)).track::<Counter>();
    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(Singleton, record_counter)
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
    component_app.track::<Meter>();
    component_app
        .world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(7) Disabled
                --
                Listener
                watch(#A, |event: Reaction<Meter>, mut log: ResMut<Log>| {
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
    resource_app.insert_resource(Counter(8)).track::<Counter>();
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
            watch(Singleton, record_counter)
        })
        .unwrap();
    resource_app.world_mut().flush();
    resource_app.update();
    assert_eq!(resource_app.world().resource::<Log>().local.len(), 1);
    assert_eq!(resource_app.world().resource::<Log>().local[0].2, 8);
}

#[test]
fn component_watch_is_scoped_to_its_source() {
    let mut app = test_app();
    app.track::<Meter>();

    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(1)
                --
            #B SourceB Meter(10)
                --
                Listener
                watch(#A, |event: Reaction<Meter>, mut log: ResMut<Log>| {
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
        watch(Singleton, record_counter)
    });
}

#[test]
fn watcher_spawned_during_update_uses_pre_registered_publisher() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).track::<Counter>();
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
    app.insert_resource(Counter(1)).track::<Counter>();
    let listener = app
        .world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(Singleton, record_counter)
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
fn watch_tracks_its_type_on_first_use() {
    let mut app = test_app();
    app.add_plugins(RadioactivePlugin)
        .insert_resource(Counter(1));

    app.world_mut()
        .spawn_scene(bsn! {
            Listener
            watch(Singleton, record_counter)
        })
        .unwrap();
    settle(&mut app);
    // The first publisher run reports the current value once.
    assert_eq!(app.world().resource::<Log>().local.len(), 1);

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter(2));
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().local.len(), 2);
    assert_eq!(app.world().resource::<Log>().local[1].2, 2);
}

#[test]
fn watch_without_runtime_reports_the_plugin() {
    let mut app = test_app();
    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Children [
            #A SourceA Meter(1)
            --
            Listener
            watch(#A, |_: Reaction<Meter>| {})
        ]
    }) else {
        panic!("watch unexpectedly resolved without the runtime");
    };
    assert!(error.to_string().contains("RadioactivePlugin"));
}

#[test]
fn watch_resource_requires_a_current_resource_value() {
    let mut app = test_app();
    app.insert_resource(Counter(1)).track::<Counter>();
    app.world_mut().remove_resource::<Counter>();

    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Listener
        watch(Singleton, record_counter)
    }) else {
        panic!("watch unexpectedly resolved without its resource");
    };

    assert!(error.to_string().contains("must exist"));
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Label(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Echo(u32);

#[test]
fn one_listener_watches_several_sources_of_one_type() {
    let mut app = test_app();
    app.track::<Meter>();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(1)
                --
                #B SourceB Meter(10)
                --
                Listener
                watch([#A, #B], |event: Reaction<Meter>, mut log: ResMut<Log>| {
                    log.local.push((event.target(), event.source(), event.0));
                })
                watch(#A, |event: Reaction<Meter>, mut log: ResMut<Log>| {
                    log.global.push((event.source(), event.0));
                })
            ]
        })
        .unwrap();
    settle(&mut app);

    let listener = app
        .world_mut()
        .query_filtered::<Entity, With<Listener>>()
        .single(app.world())
        .unwrap();
    let mut values: Vec<_> = app
        .world()
        .resource::<Log>()
        .local
        .iter()
        .map(|e| (e.0, e.2))
        .collect();
    values.sort_unstable_by_key(|entry| entry.1);
    assert_eq!(values, vec![(listener, 1), (listener, 10)]);
    assert_eq!(app.world().resource::<Log>().global.len(), 1);

    app.world_mut().despawn(listener);
    let leftovers = app
        .world_mut()
        .query::<&bevy_radioactive::SubscriptionOf>()
        .iter(app.world())
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn bind_writes_target_and_bind_or_handles_despawn() {
    let mut app = test_app();
    app.track::<Meter>();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(3)
                --
                Listener
                Label(0)
                bind_or_else(
                    #A,
                    |meter: &Meter, label: &mut Label| label.0 = meter.0,
                    |label: &mut Label| label.0 = 999,
                )
            ]
        })
        .unwrap();
    settle(&mut app);

    let label = |app: &mut App| {
        app.world_mut()
            .query_filtered::<&Label, With<Listener>>()
            .single(app.world())
            .unwrap()
            .0
    };
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
    app.world_mut().flush();
    assert_eq!(label(&mut app), 999);
}

#[test]
fn watch_removed_fires_for_component_removal_and_resource_removal() {
    let mut app = test_app();
    app.insert_resource(Counter(1))
        .track::<Counter>()
        .track::<Meter>();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(1)
                --
                Listener
                watch_removed(#A, |r: Removal<Meter>, mut log: ResMut<Log>| log.local.push((r.target(), r.source(), 0)))
                watch_removed(Singleton, |r: Removal<Counter>, mut log: ResMut<Log>| log.global.push((r.source(), 0)))
            ]
        })
        .unwrap();
    settle(&mut app);
    let source = app
        .world_mut()
        .query_filtered::<Entity, With<SourceA>>()
        .single(app.world())
        .unwrap();

    app.world_mut().entity_mut(source).remove::<Meter>();
    app.world_mut().remove_resource::<Counter>();
    app.world_mut().flush();
    assert_eq!(app.world().resource::<Log>().local.len(), 1);
    assert_eq!(app.world().resource::<Log>().local[0].1, source);
    assert_eq!(app.world().resource::<Log>().global.len(), 1);
}

#[test]
fn chained_reactions_settle_in_one_update() {
    let mut app = test_app();
    app.track::<Meter>().track::<Echo>();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA Meter(1)
                --
                #E Echo(0)
                bind(#A, |meter: &Meter, echo: &mut Echo| echo.0 = meter.0 * 2)
                --
                Listener
                Label(0)
                bind(#E, |echo: &Echo, label: &mut Label| label.0 = echo.0 + 1)
            ]
        })
        .unwrap();
    app.world_mut().flush();
    app.update();
    let label = app
        .world_mut()
        .query_filtered::<&Label, With<Listener>>()
        .single(app.world())
        .unwrap()
        .0;
    assert_eq!(label, 3);
}

#[test]
fn bind_writes_the_current_value_on_spawn() {
    let mut app = test_app();
    app.insert_resource(Counter(7)).track::<Counter>();
    settle(&mut app);
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                Listener
                Label(0)
                bind(Singleton, |counter: &Counter, label: &mut Label| label.0 = counter.0)
                --
                Echo(0)
                bind(Singleton, |counter: &Counter, echo: &mut Echo| echo.0 = counter.0)
                    .deferred()
            ]
        })
        .unwrap();
    // The resource was inserted long ago: only replay can deliver it.
    settle(&mut app);
    settle(&mut app);
    let label = app
        .world_mut()
        .query_filtered::<&Label, With<Listener>>()
        .single(app.world())
        .unwrap()
        .0;
    assert_eq!(label, 7);
    let echo = app
        .world_mut()
        .query::<&Echo>()
        .single(app.world())
        .unwrap()
        .0;
    assert_eq!(echo, 0, "a deferred bind waits for a change");
}

#[test]
fn bind_or_replays_a_missing_source_and_watch_can_replay() {
    let mut app = test_app();
    app.add_plugins(RadioactivePlugin);
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #A SourceA
                --
                #B SourceB Meter(5)
                --
                Listener
                Label(0)
                bind_or_else(#A, |meter: &Meter, label: &mut Label| label.0 = meter.0, |label: &mut Label| label.0 = 999)
                watch_immediate(#B, |event: Reaction<Meter>, mut log: ResMut<Log>| {
                    log.local.push((event.target(), event.source(), event.0));
                })
            ]
        })
        .unwrap();
    settle(&mut app);
    let label = app
        .world_mut()
        .query_filtered::<&Label, With<Listener>>()
        .single(app.world())
        .unwrap()
        .0;
    assert_eq!(label, 999);
    // Once replayed, and once more by the first publisher run for `Meter`.
    assert!(
        app.world()
            .resource::<Log>()
            .local
            .iter()
            .all(|entry| entry.2 == 5)
    );
    assert_ne!(app.world().resource::<Log>().local.len(), 0);
}

#[test]
fn values_without_listeners_are_not_published() {
    let mut app = test_app();
    app.add_plugins(RadioactivePlugin).track::<Meter>();
    let source = app.world_mut().spawn(Meter(1)).id();
    settle(&mut app);
    app.world_mut().get_mut::<Meter>(source).unwrap().0 = 2;
    settle(&mut app);
    assert_eq!(
        app.world()
            .resource::<bevy_radioactive::ReactiveDiagnostics>()
            .publications,
        0
    );

    app.world_mut().entity_mut(source).observe(
        |_: On<ValueChanged<Meter>>, mut log: ResMut<Log>| {
            log.global.push((Entity::PLACEHOLDER, 0));
        },
    );
    app.world_mut().get_mut::<Meter>(source).unwrap().0 = 3;
    settle(&mut app);
    assert_eq!(app.world().resource::<Log>().global.len(), 1);
}
