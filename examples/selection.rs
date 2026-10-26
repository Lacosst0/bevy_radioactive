//! Entity selection, a query monitor and inherited context, headless.
//!
//! Selecting a row moves the `Selected` marker, so only two rows recompute;
//! a monitor tracks which rows are `Ready`; rows read their accent from the
//! nearest ancestor's `Theme`.

use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{prelude::*, testing::settle};

#[derive(Component, Clone, Copy, Default, PartialEq)]
struct Theme(&'static str);

#[derive(Component, Default)]
struct Loaded;

#[derive(Component, Default)]
struct Failed;

#[derive(Component, Default)]
struct Ready;

#[derive(Component, Clone, Default, PartialEq)]
struct Status(String);

static RECOMPUTES: AtomicUsize = AtomicUsize::new(0);

fn row(name: &'static str) -> impl Scene {
    bsn! {
        computed(move |r: &Reads| {
            RECOMPUTES.fetch_add(1, Ordering::Relaxed);
            let accent = r.inherited::<Theme>().map_or("plain", |theme| theme.0);
            let mark = if r.this_has::<Selected>() { ">" } else { " " };
            let ready = if r.this_has::<Ready>() { "ready" } else { "waiting" };
            Status(format!("{mark} {name} ({accent}, {ready})"))
        })
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ))
    .add_monitor::<Ready>(Monitor::new().with::<Loaded>().without::<Failed>());

    let list = app
        .world_mut()
        .spawn_scene(bsn! {
            Theme("blue") Selection
            Children [ @row("alpha") -- @row("beta") -- @row("gamma") ]
        })
        .unwrap()
        .id();
    settle(&mut app);
    let rows = app.world().get::<Children>(list).unwrap().to_vec();
    print(&app, list, "spawned");

    app.world_mut()
        .commands()
        .entity(rows[0])
        .select::<Selected>();
    app.world_mut().entity_mut(rows[1]).insert(Loaded);
    settle(&mut app);
    print(&app, list, "selected alpha, beta loaded");

    let before = RECOMPUTES.load(Ordering::Relaxed);
    app.world_mut()
        .commands()
        .entity(rows[2])
        .select::<Selected>();
    settle(&mut app);
    print(&app, list, "selected gamma");
    println!(
        "  rows recomputed: {}",
        RECOMPUTES.load(Ordering::Relaxed) - before
    );

    app.world_mut().entity_mut(rows[1]).insert(Failed);
    app.world_mut().entity_mut(list).insert(Theme("red"));
    settle(&mut app);
    print(&app, list, "beta failed, theme red");
}

fn print(app: &App, list: Entity, title: &str) {
    println!("{title}:");
    for &row in app.world().get::<Children>(list).unwrap() {
        println!("  {}", app.world().get::<Status>(row).unwrap().0);
    }
}
