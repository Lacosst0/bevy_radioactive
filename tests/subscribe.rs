#![cfg(feature = "subscribe")]
//! Integration coverage for source-preserving event subscriptions.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin, ui_widgets::ValueChange};
use bevy_radioactive::subscribe::*;

#[derive(Component, Clone, Copy, Default)]
struct Source;

#[derive(Component, Clone, Copy, Default)]
struct Subscriber;

#[derive(Component)]
struct CustomDisabled;

#[derive(Resource, Default, Debug, PartialEq)]
struct Log(Vec<(Entity, Entity, Entity, i32, bool)>);

#[derive(Resource, Default)]
struct NativeEventCount(usize);

#[derive(Clone, EntityEvent)]
struct Ping {
    #[event_target]
    source: Entity,
    value: i32,
}

fn settle(app: &mut App) {
    app.world_mut().flush();
    app.update();
    app.world_mut().flush();
}

fn single_entity<T: Component>(world: &mut World) -> Entity {
    let mut query = world.query_filtered::<Entity, With<T>>();
    let mut entities = query.iter(world);
    let entity = entities.next().expect("entity exists");
    assert!(entities.next().is_none(), "expected one entity");
    entity
}

fn subscription_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
        .init_resource::<Log>();
    app
}

#[test]
fn subscribe_to_preserves_original_source_and_payload() {
    let mut app = subscription_app();

    app.world_mut()
        .spawn_scene(bsn! {
            Name::new("subscription-root")
            Children[
                #Source Source
                --
                Subscriber
                subscribe_to(#Source, |event: Subscription<ValueChange<i32>>, mut log: ResMut<Log>| {
                    log.0.push((
                        event.target(),
                        event.source(),
                        event.event().source,
                        event.value,
                        event.is_final,
                    ));
                })
            ]
        })
        .unwrap();
    settle(&mut app);

    let source = single_entity::<Source>(app.world_mut());
    let subscriber = single_entity::<Subscriber>(app.world_mut());

    app.world_mut().trigger(ValueChange {
        source,
        value: 7,
        is_final: true,
    });
    assert_eq!(app.world().resource::<Log>().0.len(), 0);
    app.world_mut().flush();

    assert_eq!(
        app.world().resource::<Log>().0,
        vec![(subscriber, source, source, 7, true)]
    );
}

#[test]
fn forwarding_does_not_retrigger_the_original_global_event() {
    let mut app = subscription_app();
    app.init_resource::<NativeEventCount>().add_observer(
        |_event: On<ValueChange<i32>>, mut count: ResMut<NativeEventCount>| {
            count.0 += 1;
        },
    );

    app.world_mut()
        .spawn_scene(bsn! {
            Children[
                #Source Source
                --
                Subscriber
                subscribe_to(#Source, |_event: Subscription<ValueChange<i32>>| {})
            ]
        })
        .unwrap();
    settle(&mut app);

    let source = single_entity::<Source>(app.world_mut());
    app.world_mut().trigger(ValueChange {
        source,
        value: 11,
        is_final: false,
    });
    settle(&mut app);

    assert_eq!(app.world().resource::<NativeEventCount>().0, 1);
}

#[derive(Component, Clone, Copy, Default)]
struct First;

#[derive(Component, Clone, Copy, Default)]
struct Second;

#[test]
fn cyclic_subscriptions_do_not_recurse() {
    let mut app = subscription_app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children[
                #First First
                subscribe_to(#Second, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                    log.0.push((event.target(), event.source(), event.source, event.value, false));
                })
                --
                #Second Second
                subscribe_to(#First, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                    log.0.push((event.target(), event.source(), event.source, event.value, false));
                })
            ]
        })
        .unwrap();
    settle(&mut app);

    let first = single_entity::<First>(app.world_mut());
    let second = single_entity::<Second>(app.world_mut());
    app.world_mut().trigger(Ping {
        source: first,
        value: 23,
    });
    settle(&mut app);

    assert_eq!(
        app.world().resource::<Log>().0,
        vec![(second, first, first, 23, false)]
    );
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
struct Total(i32);

