use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Component, Clone, Copy, Default)]
struct Health(u32);

#[derive(Component, Clone, Copy, Default)]
struct Enemy;

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ));

    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Children [
                #Player Health(100)
                --
                Name::new("health-log")
                // One entity, by name. `None` once its health is gone.
                watch_from(#Player, |hp: Option<Ref<Health>>| match hp {
                    Some(hp) => println!("the player has {} HP", hp.0),
                    None => println!("the player lost its health"),
                })
                // Every enemy, once per enemy whose health changes.
                watch_from(any_with::<Enemy>(), |e: Entity, hp: Ref<Health>| {
                    println!("enemy {e} has {} HP", hp.0);
                })
            ]
        })
        .unwrap()
        .id();

    let enemy = app.world_mut().spawn((Health(30), Enemy)).id();
    app.update();

    app.world_mut().get_mut::<Health>(enemy).unwrap().0 = 10;
    app.update();

    let player = app.world().get::<Children>(root).unwrap()[0];
    app.world_mut().despawn(player);
    app.update();
}
