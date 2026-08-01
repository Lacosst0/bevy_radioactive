//! Integration coverage for source-preserving event subscriptions.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin, ui_widgets::ValueChange};
use bevy_radioactive::subscription::{
    SubscribedTo, Subscription, SubscriptionAppExt, subscribe_to,
};

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
                (#Source Source),
                (
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
                )
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
    assert!(app.world().resource::<Log>().0.is_empty());
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
                (#Source Source),
                (
                    Subscriber
                    subscribe_to(#Source, |_event: Subscription<ValueChange<i32>>| {})
                )
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

#[test]
fn cyclic_relationships_do_not_recurse() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Log>()
        .register_subscription::<Ping>();

    let first = app.world_mut().spawn_empty().id();
    let second = app.world_mut().spawn_empty().id();

    app.world_mut()
        .entity_mut(first)
        .insert(SubscribedTo::<Ping>::new(second))
        .observe(|event: Subscription<Ping>, mut log: ResMut<Log>| {
            log.0.push((
                event.target(),
                event.source(),
                event.event().source,
                event.value,
                false,
            ));
        });
    app.world_mut()
        .entity_mut(second)
        .insert(SubscribedTo::<Ping>::new(first))
        .observe(|event: Subscription<Ping>, mut log: ResMut<Log>| {
            log.0.push((
                event.target(),
                event.source(),
                event.event().source,
                event.value,
                false,
            ));
        });
    settle(&mut app);

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

#[test]
#[should_panic(expected = "not registered")]
fn direct_relationship_requires_app_time_registration() {
    let mut app = subscription_app();
    let source = app.world_mut().spawn_empty().id();
    app.world_mut().spawn(SubscribedTo::<Ping>::new(source));
}

#[test]
fn bsn_helper_rejects_two_sources_for_the_same_event_type() {
    let mut app = subscription_app();

    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Children[
            (#A Source),
            (#B Source),
            (
                Subscriber
                subscribe_to(#A, |_: Subscription<Ping>| {})
                subscribe_to(#B, |_: Subscription<Ping>| {})
            )
        ]
    }) else {
        panic!("duplicate same-event subscription unexpectedly resolved");
    };

    assert!(error.to_string().contains("only one source"));
}

#[test]
fn bsn_helper_allows_multiple_handlers_for_the_same_source() {
    let mut app = subscription_app();

    app.world_mut()
        .spawn_scene(bsn! {
            Children[
                (#Source Source),
                (
                    Subscriber
                    subscribe_to(#Source, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                        log.0.push((event.target(), event.source(), event.event().source, event.value, false));
                    })
                    subscribe_to(#Source, |event: Subscription<Ping>, mut log: ResMut<Log>| {
                        log.0.push((event.target(), event.source(), event.event().source, event.value + 100, false));
                    })
                )
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
    app.register_subscription::<Ping>();
    app.world_mut()
        .register_disabling_component::<CustomDisabled>();

    let source = app.world_mut().spawn((Source, CustomDisabled)).id();
    let listener = app
        .world_mut()
        .spawn(SubscribedTo::<Ping>::new(source))
        .observe(|event: Subscription<Ping>, mut log: ResMut<Log>| {
            log.0.push((
                event.target(),
                event.source(),
                event.event().source,
                event.value,
                false,
            ));
        })
        .id();

    app.world_mut().trigger(Ping { source, value: 29 });
    app.world_mut().flush();

    assert_eq!(
        app.world().resource::<Log>().0,
        vec![(listener, source, source, 29, false)]
    );
}
