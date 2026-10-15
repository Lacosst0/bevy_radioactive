use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::watch::*;

#[derive(Component, Clone, Copy, Default)]
struct Meter(u32);

#[derive(Component, Clone, Copy, Default)]
struct Main;

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        WatchPlugin,
    ));

    // The query reads `Meter` and filters on `Main`, so the watch re-runs when
    // either changes, is inserted, or is removed.
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                Main Meter(1)
                --
                Name::new("meter-listener")
                watch(|meter: Query<&Meter, With<Main>>, this: This<&Name>| {
                    let Some(name) = this.get() else { return };
                    match meter.single() {
                        Ok(meter) => println!("{name} sees meter {}", meter.0),
                        Err(_) => println!("{name} sees no meter"),
                    }
                })
            ]
        })
        .unwrap();
    app.update();

    let meter = app
        .world_mut()
        .query_filtered::<Entity, With<Main>>()
        .single(app.world())
        .unwrap();
    app.world_mut().get_mut::<Meter>(meter).unwrap().0 = 2;
    app.update();

    app.world_mut().despawn(meter);
    app.update();
}
