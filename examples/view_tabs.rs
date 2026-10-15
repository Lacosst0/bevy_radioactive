//! `view!` tabs: a reactive `match` on a resource, local state, a patched
//! component, and an `effect`.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::view::*;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum Tab {
    #[default]
    Profile,
    Settings,
}

#[derive(Resource, Clone, Copy, Default)]
struct CurrentTab(Tab);

#[derive(Resource, Clone, Copy)]
struct Volume(u32);

/// `tab` and `volume` follow resources; `visits` is state of this instance.
#[component]
fn tabs(
    tab: Watch<Res<CurrentTab>>,
    volume: Watch<Res<Volume>>,
    visits: Watch<Local<u32>>,
) -> impl Scene {
    view! {
        Node
        // Runs again whenever `tab` changes, counting tab switches.
        effect(move |_: &mut Commands| {
            let _ = tab.get();
            visits.update(|visits| *visits += 1);
        })
        Children [
            // Patched in place.
            Text($(format!("Tab: {:?} (visit {})", $tab.0, $visits)))
            --
            match $tab.0 {
                Tab::Profile => {
                    Text("Profile")
                },
                Tab::Settings => {
                    Text($(format!("Volume: {}%", $volume.0)))
                    --
                    if $volume.0 > 80 { Text("Careful, that's loud") }
                },
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
    .init_resource::<CurrentTab>()
    .insert_resource(Volume(50));

    app.world_mut().spawn_scene(bsn! { @tabs() }).unwrap();

    let print = |app: &mut App, label: &str| {
        println!("-- {label}");
        let mut texts = app.world_mut().query::<&Text>();
        for text in texts.iter(app.world()) {
            println!("{}", text.0);
        }
    };

    app.update();
    print(&mut app, "profile");

    app.world_mut().resource_mut::<CurrentTab>().0 = Tab::Settings;
    app.update();
    print(&mut app, "settings");

    app.world_mut().resource_mut::<Volume>().0 = 90;
    app.update();
    print(&mut app, "louder");
}
