#![cfg(feature = "reactive")]
//! Integration coverage for `reactive`: watching, context lookup, regions,
//! keyed lists, event handlers, cleanup and use outside scenes.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Count(i32);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Score(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Health(u32);

#[derive(Component, Clone, Default, Debug, PartialEq, Eq)]
struct Label(String);

#[derive(Component, Clone, Copy, Default)]
struct Enemy;

#[derive(Component, Clone, Copy, Default)]
struct Row;

#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    title: &'static str,
}

#[derive(Resource, Clone, Default)]
struct Todos(Vec<Todo>);

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Tab {
    #[default]
    Home,
    Settings,
}

#[derive(Resource, Default)]
struct Runs(u32);

#[derive(Clone, EntityEvent)]
struct Poke {
    entity: Entity,
    by: i32,
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Score>()
    .init_resource::<Runs>()
    .init_resource::<Todos>()
    .init_resource::<Tab>();
    app
}

fn labels(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &Label)>();
    let mut found: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, label)| (entity, label.0.clone()))
        .collect();
    // Sorted by position among siblings, then by entity, for stable output.
    found.sort_by_key(|(entity, _)| sibling_index(world, *entity));
    found.into_iter().map(|(_, label)| label).collect()
}

fn sibling_index(world: &World, entity: Entity) -> (usize, Entity) {
    let index = world
        .get::<ChildOf>(entity)
        .and_then(|parent| world.get::<Children>(parent.parent()))
        .and_then(|children| children.iter().position(|child| child == entity))
        .unwrap_or(0);
    (index, entity)
}

fn label_of(app: &App, entity: Entity) -> String {
    app.world().get::<Label>(entity).expect("label").0.clone()
}

fn named(app: &mut App, name: &str) -> Entity {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &Name)>();
    query
        .iter(world)
        .find(|(_, n)| n.as_str() == name)
        .map_or_else(|| panic!("no entity named {name}"), |(entity, _)| entity)
}

fn entities_with<T: Component>(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<T>>();
    query.iter(world).collect()
}

// ---------------------------------------------------------------------------
// watch
// ---------------------------------------------------------------------------

#[test]
fn watch_runs_on_spawn_and_when_a_resource_changes() {
    let mut app = app();
    let label = app
        .world_mut()
        .spawn_scene(bsn! {
            Label
            watch(|score: Res<Score>| Label(format!("score {}", score.0)))
        })
        .unwrap()
        .id();
    assert_eq!(label_of(&app, label), "score 0", "runs right after spawn");

    app.world_mut().resource_mut::<Score>().0 = 7;
    app.update();
    assert_eq!(label_of(&app, label), "score 7");
}

#[test]
fn ref_finds_the_nearest_ancestor_and_counters_stay_independent() {
    let mut app = app();
    let counter = || {
        bsn! {
            Count(0)
            Children [
                Label
                watch(|count: Ref<Count>| Label(format!("{}", count.0)))
            ]
        }
    };
    let a = app.world_mut().spawn_scene(counter()).unwrap().id();
    let b = app.world_mut().spawn_scene(counter()).unwrap().id();
    app.update();

    app.world_mut().get_mut::<Count>(a).unwrap().0 = 5;
    app.update();
    let label_a = app.world().get::<Children>(a).unwrap()[0];
    let label_b = app.world().get::<Children>(b).unwrap()[0];
    assert_eq!(label_of(&app, label_a), "5");
    assert_eq!(
        label_of(&app, label_b),
        "0",
        "the other counter is untouched"
    );
}

#[test]
fn ref_falls_back_to_the_resource() {
    let mut app = app();
    let label = app
        .world_mut()
        .spawn_scene(bsn! {
            Label
            watch(|score: Ref<Score>| Label(format!("{}", score.0)))
        })
        .unwrap()
        .id();
    app.world_mut().resource_mut::<Score>().0 = 3;
    app.update();
    assert_eq!(label_of(&app, label), "3");
}

#[test]
fn mut_param_writes_the_nearest_component_without_rerunning() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Count(1)
            Children [
                Health(10)
                watch(|hp: Ref<Health>, count: &mut Count, mut runs: ResMut<Runs>| {
                    count.0 = hp.0.cast_signed();
                    runs.0 += 1;
                })
            ]
        })
        .unwrap()
        .id();
    app.update();
    app.update();
    assert_eq!(app.world().get::<Count>(root), Some(&Count(10)));
    assert_eq!(
        app.world().resource::<Runs>().0,
        1,
        "a write isn't a trigger"
    );
}

