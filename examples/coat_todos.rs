//! A `coat!` template with reactive `if`, keyed `for`, and patched text.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    coat::{coat, signal},
};

#[derive(Clone, PartialEq)]
struct Todo {
    id: u32,
    title: &'static str,
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ));

    let todos = signal(|| {
        vec![Todo {
            id: 1,
            title: "Write docs",
        }]
    });
    let scene = coat! {
        Node
        Children [
            Text($(format!("{} todos", $&todos.len())))
            --
            if $&todos.is_empty() {
                Text("Nothing to do")
            }
            --
            #[key(todo.id)]
            for todo in $todos {
                Text({todo.title})
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();

    app.update();
    todos.update(|todos| {
        todos.push(Todo {
            id: 2,
            title: "Ship it",
        });
    });
    app.update();

    let mut texts = app.world_mut().query::<&Text>();
    for text in texts.iter(app.world()) {
        println!("{}", text.0);
    }
}
