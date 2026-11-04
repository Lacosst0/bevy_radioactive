//! Enter and exit transitions on a toggled panel and a keyed list.
//!
//! `Transition` on the root of an `if` arm or `for` row keeps it around while
//! it animates out, and `$Ref<Presence>(#Name)` reads how present it is.
//! This example runs headless and prints each frame; in a UI, write the value
//! into `BackgroundColor`, `Node` offsets or `Transform` instead of `Alpha`.

use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin, time::TimeUpdateStrategy};
use bevy_radioactive::reactive::*;

#[derive(Resource, Clone, Copy, Default)]
struct Menu {
    open: bool,
}

#[derive(Resource, Clone, Default)]
struct Toasts(Vec<&'static str>);

#[derive(Component, Clone, Default)]
struct Label(String);

/// Stands in for an opacity.
#[derive(Component, Clone, Copy, Default)]
struct Alpha(f32);

fn screen() -> impl Scene {
    rsn! {
        Children [
            if $Res<Menu>.open {
                #Menu
                Label("menu")
                Transition::new(0.3).ease(EaseFunction::CubicOut)
                Alpha($Ref<Presence>(#Menu).value())
            }
            --
            #[key(*toast)]
            for toast in $Res<Toasts>.0.clone() {
                #Toast
                Label({ toast.to_string() })
                Transition::new(0.2).exit(0.4)
                Alpha($Ref<Presence>(#Toast).value())
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
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        100,
    )))
    .init_resource::<Menu>()
    .insert_resource(Toasts(vec!["saved"]));
    app.world_mut().spawn_scene(screen()).unwrap();
    app.update();
    print(&mut app, "start: the first toast shows at once");

    app.world_mut().resource_mut::<Menu>().open = true;
    for frame in 0..4 {
        app.update();
        print(&mut app, &format!("menu opening, frame {frame}"));
    }

    app.world_mut().resource_mut::<Menu>().open = false;
    app.world_mut().resource_mut::<Toasts>().0 = vec!["synced"];
    for frame in 0..6 {
        app.update();
        print(
            &mut app,
            &format!("menu closing, toast swapped, frame {frame}"),
        );
    }
}

fn print(app: &mut App, title: &str) {
    println!("-- {title}");
    let world = app.world_mut();
    let mut rows: Vec<String> = world
        .query::<(&Label, &Alpha, Has<Leaving>)>()
        .iter(world)
        .map(|(label, alpha, leaving)| {
            let state = if leaving { " (leaving)" } else { "" };
            format!("{:>7} {:.2}{state}", label.0, alpha.0)
        })
        .collect();
    rows.sort();
    for row in rows {
        println!("{row}");
    }
}
