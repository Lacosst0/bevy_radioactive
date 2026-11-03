#![cfg(feature = "reactive")]
//! `rsn!`: `bsn!` with Rust control flow and `$Type` reads.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Count(i32);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Score(u32);

#[derive(Component, Clone, Default, Debug, PartialEq, Eq)]
struct Label(String);

#[derive(Component, Clone, Default, Debug, PartialEq)]
struct Style {
    width: f32,
    title: String,
}

#[derive(Component, Clone, Copy, Default)]
struct Row;

#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    title: String,
}

#[derive(Resource, Clone, Default)]
struct Todos(Vec<Todo>);

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Tab {
    #[default]
    Home,
    Settings,
    Profile(u32),
}

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
    .init_resource::<Todos>()
    .init_resource::<Tab>();
    app
}

/// Labels in tree order.
fn labels(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, Without<ChildOf>>()
        .iter(world)
        .collect();
    let mut out = Vec::new();
    for root in roots {
        walk(world, root, &mut out);
    }
    out
}

fn walk(world: &World, entity: Entity, out: &mut Vec<String>) {
    if let Some(label) = world.get::<Label>(entity) {
        out.push(label.0.clone());
    }
    if let Some(children) = world.get::<Children>(entity) {
        for child in children.iter() {
            walk(world, child, out);
        }
    }
}

fn entities_with<T: Component>(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<T>>();
    query.iter(world).collect()
}

fn root(app: &mut App) -> Entity {
    entities_with::<Count>(app)[0]
}

fn set_count(app: &mut App, value: i32) {
    let root = root(app);
    app.world_mut().get_mut::<Count>(root).unwrap().0 = value;
    app.update();
}

#[test]
fn dollar_in_a_component_value_watches_it() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(1)
            Children [
                Label({ format!("count {}", $Count.0) })
                --
                Label({ format!("score {}", $Score.0) })
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["count 1", "score 0"]);

    set_count(&mut app, 2);
    app.world_mut().resource_mut::<Score>().0 = 9;
    app.update();
    assert_eq!(labels(&mut app), ["count 2", "score 9"]);
}

#[test]
fn braced_components_patch_only_reactive_fields() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(3)
            Children [ Style { width: $Count.0 as f32 * 2.0, title: "fixed" } ]
        })
        .unwrap();
    let style = entities_with::<Style>(&mut app)[0];
    assert_eq!(
        app.world().get::<Style>(style),
        Some(&Style {
            width: 6.0,
            title: "fixed".into()
        })
    );
    set_count(&mut app, 5);
    assert_eq!(
        app.world().get::<Style>(style),
        Some(&Style {
            width: 10.0,
            title: "fixed".into()
        })
    );
}

#[test]
fn if_else_if_else_switches_branches() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(0)
            Children [
                Label("before")
                --
                if $Count.0 > 0 {
                    Label("positive")
                } else if $Count.0 < 0 {
                    Label("negative") -- Label("really")
                } else {
                    Label("zero")
                }
                --
                if $Count.0 > 100 { Label("huge") }
                --
                Label("after")
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["before", "zero", "after"]);

    set_count(&mut app, 3);
    assert_eq!(labels(&mut app), ["before", "positive", "after"]);
    set_count(&mut app, -1);
    assert_eq!(labels(&mut app), ["before", "negative", "really", "after"]);
    set_count(&mut app, 101);
    assert_eq!(labels(&mut app), ["before", "positive", "huge", "after"]);
}

#[test]
fn match_on_a_resource_with_bindings() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(0)
            Children [
                match *$Tab {
                    Tab::Home => Label("home"),
                    Tab::Settings => { Label("settings") -- Label("more") }
                    Tab::Profile(id) => Label({ format!("profile {id}") }) Row,
                }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["home"]);

    *app.world_mut().resource_mut::<Tab>() = Tab::Settings;
    app.update();
    assert_eq!(labels(&mut app), ["settings", "more"]);

    *app.world_mut().resource_mut::<Tab>() = Tab::Profile(7);
    app.update();
    assert_eq!(labels(&mut app), ["profile 7"]);
}

fn todo(id: u32, title: &str) -> Todo {
    Todo {
        id,
        title: title.into(),
    }
}

#[test]
fn keyed_for_keeps_rows_and_uses_the_item_more_than_once() {
    let mut app = app();
    app.world_mut().resource_mut::<Todos>().0 = vec![todo(1, "a"), todo(2, "b")];
    app.world_mut()
        .spawn_scene(rsn! {
            Count(0)
            Children [
                #[key(todo.id)]
                for todo in $Todos.0.clone() {
                    Row
                    Label({ todo.title.clone() })
                    Children [
                        Label({ format!("#{} {}", todo.id, todo.title) })
                        --
                        if i64::from($Count.0) > i64::from(todo.id) { Label({ format!("done {}", todo.title) }) }
                    ]
                }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["a", "#1 a", "b", "#2 b"]);
    let rows = entities_with::<Row>(&mut app);

    app.world_mut().resource_mut::<Todos>().0 = vec![todo(2, "b"), todo(1, "a"), todo(3, "c")];
    app.update();
    assert_eq!(labels(&mut app), ["b", "#2 b", "a", "#1 a", "c", "#3 c"]);
    let after = entities_with::<Row>(&mut app);
    assert!(rows.iter().all(|row| after.contains(row)), "rows kept");

    set_count(&mut app, 2);
    assert_eq!(
        labels(&mut app),
        ["b", "#2 b", "a", "#1 a", "done a", "c", "#3 c"]
    );
}

#[test]
fn unkeyed_and_plain_for_loops() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(2)
            Children [
                for i in 0..$Count.0 { Label({ format!("row {i}") }) }
                --
                for name in ["x", "y"] { Label({ name.to_string() }) Row }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["row 0", "row 1", "x", "y"]);
    set_count(&mut app, 3);
    assert_eq!(labels(&mut app), ["row 0", "row 1", "row 2", "x", "y"]);
}

#[test]
fn on_writes_the_nearest_component() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(0)
            Children [
                #Button
                on(|poke: On<Poke>| $Count.0 += poke.by)
                --
                Label({ $Count.0.to_string() })
            ]
        })
        .unwrap();
    let button = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Name)>();
        query
            .iter(world)
            .find(|(_, name)| name.as_str() == "Button")
            .unwrap()
            .0
    };
    app.world_mut().trigger(Poke {
        entity: button,
        by: 2,
    });
    app.world_mut().trigger(Poke {
        entity: button,
        by: 3,
    });
    app.update();
    let root = root(&mut app);
    assert_eq!(app.world().get::<Count>(root), Some(&Count(5)));
    assert_eq!(labels(&mut app), ["5"]);
}

