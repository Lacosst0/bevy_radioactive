//! Two independent counters, each with its own `Count` component, plus a
//! total that watches every counter with a query.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Default)]
struct Count(i32);

#[derive(Component, Clone, Default)]
struct Label(String);

#[derive(Clone, EntityEvent)]
struct Increment {
    entity: Entity,
}

fn counter(name: &'static str) -> impl Scene {
    rsn! {
        #Counter
        Count(0)
        Children [
            Name({name})
            // `#Counter#Counter$Count` is on the nearest `#Counter`: this one, not the other.
            on(|_: On<Increment>| #Counter$Count.0 += 1)
            --
            Label({ format!("{name} = {}", #Counter$Count.0) })
            --
            if #Counter$Count.0 >= 3 { Label("  that's a lot") }
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
    ));
    app.world_mut()
        .spawn_scene_list(rsn_list! {
            @counter("a")
            --
            @counter("b")
            --
            Label
            // Plain `bsn!` items work too. `Query` parameters re-run when any
            // `Count` changes.
            watch(|counts: Query<&Count>| {
                Label(format!("total = {}", counts.iter().map(|c| c.0).sum::<i32>()))
            })
        })
        .unwrap();

    let a = find(app.world_mut(), "a");
    for _ in 0..3 {
        app.world_mut().trigger(Increment { entity: a });
    }
    app.world_mut().flush();
    app.update();

    let mut labels = app.world_mut().query::<&Label>();
    for label in labels.iter(app.world()) {
        println!("{}", label.0);
    }
}

fn find(world: &mut World, name: &str) -> Entity {
    let mut names = world.query::<(Entity, &Name)>();
    names
        .iter(world)
        .find(|(_, n)| n.as_str() == name)
        .map(|(entity, _)| entity)
        .expect("named entity")
}
