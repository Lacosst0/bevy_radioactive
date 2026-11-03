//! A keyed list of todo entities, a filter switch, and an empty state.
//!
//! Each todo is an entity. The list is keyed by entity, and each row reads
//! its todo through `Source`, so editing a todo patches its row instead of
//! rebuilding it.

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
    bsn! {
        Children [
            {switch(|filter: Res<Filter>| *filter, |filter| match filter {
                Filter::All => bsn! { Label("Showing all") }.into(),
                Filter::Open => bsn! { Label("Showing open") }.into(),
            })}
            --
            {when(|todos: Query<(), With<Todo>>| todos.is_empty(), || bsn! { Label("Nothing to do") })}
            --
            {each(
                |todos: Query<(Entity, &Todo)>, filter: Res<Filter>| {
                    let mut rows: Vec<Entity> = todos
                        .iter()
                        .filter(|(_, todo)| *filter == Filter::All || !todo.done)
                        .map(|(entity, _)| entity)
                        .collect();
                    rows.sort();
                    rows
                },
                |&todo| bsn! {
                    Source({todo})
                    Label
                    watch(|todo: Ref<Todo>| {
                        Label(format!("[{}] {}", if todo.done { "x" } else { " " }, todo.title))
                    })
                },
            )
            .key(|&todo| todo)}
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