#[test]
fn rsn_list_and_plain_bsn_items_mix() {
    let mut app = app();
    let root = app.world_mut().spawn(Count(1)).id();
    app.world_mut()
        .entity_mut(root)
        .queue_spawn_related_scenes::<Children>(rsn_list! {
            Label("plain")
            --
            if $Count.0 == 1 { Label("one") } else { Label("other") }
        });
    app.update();
    assert_eq!(labels(&mut app), ["plain", "one"]);
    set_count(&mut app, 2);
    assert_eq!(labels(&mut app), ["plain", "other"]);
}

fn badge(text: &'static str) -> impl Scene {
    rsn! { Label({ format!("{text} {}", $Count.0) }) }
}

#[test]
fn scene_functions_in_arms_and_nested_rsn_in_rows() {
    let mut app = app();
    let target = app.world_mut().spawn(Count(42)).id();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(1)
            Children [
                match *$Tab {
                    Tab::Home => @badge("home"),
                    _ => Label("other"),
                }
                --
                @each(move || vec![target], |&e| e, |&e| rsn! {
                    Source({e})
                    Label({ format!("source {}", $Count.0) })
                })
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["home 1", "source 42"]);
    app.world_mut().get_mut::<Count>(target).unwrap().0 = 43;
    app.update();
    assert_eq!(labels(&mut app), ["home 1", "source 43"]);
}

#[derive(Component, Clone, Debug, PartialEq)]
struct Task {
    title: String,
    done: bool,
}

#[derive(Component, Clone, Copy, Default)]
struct Player;

#[test]
fn for_over_a_query_tracks_membership_and_values() {
    let mut app = app();
    let first = app
        .world_mut()
        .spawn(Task {
            title: "a".into(),
            done: false,
        })
        .id();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(0)
            Children [
                for task in $Query<&Task> {
                    Row
                    Label({ format!("{} {}", task.title, if task.done { "x" } else { "-" }) })
                }
                --
                Label({ format!("{} open", $Query<&Task>.iter().filter(|t| !t.done).count()) })
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["a -", "1 open"]);

    app.world_mut().spawn(Task {
        title: "b".into(),
        done: false,
    });
    app.update();
    assert_eq!(labels(&mut app), ["a -", "b -", "2 open"]);

    app.world_mut().get_mut::<Task>(first).unwrap().done = true;
    app.update();
    assert_eq!(labels(&mut app), ["a x", "b -", "1 open"]);

    app.world_mut().entity_mut(first).despawn();
    app.update();
    assert_eq!(labels(&mut app), ["b -", "1 open"]);
}

#[test]
fn query_rows_read_their_entity_with_dollar() {
    let mut app = app();
    let task = app
        .world_mut()
        .spawn(Task {
            title: "a".into(),
            done: false,
        })
        .id();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(0)
            Children [
                // Entity rows: the row is never rebuilt, `$Task` reads its entity.
                for _entity in $Query<Entity, With<Task>> {
                    Label({ $Task.title.clone() })
                }
            ]
        })
        .unwrap();
    let rows = entities_with::<Label>(&mut app);
    app.world_mut().get_mut::<Task>(task).unwrap().title = "b".into();
    app.update();
    assert_eq!(labels(&mut app), ["b"]);
    assert_eq!(entities_with::<Label>(&mut app), rows, "updated in place");
}

#[test]
fn explicit_sources_entity_and_resource() {
    let mut app = app();
    let player = app.world_mut().spawn((Player, Count(10))).id();
    app.world_mut()
        .spawn_scene(rsn! {
            Count(1)
            Children [
                Label({ format!("{} {} {}", $Count.0, $(player).Count.0, $Res<Score>.0) })
                --
                Label({ format!("single {}", $Single<&Count, With<Player>>.0) })
                --
                #Button
                on(|poke: On<Poke>| $(player).Count.0 += poke.by)
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["1 10 0", "single 10"]);

    let button = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Name)>();
        query
            .iter(world)
            .find(|(_, name)| name.as_str() == "Button")
            .unwrap()
            .0
    };
    app.world_mut().trigger(Poke {
        entity: button,
        by: 5,
    });
    app.world_mut().resource_mut::<Score>().0 = 3;
    app.update();
    assert_eq!(labels(&mut app), ["1 15 3", "single 15"]);
}
