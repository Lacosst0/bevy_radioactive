use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Copy, PartialEq)]
struct Counter {
    value: u32,
}

#[derive(Resource, Clone, Copy, Default)]
struct Step(u32);

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
    .insert_resource(Counter { value: 1 })
    .insert_resource(Step(10));

    app.world_mut()
        .spawn_scene(bsn! {
            Name::new("counter-listener")
            // Re-runs when `Counter` changes; `Step` is read but not tracked.
            watch(|counter: Ref<Counter>, step: Res<Step>| {
                println!("counter changed to {} (step {})", counter.value, step.0);
            })
            // Runs on spawn, then on every change, and inserts the returned component.
            watch(|counter: Ref<Counter>| Shown(counter.value))
            // Components of this entity are inputs too, so this runs after the one above.
            watch(|shown: Ref<Shown>| println!("shown: {}", shown.0))
        })
        .unwrap();

    app.update();

    app.world_mut()
        .resource_mut::<Counter>()
        .set_if_neq(Counter { value: 2 });
    app.update();
}
