//! Routes a slider event to a subscribed listener without changing its source.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin, ui_widgets::ValueChange};
use bevy_radioactive::subscribe::*;

#[derive(Component, Clone, Copy, Default)]
struct Slider;

#[derive(Resource, Default)]
struct Received(Option<(Entity, Entity, f32)>);

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin))
        .init_resource::<Received>();

    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Name::new("subscription-root")
            Children[
                #Slider Slider
                --
                Name::new("subscribed label")
                subscribe_to(#Slider, |event: Subscription<ValueChange<f32>>, mut received: ResMut<Received>| {
                    received.0 = Some((event.target(), event.source(), event.value));
                })
            ]
        })
        .unwrap()
        .id();
    app.world_mut().flush();
    app.update();
    app.world_mut().flush();

    let slider = app
        .world_mut()
        .query_filtered::<Entity, With<Slider>>()
        .iter(app.world())
        .next()
        .expect("slider exists");
    app.world_mut().trigger(ValueChange {
        source: slider,
        value: 42.0_f32,
        is_final: true,
    });
    app.world_mut().flush();

    let (listener, source, value) = app
        .world()
        .resource::<Received>()
        .0
        .expect("subscription delivery was flushed");
    println!("listener {listener:?} received {value} from {source:?}");

    app.world_mut().despawn(root);
}
