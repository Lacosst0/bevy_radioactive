use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Copy, PartialEq)]
struct Counter {
    value: u32,
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ))
    .insert_resource(Counter { value: 1 });

    // `Res<Counter>` makes the watch re-run whenever `Counter` changes.
    app.world_mut()
        .spawn_scene(bsn! {
            Name::new("counter-listener")
            watch(|counter: Res<Counter>, this: This<&Name>| {
                if let Some(name) = this.get() {
                    println!("{name} sees counter {}", counter.value);
                }
            })
        })
        .unwrap();

    app.update();

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter { value: 2 });
    app.update();
}
