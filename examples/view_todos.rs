//! A `view!` component with reactive `if`, keyed `for`, and patched text,
//! driven by a resource.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::view::*;

#[derive(Clone, PartialEq)]
struct Todo {
    id: u32,
    title: &'static str,
}

#[derive(Resource, Clone, Default)]
struct Todos(Vec<Todo>);

/// `Watch<Res<Todos>>`: the list follows the `Todos` resource.
#[component]
fn todo_list(todos: Watch<Res<Todos>>) -> impl Scene {
    view! {
        Node
        Children [
            Text($(format!("{} todos", $todos.0.len())))
            --
            if $todos.0.is_empty() {
                Text("Nothing to do")
            }
            --
            #[key(todo.id)]
            for todo in $todos.0 {
                Text({todo.title})
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
        ViewPlugin,
    ))
    .insert_resource(Todos(vec![Todo {
        id: 1,
        title: "Write docs",
    }]));

    app.world_mut().spawn_scene(bsn! { @todo_list() }).unwrap();
    app.update();

    app.world_mut().resource_mut::<Todos>().0.push(Todo {
        id: 2,
        title: "Ship it",
    });
    app.update();

    let mut texts = app.world_mut().query::<&Text>();
    for text in texts.iter(app.world()) {
        println!("{}", text.0);
    }
}
