use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{Reaction, ReactiveAppExt, watch_resource};

#[derive(Resource, Clone, Copy, PartialEq)]
struct Counter {
    value: u32,
}

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
        .insert_resource(Counter { value: 1 })
        .track_resource::<Counter>();

    app.world_mut()
        .spawn_scene(bsn! {
            Name::new("counter-listener")
            watch_resource(|event: Reaction<Counter>| {
                println!("counter changed to {}", event.value);
            })
        })
        .unwrap();

    app.update();

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter { value: 2 });
    app.update();
}