#[test]
fn query_params_track_spawns_changes_and_despawns() {
    let mut app = app();
    let label = app
        .world_mut()
        .spawn_scene(bsn! {
            Label
            watch(|enemies: Query<&Health, With<Enemy>>| {
                let total: u32 = enemies.iter().map(|hp| hp.0).sum();
                Label(format!("{} enemies, {} hp", enemies.iter().count(), total))
            })
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label_of(&app, label), "0 enemies, 0 hp");

    let goblin = app.world_mut().spawn((Enemy, Health(5))).id();
    app.update();
    assert_eq!(label_of(&app, label), "1 enemies, 5 hp");

    app.world_mut().get_mut::<Health>(goblin).unwrap().0 = 2;
    app.update();
    assert_eq!(label_of(&app, label), "1 enemies, 2 hp");

    app.world_mut().despawn(goblin);
    app.update();
    assert_eq!(label_of(&app, label), "0 enemies, 0 hp");
}

#[test]
fn source_redirects_lookups_for_a_subtree() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            #Root
            Children [
                #Player Health(100)
                --
                Source(#Player)
                Children [
                    #Hp Label
                    watch(|hp: Ref<Health>| Label(format!("HP {}", hp.0)))
                ]
            ]
        })
        .unwrap();
    let player = named(&mut app, "Player");
    let hp = named(&mut app, "Hp");
    assert_eq!(label_of(&app, hp), "HP 100");
    app.world_mut().get_mut::<Health>(player).unwrap().0 = 40;
    app.update();
    assert_eq!(label_of(&app, hp), "HP 40");
}

#[test]
fn a_returned_component_is_a_derived_value_for_descendants() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Health(3)
            watch(|hp: Ref<Health>| Count(hp.0.cast_signed() * 2))
            Children [
                Label
                watch(|doubled: Ref<Count>| Label(format!("{}", doubled.0)))
            ]
        })
        .unwrap()
        .id();
    app.update();
    let label = app.world().get::<Children>(root).unwrap()[0];
    assert_eq!(label_of(&app, label), "6");
    app.world_mut().get_mut::<Health>(root).unwrap().0 = 5;
    app.update();
    assert_eq!(
        label_of(&app, label),
        "10",
        "chained reactions settle in one frame"
    );
}

#[test]
fn a_missing_ref_skips_runs_until_it_appears() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Label
            watch(|hp: Ref<Health>| Label(format!("{}", hp.0)))
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(label_of(&app, entity), "");
    app.world_mut().entity_mut(entity).insert(Health(9));
    app.update();
    assert_eq!(label_of(&app, entity), "9");
}

#[test]
fn option_ref_sees_removal() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Health(1)
            watch(|hp: Option<Ref<Health>>| Label(format!("{:?}", hp.map(|hp| hp.0))))
        })
        .unwrap()
        .id();
    assert_eq!(label_of(&app, entity), "Some(1)");
    app.world_mut().entity_mut(entity).remove::<Health>();
    app.update();
    assert_eq!(label_of(&app, entity), "None");
}

#[test]
fn untracked_params_do_not_rerun() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            watch(|_score: Untracked<Res<Score>>, mut runs: ResMut<Runs>| runs.0 += 1)
        })
        .unwrap();
    app.update();
    app.world_mut().resource_mut::<Score>().0 += 1;
    app.update();
    assert_eq!(app.world().resource::<Runs>().0, 1);
}

// ---------------------------------------------------------------------------
// when / switch
// ---------------------------------------------------------------------------

#[test]
fn when_otherwise_switches_branches_and_despawns_the_old_one() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Count(0)
            Children [
                {when(|count: Ref<Count>| count.0 > 0, || bsn! { Label("positive") })
                    .else_when(|count: Ref<Count>| count.0 < 0, || bsn! { Label("negative") })
                    .otherwise(|| bsn! { Label("zero") })}
            ]
        })
        .unwrap();
    assert_eq!(
        labels(&mut app),
        ["zero"],
        "content exists right after spawn"
    );

    let root = entities_with::<Count>(&mut app)[0];
    app.world_mut().get_mut::<Count>(root).unwrap().0 = 3;
    app.update();
    assert_eq!(labels(&mut app), ["positive"]);
    let positive = entities_with::<Label>(&mut app)[0];

    app.world_mut().get_mut::<Count>(root).unwrap().0 = 4;
    app.update();
    assert_eq!(
        entities_with::<Label>(&mut app),
        [positive],
        "same branch: not rebuilt"
    );

    app.world_mut().get_mut::<Count>(root).unwrap().0 = -1;
    app.update();
    assert_eq!(labels(&mut app), ["negative"]);
}

