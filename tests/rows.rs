//! Integration coverage for `coat!` rows with item signals and `Value` props.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    coat::{Value, coat, memo, signal},
    testing::settle,
};

#[derive(Component, Clone, Default)]
struct Root;

#[derive(Component, Clone, Default)]
struct Label(u32);

#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    title: &'static str,
    done: bool,
}

fn todo(id: u32, title: &'static str, done: bool) -> Todo {
    Todo { id, title, done }
}

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

/// `(label, entity, text)` of every labelled row, in child order of the
/// region under `Root`.
fn rows(app: &mut App) -> Vec<(u32, Entity, String)> {
    let root = app
        .world_mut()
        .query_filtered::<Entity, With<Root>>()
        .single(app.world())
        .unwrap();
    let region = app.world().get::<Children>(root).unwrap()[0];
    let children = app
        .world()
        .get::<Children>(region)
        .map(|children| children.to_vec())
        .unwrap_or_default();
    children
        .into_iter()
        .map(|row| {
            let label = app.world().get::<Label>(row).unwrap().0;
            let text = app.world().get::<Text>(row).unwrap().0.clone();
            (label, row, text)
        })
        .collect()
}

#[test]
fn keyed_item_signals_patch_rows_in_place() {
    let todos = signal(|| vec![todo(1, "docs", false), todo(2, "ship", false)]);
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            #[key(todo.id)]
            for $todo in $todos {
                Label($(todo.get_untracked().id))
                Text($(format!("{} {}", $&todo.title, if $&todo.done { "[x]" } else { "[ ]" })))
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    settle(&mut app);
    let before = rows(&mut app);
    assert_eq!(before[0].2, "docs [ ]");
    assert_eq!(before[1].2, "ship [ ]");

    // A changed item keeps its row entity and patches the text.
    todos.update(|todos| todos[0].done = true);
    settle(&mut app);
    let after = rows(&mut app);
    assert_eq!(after[0].1, before[0].1, "row was respawned");
    assert_eq!(after[0].2, "docs [x]");
    assert_eq!(after[1].1, before[1].1);

    // Reordering moves rows, adding spawns one, removing despawns one.
    todos.set(vec![
        todo(3, "rest", false),
        todo(2, "ship", true),
        todo(1, "docs", true),
    ]);
    settle(&mut app);
    let moved = rows(&mut app);
    assert_eq!(moved.iter().map(|row| row.0).collect::<Vec<_>>(), [3, 2, 1]);
    assert_eq!(moved[1].1, before[1].1);
    assert_eq!(moved[1].2, "ship [x]");
    assert_eq!(moved[2].1, before[0].1);

    todos.set(vec![todo(2, "ship", true)]);
    settle(&mut app);
    let left = rows(&mut app);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].1, before[1].1);
    assert!(app.world().get_entity(before[0].1).is_err());
}

#[test]
fn indexed_item_signals_keep_slots() {
    let names = signal(|| vec!["a", "b"]);
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            for $name in $names {
                Label Text($(String::from($name)))
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    settle(&mut app);
    let before = rows(&mut app);
    assert_eq!(
        before.iter().map(|row| row.2.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );

    // Same slots, new items.
    names.set(vec!["b", "c", "d"]);
    settle(&mut app);
    let after = rows(&mut app);
    assert_eq!(after[0].1, before[0].1);
    assert_eq!(after[1].1, before[1].1);
    assert_eq!(
        after.iter().map(|row| row.2.as_str()).collect::<Vec<_>>(),
        ["b", "c", "d"]
    );

    names.set(vec!["z"]);
    settle(&mut app);
    let shrunk = rows(&mut app);
    assert_eq!(shrunk.len(), 1);
    assert_eq!(shrunk[0].1, before[0].1);
    assert_eq!(shrunk[0].2, "z");
}

#[test]
fn rows_despawn_with_their_region() {
    let todos = signal(|| vec![todo(1, "docs", false)]);
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            #[key(todo.id)]
            for $todo in $todos { Label Text($(String::from($&todo.title))) }
        ]
    };
    let root = app.world_mut().spawn_scene(scene).unwrap().id();
    settle(&mut app);
    app.world_mut().despawn(root);
    settle(&mut app);
    todos.update(|todos| todos[0].title = "changed");
    settle(&mut app);
    let left = app.world_mut().query::<&Label>().iter(app.world()).count();
    assert_eq!(left, 0);
}

fn caption(text: impl Into<Value<String>>) -> impl Scene {
    let text: Value<String> = text.into();
    coat! { Label Text($text) }
}

#[test]
fn value_props_accept_constants_signals_and_memos() {
    let name = signal(|| "Ada".to_string());
    let source = name.clone();
    let shout = memo(move || source.get().to_uppercase());
    let mut app = app();
    let scene = bsn! {
        Root
        Children [
            Node Children [ @caption("fixed") -- @caption(name.clone()) -- @caption(shout.clone()) ]
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    settle(&mut app);
    let texts = |app: &mut App| rows(app).into_iter().map(|row| row.2).collect::<Vec<_>>();
    assert_eq!(texts(&mut app), ["fixed", "Ada", "ADA"]);

    name.set("Grace".into());
    settle(&mut app);
    assert_eq!(texts(&mut app), ["fixed", "Grace", "GRACE"]);
}
