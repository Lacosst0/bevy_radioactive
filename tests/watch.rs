#![cfg(feature = "watch")]
//! Integration coverage for `watch`: tracked `Ref` inputs, plain system
//! params, `&mut C` writes, returned components and sources.

use bevy::{
    asset::AssetPlugin,
    ecs::{entity_disabling::Disabled, query::Allow, system::SystemParam},
    prelude::*,
    scene::ScenePlugin,
};
use bevy_radioactive::watch::*;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Health(u32);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
struct MaxHealth(u32);

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
struct Score(u32);

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Bonus(u32);

#[derive(Component, Clone, Copy, Default)]
struct Enemy;

#[derive(Component, Clone, Copy, Default)]
struct Listener;

#[derive(Component, Clone, Default, Debug, PartialEq, Eq)]
struct Label(u32);

/// `(source entity, value)` per run.
#[derive(Resource, Default)]
struct Log(Vec<(Entity, u32)>);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ))
    .init_resource::<Log>()
    .init_resource::<Bonus>();
    app
}

fn log(app: &App) -> Vec<(Entity, u32)> {
    app.world().resource::<Log>().0.clone()
}

fn clear(app: &mut App) {
    app.world_mut().resource_mut::<Log>().0.clear();
}

fn label(app: &App, entity: Entity) -> Option<u32> {
    app.world().get::<Label>(entity).map(|label| label.0)
}

#[test]
fn returned_component_is_inserted_on_spawn_and_on_change() {
    let mut app = app();
    app.insert_resource(Score(7));
    let entity = app
        .world_mut()
        .spawn_scene(bsn! { watch(|score: Ref<Score>| Label(score.0)) })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label(&app, entity), Some(7));

    app.world_mut().resource_mut::<Score>().0 = 8;
    app.update();
    assert_eq!(label(&app, entity), Some(8));
}

#[test]
fn mut_param_writes_a_component_of_the_declaring_entity() {
    let mut app = app();
    app.insert_resource(Score(3));
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Label(0)
            watch(|score: Ref<Score>, label: &mut Label| label.0 = score.0)
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label(&app, entity), Some(3));
}

#[test]
fn plain_params_are_read_but_do_not_trigger() {
    let mut app = app();
    app.insert_resource(Score(1));
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            watch(|score: Ref<Score>, bonus: Res<Bonus>| Label(score.0 + bonus.0))
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label(&app, entity), Some(1));

    app.world_mut().resource_mut::<Bonus>().0 = 10;
    app.update();
    assert_eq!(label(&app, entity), Some(1), "Res is not tracked");

    app.world_mut().resource_mut::<Score>().0 = 2;
    app.update();
    assert_eq!(
        label(&app, entity),
        Some(12),
        "but it is read on the next run"
    );
}

#[test]
fn runs_once_per_frame_with_the_latest_value() {
    let mut app = app();
    app.insert_resource(Score(1));
    app.world_mut()
        .spawn_scene(bsn! {
            watch(|e: Entity, score: Ref<Score>, mut log: ResMut<Log>| log.0.push((e, score.0)))
        })
        .unwrap();
    app.update();
    assert_eq!(log(&app).len(), 1);
    clear(&mut app);

    app.world_mut().resource_mut::<Score>().0 = 5;
    app.world_mut().resource_mut::<Score>().0 = 6;
    app.update();
    assert_eq!(log(&app).iter().map(|&(_, v)| v).collect::<Vec<_>>(), [6]);

    clear(&mut app);
    app.update();
    assert!(log(&app).is_empty(), "nothing changed");
}

#[test]
fn several_ref_inputs_each_trigger_and_is_changed_tells_which() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Health(5)
            MaxHealth(10)
            watch(|hp: Ref<Health>, max: Ref<MaxHealth>, mut log: ResMut<Log>| {
                log.0.push((Entity::PLACEHOLDER, u32::from(hp.is_changed()) * 10 + u32::from(max.is_changed())));
            })
        })
        .unwrap()
        .id();
    app.update();
    clear(&mut app);

    app.world_mut().get_mut::<MaxHealth>(entity).unwrap().0 = 20;
    app.update();
    assert_eq!(log(&app), [(Entity::PLACEHOLDER, 1)]);

    clear(&mut app);
    app.world_mut().get_mut::<Health>(entity).unwrap().0 = 4;
    app.update();
    assert_eq!(log(&app), [(Entity::PLACEHOLDER, 10)]);
}

#[test]
fn own_components_are_read_from_the_declaring_entity() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! { Health(5) watch(|hp: Ref<Health>| Label(hp.0)) })
        .unwrap()
        .id();
    // Another entity's Health doesn't count.
    app.world_mut().spawn(Health(100));
    app.update();
    assert_eq!(label(&app, entity), Some(5));

    let other = app.world_mut().spawn(Health(1)).id();
    app.world_mut().get_mut::<Health>(other).unwrap().0 = 2;
    app.update();
    assert_eq!(label(&app, entity), Some(5));

    app.world_mut().get_mut::<Health>(entity).unwrap().0 = 6;
    app.update();
    assert_eq!(label(&app, entity), Some(6));
}

