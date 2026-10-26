//! A React-style `webby` counter, mounted with `@counter(..)`.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    webby::{Cx, Dispatch, component},
};

#[derive(Resource, Default)]
struct Increment(Option<Dispatch<u32>>);

#[component]
fn counter(cx: &Cx, label: &'static str) -> impl Scene {
    let (count, add) = cx.use_reducer(|count, delta: u32| count + delta, || 0);
    let doubled = cx.use_memo(count, |count| count * 2);
    cx.use_effect((), move |(), commands| {
        commands.insert_resource(Increment(Some(add.clone())));
    });
    bsn! { Text({format!("{label}: {count} (x2 = {doubled})")}) }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ))
    .init_resource::<Increment>();

    let host = app
        .world_mut()
        .spawn_scene(bsn! { Node @counter("clicks") })
        .unwrap()
        .id();

    for _ in 0..3 {
        app.update();
        let child = app.world().get::<Children>(host).unwrap()[0];
        println!("{}", app.world().get::<Text>(child).unwrap().0);
        if let Some(increment) = &app.world().resource::<Increment>().0 {
            increment(1);
        }
    }
}
