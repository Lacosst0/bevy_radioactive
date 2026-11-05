//! A scoreboard written with `#[rfn]`: the body declares what it reads and
//! derives values from it; `$name` reads them in `rsn!`.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::rfn::*;

#[derive(Resource, Clone, Copy, Default)]
struct Score(u32);

#[derive(Component, Clone, Copy, PartialEq)]
struct Health(u32);

#[derive(Component, Clone, Copy, Default)]
struct Enemy;

#[derive(Component, Clone, Default)]
struct Label(String);

#[derive(Clone, EntityEvent)]
struct Hit {
    entity: Entity,
}

#[rfn]
fn scoreboard(title: &'static str) -> impl Scene {
    // System parameters, as in a Bevy system.
    let score: Res<Score>;
    let enemies: Query<&Health, With<Enemy>>;

    // Derived: they read reactive values, so they re-run when those change.
    let alive = enemies.iter().filter(|hp| hp.0 > 0).count();
    let status = if alive == 0 { "cleared" } else { "fighting" };

    // Plain: runs once.
    let heading = title.to_uppercase();

    rsn! {
        Node
        Children [
            Label({ heading })
            --
            Label({ format!("score {} | {} alive | {}", $score.0, $alive, $status) })
            --
            #Button
            // In `on(..)` reads are writable: `Res<Score>` becomes `ResMut<Score>`.
            on(|_: On<Hit>| $score.0 += 10)
            --
            if $alive == 0 { Label("  you win!") }
        ]
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Score>();
    let enemy = app.world_mut().spawn((Enemy, Health(1))).id();
    app.world_mut().spawn_scene(scoreboard("round 1")).unwrap();
    print(&mut app);

    let button = find(app.world_mut(), "Button");
    app.world_mut().trigger(Hit { entity: button });
    app.world_mut().get_mut::<Health>(enemy).unwrap().0 = 0;
    app.update();
    print(&mut app);
}

fn print(app: &mut App) {
    let mut labels = app.world_mut().query::<&Label>();
    for label in labels.iter(app.world()) {
        println!("{}", label.0);
    }
    println!("--");
}

fn find(world: &mut World, name: &str) -> Entity {
    let mut names = world.query::<(Entity, &Name)>();
    names
        .iter(world)
        .find(|(_, n)| n.as_str() == name)
        .map(|(entity, _)| entity)
        .expect("named entity")
}