#[test]
fn region_is_a_child_with_the_content_as_its_children() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Children [
                Label("before")
                --
                {when(|| true, || bsn_list! { Label("a") -- Label("b") })}
                --
                Label("after")
            ]
        })
        .unwrap()
        .id();
    let children: Vec<Entity> = app.world().get::<Children>(root).unwrap().to_vec();
    assert_eq!(children.len(), 3);
    let region = children[1];
    assert!(app.world().get::<Region>(region).is_some());
    let inner: Vec<String> = app
        .world()
        .get::<Children>(region)
        .unwrap()
        .iter()
        .map(|child| label_of(&app, child))
        .collect();
    assert_eq!(inner, ["a", "b"]);
}

#[test]
fn switch_rebuilds_only_when_the_key_changes() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                {switch(|tab: Res<Tab>| *tab, |tab| match tab {
                    Tab::Home => bsn! { Label("home") }.into(),
                    Tab::Settings => bsn_list! { Label("settings") -- Label("more") }.into(),
                })}
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["home"]);
    *app.world_mut().resource_mut::<Tab>() = Tab::Settings;
    app.update();
    assert_eq!(labels(&mut app), ["settings", "more"]);
}

#[test]
fn nested_regions_clean_up_with_their_branch() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Count(1)
            Children [
                {when(|count: Ref<Count>| count.0 > 0, || bsn! {
                    Health(1)
                    Children [
                        {when(|hp: Ref<Health>| hp.0 > 0, || bsn! {
                            Label("inner")
                            watch(|score: Res<Score>| Label(format!("inner {}", score.0)))
                        })}
                    ]
                })}
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["inner 0"]);
    let reactions_before = entities_with::<ReactionOf>(&mut app).len();
    assert_eq!(reactions_before, 3);

    let root = entities_with::<Count>(&mut app)[0];
    app.world_mut().get_mut::<Count>(root).unwrap().0 = 0;
    app.update();
    assert!(labels(&mut app).is_empty());
    assert_eq!(
        entities_with::<ReactionOf>(&mut app).len(),
        1,
        "only the outer region's reaction is left"
    );
    assert!(entities_with::<Health>(&mut app).is_empty());
}

// ---------------------------------------------------------------------------
// each
// ---------------------------------------------------------------------------

fn todo(id: u32, title: &'static str) -> Todo {
    Todo { id, title }
}

#[test]
fn keyed_each_keeps_unchanged_rows_and_reorders() {
    let mut app = app();
    app.world_mut().resource_mut::<Todos>().0 = vec![todo(1, "a"), todo(2, "b"), todo(3, "c")];
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                {each(|todos: Res<Todos>| todos.0.clone(), |todo| {
                    let title = todo.title;
                    bsn! { Row Label({title.to_string()}) }
                })
                .key(|todo| todo.id)}
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["a", "b", "c"]);
    let mut by_title = std::collections::HashMap::new();
    for entity in entities_with::<Row>(&mut app) {
        by_title.insert(label_of(&app, entity), entity);
    }

    app.world_mut().resource_mut::<Todos>().0 = vec![todo(3, "c"), todo(1, "a"), todo(4, "d")];
    app.update();
    assert_eq!(labels(&mut app), ["c", "a", "d"]);
    let rows = entities_with::<Row>(&mut app);
    assert!(rows.contains(&by_title["a"]), "kept row a");
    assert!(rows.contains(&by_title["c"]), "kept row c");
    assert!(!rows.contains(&by_title["b"]), "removed row b");

    app.world_mut().resource_mut::<Todos>().0[0].title = "C";
    app.update();
    assert_eq!(
        labels(&mut app),
        ["C", "a", "d"],
        "a changed item rebuilds its row"
    );
}

#[test]
fn unkeyed_each_rebuilds_only_changed_positions() {
    let mut app = app();
    app.world_mut().resource_mut::<Todos>().0 = vec![todo(1, "a"), todo(2, "b")];
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                {each(|todos: Res<Todos>| todos.0.clone(), |todo| {
                    let title = todo.title;
                    bsn! { Row Label({title.to_string()}) }
                })}
            ]
        })
        .unwrap();
    let first = entities_with::<Row>(&mut app)
        .into_iter()
        .find(|&row| label_of(&app, row) == "a")
        .unwrap();
    app.world_mut().resource_mut::<Todos>().0[1].title = "B";
    app.update();
    assert_eq!(labels(&mut app), ["a", "B"]);
    assert!(entities_with::<Row>(&mut app).contains(&first));
}