#[test]
fn watch_from_a_named_entity() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Children [
                #Player Health(50)
                --
                Listener
                watch_from(#Player, |hp: Ref<Health>| Label(hp.0))
            ]
        })
        .unwrap()
        .id();
    app.update();
    let children: Vec<Entity> = app.world().get::<Children>(root).unwrap().to_vec();
    let (player, listener) = (children[0], children[1]);
    assert_eq!(label(&app, listener), Some(50));

    app.world_mut().get_mut::<Health>(player).unwrap().0 = 40;
    app.update();
    assert_eq!(label(&app, listener), Some(40));
}

#[test]
fn a_missing_ref_skips_the_run_until_it_exists() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! { watch(|score: Ref<Score>| Label(score.0)) })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label(&app, entity), None);

    app.insert_resource(Score(4));
    app.update();
    assert_eq!(label(&app, entity), Some(4));
}

#[test]
fn option_ref_runs_on_add_change_and_removal() {
    let mut app = app();
    let player = app.world_mut().spawn_empty().id();
    app.world_mut()
        .spawn_scene(bsn! {
            watch_from(player, |hp: Option<Ref<Health>>, mut log: ResMut<Log>| {
                // 0 = missing, 1 = added, 2 = changed
                let kind = match hp {
                    None => 0,
                    Some(hp) if hp.is_added() => 1,
                    Some(_) => 2,
                };
                log.0.push((Entity::PLACEHOLDER, kind));
            })
        })
        .unwrap();
    app.update();
    app.world_mut().entity_mut(player).insert(Health(1));
    app.update();
    app.world_mut().get_mut::<Health>(player).unwrap().0 = 2;
    app.update();
    app.world_mut().entity_mut(player).remove::<Health>();
    app.update();
    let kinds: Vec<u32> = log(&app).into_iter().map(|(_, kind)| kind).collect();
    assert_eq!(kinds, [0, 1, 2, 0]);
}

#[test]
fn option_ref_sees_despawn_and_resource_removal() {
    let mut app = app();
    app.insert_resource(Score(1));
    let player = app.world_mut().spawn(Health(1)).id();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                watch_from(player, |hp: Option<Ref<Health>>, mut log: ResMut<Log>| {
                    log.0.push((Entity::PLACEHOLDER, hp.map_or(0, |hp| hp.0)));
                })
                --
                watch(|score: Option<Ref<Score>>, mut log: ResMut<Log>| {
                    log.0.push((Entity::PLACEHOLDER, score.map_or(100, |s| 100 + s.0)));
                })
            ]
        })
        .unwrap();
    app.update();
    let mut seen: Vec<u32> = log(&app).into_iter().map(|(_, v)| v).collect();
    seen.sort_unstable();
    assert_eq!(seen, [1, 101]);
    clear(&mut app);

    app.world_mut().despawn(player);
    app.world_mut().remove_resource::<Score>();
    app.update();
    let mut seen: Vec<u32> = log(&app).into_iter().map(|(_, v)| v).collect();
    seen.sort_unstable();
    assert_eq!(seen, [0, 100]);
}

#[test]
fn any_with_runs_per_matching_entity() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            watch_from(any_with::<Enemy>(), |e: Entity, hp: Ref<Health>, mut log: ResMut<Log>| {
                log.0.push((e, hp.0));
            })
        })
        .unwrap();
    let existing = app.world_mut().spawn((Health(1), Enemy)).id();
    app.world_mut().spawn(Health(2));
    app.update();
    assert_eq!(log(&app), [(existing, 1)]);
    clear(&mut app);

    let a = app.world_mut().spawn((Health(3), Enemy)).id();
    let b = app.world_mut().spawn((Health(4), Enemy)).id();
    app.update();
    let mut seen = log(&app);
    seen.sort();
    let mut expected = vec![(a, 3), (b, 4)];
    expected.sort();
    assert_eq!(seen, expected);
}

#[test]
fn any_matching_follows_default_query_filters() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                watch_from(any(), |e: Entity, _: Ref<Health>, mut log: ResMut<Log>| log.0.push((e, 1)))
                --
                watch_from(any_matching::<Allow<Disabled>>(), |e: Entity, _: Ref<Health>, mut log: ResMut<Log>| log.0.push((e, 2)))
            ]
        })
        .unwrap();
    app.update();
    app.world_mut().spawn((Health(1), Disabled));
    app.update();
    let kinds: Vec<u32> = log(&app).into_iter().map(|(_, kind)| kind).collect();
    assert_eq!(kinds, [2]);
}

