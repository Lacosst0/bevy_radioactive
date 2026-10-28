use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Component, Clone, Copy, Track)]
struct Health(u32);

#[derive(Component, Clone, Copy)]
struct Enemy;

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ));

    app.world_mut()
        .spawn_scene(bsn! {
            Name::new("health-log")
            watch(|hp: Watch<Query<&Health>>| {
                println!("{:?} has {} HP", hp.entity(), hp.0);
            })
            watch(|hp: Watch<Query<&Health, With<Enemy>>>| {
                println!("enemy {:?} has {} HP", hp.entity(), hp.0);
            })
            watch(|gone: Removed<Query<&Health>>| {
                println!("{:?} lost its health", gone.entity());
            })
        })
        .unwrap();

    let player = app.world_mut().spawn(Health(100)).id();
    let enemy = app.world_mut().spawn((Health(30), Enemy)).id();
    app.update();

    app.world_mut().get_mut::<Health>(enemy).unwrap().0 = 10;
    app.update();

    app.world_mut().despawn(player);
    app.update();
}
