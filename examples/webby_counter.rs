//! A `webby` counter: the label re-renders whenever the resource it reads changes.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{RadioactivePlugin, webby::text};

#[derive(Resource, Default)]
struct Clicks(u32);

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ))
    .init_resource::<Clicks>();

    let label = app
        .world_mut()
        .spawn_scene(bsn! {
            Text("")
            text((), |cx, ()| format!("clicked {} times", cx.resource::<Clicks>().map_or(0, |c| c.0)))
        })
        .unwrap()
        .id();

    for _ in 0..3 {
        app.update();
        println!("{}", app.world().get::<Text>(label).unwrap().0);
        app.world_mut().resource_mut::<Clicks>().0 += 1;
    }
}
