//! `view!` tabs: a reactive `match`, a patched component, a `memo`, and an
//! `effect`.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::view::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    Profile,
    Settings,
}

#[derive(Resource, Default)]
struct VolumeSetting(u32);

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ViewPlugin,
    ))
    .init_resource::<VolumeSetting>();

    let tab = signal(|| Tab::Profile);
    let volume = signal(|| 50_u32);
    let title = {
        let tab = tab.clone();
        memo(move || format!("Tab: {:?}", tab.get()))
    };

    let scene = {
        let (tab, volume, title) = (tab.clone(), volume.clone(), title.clone());
        view! {
            Node
            Children [
                // Patched in place whenever the memo changes.
                Text($(title.get()))
                --
                match $tab {
                    Tab::Profile => {
                        Text("Profile")
                    },
                    Tab::Settings => {
                        Text($(format!("Volume: {}%", $volume)))
                        --
                        if $volume > 80 { Text("Careful, that's loud") }
                        --
                        // Runs again whenever `volume` changes.
                        Node
                        effect(move |commands: &mut Commands| {
                            commands.insert_resource(VolumeSetting(volume.get()));
                        })
                    },
                }
            ]
        }
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
    println!(
        "saved volume: {}",
        app.world().resource::<VolumeSetting>().0
    );
}