#[test]
fn a_watch_stops_with_its_entity() {
    let mut app = app();
    app.insert_resource(Score(1));
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            watch(|score: Ref<Score>, mut log: ResMut<Log>| log.0.push((Entity::PLACEHOLDER, score.0)))
        })
        .unwrap()
        .id();
    app.update();
    clear(&mut app);
    app.world_mut().despawn(root);
    app.world_mut().resource_mut::<Score>().0 = 2;
    app.update();
    assert!(log(&app).is_empty());
}

#[test]
fn app_watch_runs_without_a_scene() {
    let mut app = app();
    app.insert_resource(Score(1));
    app.watch(|score: Ref<Score>, mut log: ResMut<Log>| log.0.push((Entity::PLACEHOLDER, score.0)));
    app.watch_from(any(), |e: Entity, hp: Ref<Health>, mut log: ResMut<Log>| {
        log.0.push((e, hp.0));
    });
    let entity = app.world_mut().spawn(Health(9)).id();
    app.update();
    let mut seen = log(&app);
    seen.sort();
    let mut expected = vec![(Entity::PLACEHOLDER, 1), (entity, 9)];
    expected.sort();
    assert_eq!(seen, expected);
}

#[test]
fn watch_without_the_plugin_reports_it() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    let Err(error) = app
        .world_mut()
        .spawn_scene(bsn! { watch(|score: Ref<Score>| Label(score.0)) })
    else {
        panic!("watch resolved without its plugin");
    };
    assert!(error.to_string().contains("WatchPlugin"));
}

#[test]
fn chained_watches_settle_in_one_update() {
    let mut app = app();
    app.insert_resource(Score(0));
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Children [
                Health(0)
                // Health changes bump the score; the score is shown on a label.
                watch(|hp: Ref<Health>, mut score: ResMut<Score>| score.0 = hp.0 * 2)
            ]
            watch(|score: Ref<Score>| Label(score.0))
        })
        .unwrap()
        .id();
    app.update();
    let child = app.world().get::<Children>(entity).unwrap()[0];
    app.world_mut().get_mut::<Health>(child).unwrap().0 = 5;
    app.update();
    assert_eq!(label(&app, entity), Some(10));
}

#[test]
fn watchers_spawned_mid_frame_run_that_frame() {
    let mut app = app();
    app.insert_resource(Score(3));
    app.add_systems(PostUpdate, spawn_late.before(WatchSystems));
    app.update();
    assert_eq!(log(&app).len(), 1);
}

fn spawn_late(mut commands: Commands, mut done: Local<bool>) {
    if !std::mem::replace(&mut *done, true) {
        commands.spawn_scene(bsn! {
            watch(|score: Ref<Score>, mut log: ResMut<Log>| log.0.push((Entity::PLACEHOLDER, score.0)))
        });
    }
}

#[test]
fn every_kind_of_system_param_is_accepted() {
    #[derive(SystemParam)]
    struct Custom<'w> {
        score: Res<'w, Score>,
    }

    let mut app = app();
    app.insert_resource(Score(2));
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Label(0)
            watch(|
                score: Ref<Score>,
                _query: Query<&Health>,
                _single: Option<Single<&Enemy>>,
                mut commands: Commands,
                mut counter: Local<u32>,
                custom: Param<Custom>,
                label: &mut Label,
            | {
                *counter += 1;
                label.0 = score.0 + custom.0.score.0;
                commands.spawn(Enemy);
            })
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label(&app, entity), Some(4));
    assert_eq!(
        app.world_mut()
            .query_filtered::<(), With<Enemy>>()
            .iter(app.world())
            .count(),
        1
    );
}

#[test]
fn lifecycle_uses_plain_bevy_observers() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Health(1)
            on(|_: On<Add<Health>>, mut log: ResMut<Log>| log.0.push((Entity::PLACEHOLDER, 1)))
            on(|_: On<Remove<Health>>, mut log: ResMut<Log>| log.0.push((Entity::PLACEHOLDER, 0)))
        })
        .unwrap()
        .id();
    app.world_mut().entity_mut(entity).remove::<Health>();
    let kinds: Vec<u32> = log(&app).into_iter().map(|(_, kind)| kind).collect();
    assert_eq!(kinds, [1, 0]);
}

#[test]
fn a_watch_runs_once_for_an_input_written_earlier_in_the_same_round() {
    let mut app = app();
    app.insert_resource(Score(1));
    app.world_mut()
        .spawn_scene(bsn! {
            watch(|score: Ref<Score>| Label(score.0))
            watch(|label: Ref<Label>, mut log: ResMut<Log>| log.0.push((Entity::PLACEHOLDER, label.0)))
        })
        .unwrap();
    app.update();
    let values: Vec<u32> = log(&app).into_iter().map(|(_, v)| v).collect();
    assert_eq!(values, [1]);
}
