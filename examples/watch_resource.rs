use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Copy, PartialEq, Track)]
struct Counter {
    value: u32,
}

#[derive(Component, Clone, Default)]
struct Shown(u32);

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ))
    .insert_resource(Counter { value: 1 });

    app.world_mut()
        .spawn_scene(bsn! {
            Name::new("counter-listener")
            watch(|counter: Watch<Res<Counter>>| {
                println!("counter changed to {}", counter.value);
            })
            // Written on spawn, then on every change.
            Shown(0)
            bind(|counter: &Counter, shown: &mut Shown| shown.0 = counter.value)
        })
        .unwrap();

    app.update();

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter { value: 2 });
    app.update();
}