#[test]
fn bind_event_writes_the_declaring_entity() {
    let mut app = subscription_app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children[
                #Source Source
                --
                Subscriber
                Total(0)
                bind_event(#Source, |ping: &Ping, total: &mut Total| total.0 += ping.value)
            ]
        })
        .unwrap();
    settle(&mut app);

    let source = single_entity::<Source>(app.world_mut());
    app.world_mut().trigger(Ping { source, value: 2 });
    app.world_mut().trigger(Ping { source, value: 5 });
    app.world_mut().flush();

    let subscriber = single_entity::<Subscriber>(app.world_mut());
    assert_eq!(app.world().get::<Total>(subscriber), Some(&Total(7)));
}

#[test]
fn bsn_helper_allows_two_sources_for_the_same_event_type() {
    let mut app = subscription_app();

    app.world_mut()
        .spawn_scene(bsn! {
            Children[
                #A Source
                --
                #B Source
                --
                Subscriber
                subscribe_to(#A, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                    log.0.push((event.target(), event.source(), event.event().source, event.value, false));
                })
                subscribe_to(#B, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                    log.0.push((event.target(), event.source(), event.event().source, event.value + 100, false));
                })
            ]
        })
        .unwrap();
    settle(&mut app);

    let listener = single_entity::<Subscriber>(app.world_mut());
    let mut sources = app.world_mut().query_filtered::<Entity, With<Source>>();
    let sources: Vec<Entity> = sources.iter(app.world()).collect();
    app.world_mut().trigger(Ping {
        source: sources[0],
        value: 1,
    });
    app.world_mut().trigger(Ping {
        source: sources[1],
        value: 1,
    });
    app.world_mut().flush();

    let mut values: Vec<_> = app
        .world()
        .resource::<Log>()
        .0
        .iter()
        .map(|e| e.3)
        .collect();
    values.sort_unstable();
    assert_eq!(values, vec![1, 101]);
    assert!(
        app.world()
            .resource::<Log>()
            .0
            .iter()
            .all(|e| e.0 == listener)
    );
}

#[test]
fn bsn_helper_allows_multiple_handlers_for_the_same_source() {
    let mut app = subscription_app();

    app.world_mut()
        .spawn_scene(bsn! {
            Children[
                #Source Source
                --
                Subscriber
                subscribe_to(#Source, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                    log.0.push((event.target(), event.source(), event.event().source, event.value, false));
                })
                subscribe_to(#Source, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                    log.0.push((event.target(), event.source(), event.event().source, event.value + 100, false));
                })
            ]
        })
        .unwrap();
    settle(&mut app);

    let source = single_entity::<Source>(app.world_mut());
    let listener = single_entity::<Subscriber>(app.world_mut());
    app.world_mut().trigger(Ping { source, value: 31 });
    app.world_mut().flush();

    let mut values = app
        .world()
        .resource::<Log>()
        .0
        .iter()
        .map(|entry| entry.3)
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(values, vec![31, 131]);
    assert!(
        app.world()
            .resource::<Log>()
            .0
            .iter()
            .all(|entry| entry.0 == listener && entry.1 == source && entry.2 == source)
    );
}

#[test]
fn custom_disabled_sources_still_forward() {
    let mut app = subscription_app();
    app.world_mut()
        .register_disabling_component::<CustomDisabled>();

    let source = app.world_mut().spawn((Source, CustomDisabled)).id();
    let listener = app
        .world_mut()
        .spawn_scene(bsn! {
            Subscriber
            subscribe_to(source, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                log.0.push((event.target(), event.source(), event.source, event.value, false));
            })
        })
        .unwrap()
        .id();
    settle(&mut app);

    app.world_mut().trigger(Ping { source, value: 29 });
    app.world_mut().flush();

    assert_eq!(
        app.world().resource::<Log>().0,
        vec![(listener, source, source, 29, false)]
    );
}
