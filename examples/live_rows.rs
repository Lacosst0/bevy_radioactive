//! `coat!` rows that patch in place, a `Value` prop and a `use_task` hook,
//! headless.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    prelude::*,
    testing::settle_n,
    webby::{TaskState, component},
};

#[derive(Clone, PartialEq)]
struct Todo {
    id: u32,
    title: &'static str,
    done: bool,
}

/// A caption that accepts a constant, a signal or a memo.
fn caption(text: impl Into<Value<String>>) -> impl Scene {
    let text: Value<String> = text.into();
    coat! { Text($text) }
}

/// Shows "loading" until a background computation finishes.
#[component]
fn total(cx: &Cx, count: usize) -> impl Scene {
    let label = match cx.use_task(count, |count| async move { (1..=count).sum::<usize>() }) {
        TaskState::Pending => "summing...".to_string(),
        TaskState::Ready(sum) => format!("sum of 1..={count} is {sum}"),
    };
    bsn! { Text({label}) }
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
        vec![
            Todo {
                id: 1,
                title: "Write docs",
                done: false,
            },
            Todo {
                id: 2,
                title: "Ship it",
                done: false,
            },
        ]
    });
    let title = signal(|| "Todos".to_string());
    let scene = coat! {
        Node
        Children [
            @caption(title.clone())
            --
            @caption("(rows patch in place)")
            --
            @total(100)
            --
            // each row receives its todo as a signal; changing one todo
            // patches its text instead of respawning the row
            #[key(todo.id)]
            for $todo in $todos {
                Text($(format!("[{}] {}", if $&todo.done { "x" } else { " " }, $&todo.title)))
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    settle_n(&mut app, 2);
    print(&mut app, "spawned");

    let first_row = row_entity(&mut app, "Write docs");
    todos.update(|todos| todos[0].done = true);
    title.set("Todos (1 done)".into());
    settle_n(&mut app, 2);
    print(&mut app, "after finishing one todo");
    println!(
        "  first row kept its entity: {}",
        row_entity(&mut app, "Write docs") == first_row
    );
}

fn row_entity(app: &mut App, title: &str) -> Option<Entity> {
    app.world_mut()
        .query::<(Entity, &Text)>()
        .iter(app.world())
        .find(|(_, text)| text.0.ends_with(title))
        .map(|(entity, _)| entity)
}

fn print(app: &mut App, heading: &str) {
    println!("{heading}:");
    let mut texts: Vec<String> = app
        .world_mut()
        .query::<&Text>()
        .iter(app.world())
        .map(|text| text.0.clone())
        .collect();
    texts.sort();
    for text in texts {
        println!("  {text}");
    }
}
