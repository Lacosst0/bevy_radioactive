use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{Reaction, ReactiveAppExt, watch_component};

#[derive(Component, Clone, Copy, Default)]
struct Meter(u32);

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
        .track_component::<Meter>();

    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                (#Meter Meter(1)),
                (
                    Name::new("meter-listener")
                    watch_component(#Meter, |event: Reaction<Meter>| {
                        println!(
                            "listener {:?} saw source {:?} change to {}",
                            event.target(),
                            event.source(),
                            event.0,
                        );
                    })
                )
            ]
        })
        .unwrap();

    app.update();
}
