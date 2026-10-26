//! Integration coverage for `computed` and `computed_opt`.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{RadioactivePlugin, Reads, computed, computed_opt, testing::settle};

#[derive(Resource, Clone, Copy, Default)]
struct Count(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Label(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pressed;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
struct Highlight(u32);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ))
    .init_resource::<Count>();
    app
}

fn single<T: Component + Copy>(app: &mut App) -> Option<T> {
    app.world_mut()
        .query::<&T>()
        .iter(app.world())
        .next()
        .copied()
}

#[test]
fn computed_inserts_and_follows_its_reads() {
    let mut app = app();
    app.world_mut().resource_mut::<Count>().0 = 4;
    app.world_mut()
        .spawn_scene(bsn! {
            computed(|r: &Reads| Label(r.resource::<Count>().map_or(0, |c| c.0) * 10))
        })
        .unwrap();
    settle(&mut app);
    assert_eq!(single::<Label>(&mut app), Some(Label(40)));

    app.world_mut().resource_mut::<Count>().0 = 5;
    settle(&mut app);
    assert_eq!(single::<Label>(&mut app), Some(Label(50)));
}

#[test]
fn computed_reacts_to_presence_and_skips_equal_writes() {
    let mut app = app();
    let runs = Arc::new(AtomicUsize::new(0));
    let probe = runs.clone();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            computed(move |r: &Reads| {
                probe.fetch_add(1, Ordering::Relaxed);
                // Reading the computed component itself is not a dependency.
                let _ = r.this::<Label>();
                let parity = r.resource::<Count>().map_or(0, |c| c.0) % 2;
                Label(u32::from(r.this::<Pressed>().is_some()) + 2 * parity)
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    settle(&mut app);
    assert_eq!(single::<Label>(&mut app), Some(Label(0)));
    assert_eq!(
        runs.load(Ordering::Relaxed),
        1,
        "computed re-triggered itself"
    );

    app.world_mut().entity_mut(entity).insert(Pressed);
    settle(&mut app);
    assert_eq!(single::<Label>(&mut app), Some(Label(1)));

    app.world_mut().entity_mut(entity).remove::<Pressed>();
    settle(&mut app);
    assert_eq!(single::<Label>(&mut app), Some(Label(0)));

    // A dependency change that yields the same value does not mark `Label`.
    let before = app
        .world()
        .entity(entity)
        .get_ref::<Label>()
        .unwrap()
        .last_changed();
    app.world_mut().resource_mut::<Count>().0 = 2;
    settle(&mut app);
    assert_eq!(runs.load(Ordering::Relaxed), 4);
    let after = app
        .world()
        .entity(entity)
        .get_ref::<Label>()
        .unwrap()
        .last_changed();
    assert_eq!(before, after);
}

#[test]
fn computed_opt_inserts_and_removes_immutable_components() {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            computed_opt(|r: &Reads| {
                let count = r.resource::<Count>().map_or(0, |c| c.0);
                (count > 0).then_some(Highlight(count))
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert!(app.world().get::<Highlight>(entity).is_none());

    app.world_mut().resource_mut::<Count>().0 = 2;
    settle(&mut app);
    assert_eq!(app.world().get::<Highlight>(entity), Some(&Highlight(2)));

    app.world_mut().resource_mut::<Count>().0 = 0;
    settle(&mut app);
    assert!(app.world().get::<Highlight>(entity).is_none());
}
