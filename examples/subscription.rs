//! Routes a slider event to a subscribed listener without changing its source.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin, ui_widgets::ValueChange};
use bevy_radioactive::subscription::{Subscription, subscribe_to};

#[derive(Component, Clone, Copy, Default)]
struct Slider;

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));

    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Children[
                (#Slider Slider),
                (
                    Name::new("subscribed label")
                    subscribe_to(#Slider, |event: Subscription<ValueChange<f32>>| {
                        println!(
                            "listener {:?} received {} from {:?}",
                            event.target(),
                            event.value,
                            event.source(),
                        );
                    })
                )
            ]
        })
        .unwrap()
        .id();
    app.update();

    let slider = app
        .world_mut()
        .query_filtered::<Entity, With<Slider>>()
        .iter(app.world())
        .next()
        .expect("slider exists");

    app.world_mut().trigger(ValueChange {
        source: slider,
        value: 42.0,
        is_final: true,
    });
    app.update();

    app.world_mut().despawn(root);
}