#[test]
fn each_over_entities_with_source_updates_rows_in_place() {
    let mut app = app();
    let goblin = app.world_mut().spawn((Enemy, Health(3))).id();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                {each(
                    |enemies: Query<Entity, With<Enemy>>| {
                        let mut enemies: Vec<Entity> = enemies.iter().collect();
                        enemies.sort();
                        enemies
                    },
                    |&enemy| bsn! {
                        Row
                        Source({enemy})
                        Label
                        watch(|hp: Ref<Health>| Label(format!("hp {}", hp.0)))
                    },
                )
                .key(|&enemy| enemy)}
            ]
        })
        .unwrap();
    app.update();
    assert_eq!(labels(&mut app), ["hp 3"]);
    let row = entities_with::<Row>(&mut app)[0];

    app.world_mut().get_mut::<Health>(goblin).unwrap().0 = 1;
    app.update();
    assert_eq!(labels(&mut app), ["hp 1"]);
    assert_eq!(
        entities_with::<Row>(&mut app),
        [row],
        "patched, not rebuilt"
    );

    app.world_mut().spawn((Enemy, Health(8)));
    app.update();
    assert_eq!(labels(&mut app).len(), 2);

    app.world_mut().despawn(goblin);
    app.update();
    assert_eq!(labels(&mut app), ["hp 8"]);
}

// ---------------------------------------------------------------------------
// handle, Nearest
// ---------------------------------------------------------------------------

#[test]
fn handle_writes_the_nearest_component() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Count(0)
            Children [
                #Button
                handle(|poke: &Poke, count: &mut Count| count.0 += poke.by)
                --
                Label
                watch(|count: Ref<Count>| Label(format!("{}", count.0)))
            ]
        })
        .unwrap()
        .id();
    let button = named(&mut app, "Button");
    app.world_mut().trigger(Poke {
        entity: button,
        by: 2,
    });
    app.world_mut().trigger(Poke {
        entity: button,
        by: 3,
    });
    // Handlers run as commands, right after the observers that queue them.
    app.world_mut().flush();
    assert_eq!(app.world().get::<Count>(root), Some(&Count(5)));
    app.update();
    let label = app.world().get::<Children>(root).unwrap()[1];
    assert_eq!(label_of(&app, label), "5");
}

#[test]
fn nearest_works_in_plain_observers() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Count(0)
            Children [
                #Button
                on(|poke: On<Poke>, mut counts: Nearest<&mut Count>| {
                    if let Some(mut count) = counts.get_mut(poke.entity) {
                        count.0 += poke.by;
                    }
                })
            ]
        })
        .unwrap()
        .id();
    let button = named(&mut app, "Button");
    app.world_mut().trigger(Poke {
        entity: button,
        by: 4,
    });
    assert_eq!(app.world().get::<Count>(root), Some(&Count(4)));
}

// ---------------------------------------------------------------------------
// Outside scenes
// ---------------------------------------------------------------------------

#[test]
fn react_attaches_watches_and_handlers_without_scenes() {
    let mut app = app();
    app.add_systems(Startup, |mut commands: Commands| {
        commands
            .spawn((Count(1), Label::default()))
            .react(watch(|count: Ref<Count>| Label(format!("{}", count.0))))
            .react(handle(|poke: &Poke, count: &mut Count| count.0 += poke.by));
    });
    app.update();
    let entity = entities_with::<Count>(&mut app)[0];
    assert_eq!(label_of(&app, entity), "1");
    app.world_mut().trigger(Poke { entity, by: 9 });
    app.update();
    assert_eq!(label_of(&app, entity), "10");
}

#[test]
fn regions_spawn_from_commands() {
    let mut app = app();
    app.add_systems(Startup, |mut commands: Commands| {
        commands
            .spawn(Count(0))
            .queue_spawn_related_scenes::<Children>(when(
                |count: Ref<Count>| count.0 > 0,
                || bsn! { Label("shown") },
            ));
    });
    app.update();
    assert!(labels(&mut app).is_empty());
    let root = entities_with::<Count>(&mut app)[0];
    app.world_mut().get_mut::<Count>(root).unwrap().0 = 1;
    app.update();
    assert_eq!(labels(&mut app), ["shown"]);
}

#[test]
fn despawning_the_host_despawns_its_reactions() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            watch(|score: Res<Score>| Label(format!("{}", score.0)))
            watch(|_: Res<Score>| {})
        })
        .unwrap()
        .id();
    assert_eq!(entities_with::<ReactionOf>(&mut app).len(), 2);
    app.world_mut().despawn(entity);
    assert!(entities_with::<ReactionOf>(&mut app).is_empty());
}

#[test]
fn missing_plugin_is_an_error() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    let result = app.world_mut().spawn_scene(bsn! {
        watch(|_: Res<Runs>| {})
    });
    assert!(result.is_err());
}
