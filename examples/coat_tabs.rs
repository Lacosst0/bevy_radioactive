//! `coat!` tabs: a reactive `match`, a patched component, a `memo`, and a
//! `shard` that queries the world when its argument changes.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    coat::{coat, memo, shard, signal},
};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    Profile,
    Settings,
}

#[derive(Component, Clone, Default)]
struct Score(u32);

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ));
    app.world_mut().spawn(Score(10));
    app.world_mut().spawn(Score(32));

    let tab = signal(|| Tab::Profile);
    let volume = signal(|| 50_u32);
    let title = {
        let tab = tab.clone();
        memo(move || format!("Tab: {:?}", tab.get()))
    };

    let scene = coat! {
        Node
        Children [
            // Patched in place whenever the memo changes.
            Text($(title.get()))
            --
            match $tab {
                Tab::Profile => {
                    Text("Profile")
                    --
                    // Like a Topcoat `#[shard]`: renders from the world, and again
                    // whenever its arguments (none here) change.
                    Node
                    shard(move || (), |(), world: &World| {
                        let total: u32 = world
                            .try_query::<&Score>()
                            .map(|mut q| q.iter(world).map(|s| s.0).sum())
                            .unwrap_or_default();
                        bsn! { Text({format!("Total score: {total}")}) }
                    })
                },
                Tab::Settings => {
                    Text($(format!("Volume: {}%", $volume)))
                    --
                    if $volume > 80 { Text("Careful, that's loud") }
                },
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();

    let print = |app: &mut App, label: &str| {
        println!("-- {label}");
        let mut texts = app.world_mut().query::<&Text>();
        for text in texts.iter(app.world()) {
            println!("{}", text.0);
        }
    };

    app.update();
    print(&mut app, "profile");

    tab.set(Tab::Settings);
    app.update();
    print(&mut app, "settings");

    volume.set(90);
    app.update();
    print(&mut app, "louder");
}
