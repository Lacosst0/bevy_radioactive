//! A keyed list of todo entities, a filter switch, and an empty state.
//!
//! Each todo is an entity. `for _ in $Query<..>` keeps one row per todo
//! entity, and `$(todo).Todo` inside a row reads that entity, so editing a todo
//! patches its row instead of rebuilding it.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Default)]
struct Todo {
    title: String,
    done: bool,
}

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Open,
}

#[derive(Component, Clone, Default)]
struct Label(String);

fn todo_list() -> impl Scene {
    rsn! {
        Children [
            match *$Res<Filter> {
                Filter::All => Label("Showing all"),
                Filter::Open => Label("Showing open"),
            }
            --
            if $Query<(), With<Todo>>.is_empty() { Label("Nothing to do") }
            --
            // One row per todo entity, kept while it matches. `$(todo).Todo` in a row
            // reads that row's entity, so edits update the row in place.
            for todo in $Query<Entity, With<Todo>> {
                if *$Res<Filter> == Filter::All || !$(todo).Todo.done {
                    Label({ format!("[{}] {}", if $(todo).Todo.done { "x" } else { " " }, $(todo).Todo.title) })
                }
            }
        ]
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Filter>();
    app.world_mut().spawn_scene(todo_list()).unwrap();
    print(&mut app, "empty");

    let docs = app
        .world_mut()
        .spawn(Todo {
            title: "Write docs".into(),
            done: false,
        })
        .id();
    app.world_mut().spawn(Todo {
        title: "Ship it".into(),
        done: false,
    });
    app.update();
    print(&mut app, "two todos");

    app.world_mut().get_mut::<Todo>(docs).unwrap().done = true;
    app.update();
    print(&mut app, "docs done");

    *app.world_mut().resource_mut::<Filter>() = Filter::Open;
    app.update();
    print(&mut app, "open only");
}

fn print(app: &mut App, title: &str) {
    println!("-- {title}");
    let mut labels = app.world_mut().query::<&Label>();
    let mut lines: Vec<String> = labels.iter(app.world()).map(|l| l.0.clone()).collect();
    lines.sort();
    for line in lines {
        println!("{line}");
    }
}
