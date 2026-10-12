//! React-style `hooks` todo list: a resource drives keyed rows, each row keeps
//! its own `use_state`, and a theme flows down through context.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::hooks::*;

#[derive(Clone, Copy, PartialEq)]
struct Todo {
    id: u32,
    title: &'static str,
}

#[derive(Resource, Clone, Default)]
struct Todos(Vec<Todo>);

#[derive(Resource, Clone, Copy, PartialEq, Debug)]
enum Theme {
    Light,
    Dark,
}

#[component]
fn todo_app(cx: &Cx) -> impl Scene {
    // Like `<ThemeContext.Provider value={theme}>`. Re-renders when `Theme`
    // changes, and the rows render again with the new context.
    let theme = cx.use_resource::<Theme>().unwrap_or(Theme::Light);
    cx.provide_context(theme);

    // Re-renders when the `Todos` resource changes.
    let todos = cx.use_resource::<Todos>().unwrap_or_default().0;
    cx.use_effect(todos.len(), |count, _commands| {
        println!("[effect] {count} todos");
    });

    // Like `todos.map(todo => <TodoRow key={todo.id} todo={todo} />)`.
    let rows = todos
        .into_iter()
        .map(|todo| todo_row(todo).key(todo.id))
        .collect::<Vec<_>>();

    bsn! {
        Node
        Children [
            Text("Todos")
            --
            Node Children [ {rows} ]
        ]
    }
}

#[component]
fn todo_row(cx: &Cx, todo: Todo) -> impl Scene {
    // Each keyed row keeps its own state across parent re-renders.
    let (renders, set_renders) = cx.use_state(|| 0_u32);
    cx.use_effect((), move |(), _commands| {
        set_renders(renders + 1);
    });

    let theme = cx.use_context::<Theme>().unwrap_or(Theme::Light);
    bsn! { Text({format!("[{theme:?}] #{} {} (mounted {renders}x)", todo.id, todo.title)}) }
}

fn print_texts(app: &mut App, label: &str) {
    println!("-- {label}");
    let mut texts = app.world_mut().query::<&Text>();
    for text in texts.iter(app.world()) {
        println!("{}", text.0);
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        HooksPlugin,
    ))
    .insert_resource(Theme::Light)
    .insert_resource(Todos(vec![Todo {
        id: 1,
        title: "Write docs",
    }]));

    app.world_mut()
        .spawn_scene(bsn! { Node @todo_app() })
        .unwrap();
    app.update();
    print_texts(&mut app, "initial");

    app.world_mut().resource_mut::<Todos>().0.insert(
        0,
        Todo {
            id: 2,
            title: "Ship it",
        },
    );
    app.update();
    print_texts(&mut app, "after adding a todo");

    *app.world_mut().resource_mut::<Theme>() = Theme::Dark;
    app.update();
    print_texts(&mut app, "after switching theme");
}
