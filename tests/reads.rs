//! Integration coverage for presence-only reads, inherited components and
//! post-layout effects.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, ReactiveSet, Reads,
    coat::{effect, layout_effect},
    computed,
    testing::{settle, settle_n},
};

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Level(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Flag(bool);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Theme(u32);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Accent(u32);

/// Written by a system between the main and post-layout phases, standing in
/// for UI layout output such as `ComputedNode`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Measured(u32);

#[derive(Resource, Default)]
struct Layout(u32);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ));
    app
}

fn counter() -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    (runs.clone(), runs)
}

#[test]
fn has_depends_on_presence_only() {
    let mut app = app();
    let (runs, probe) = counter();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            effect(move |r: &Reads, _: &mut Commands| {
                probe.fetch_add(1, Ordering::Relaxed);
                log.lock().unwrap().push(r.this_has::<Level>());
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(runs.load(Ordering::Relaxed), 1);

    app.world_mut().entity_mut(entity).insert(Level(1));
    settle(&mut app);
    assert_eq!(runs.load(Ordering::Relaxed), 2);

    // A value change is not a presence change.
    app.world_mut().get_mut::<Level>(entity).unwrap().0 = 2;
    settle(&mut app);
    assert_eq!(runs.load(Ordering::Relaxed), 2);

    app.world_mut().entity_mut(entity).remove::<Level>();
    settle(&mut app);
    assert_eq!(runs.load(Ordering::Relaxed), 3);
    assert_eq!(*seen.lock().unwrap(), vec![false, true, false]);
}

#[test]
fn has_on_other_entities_tracks_their_despawn() {
    let mut app = app();
    let other = app.world_mut().spawn(Flag(true)).id();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            computed(move |r: &Reads| Level(u32::from(r.has::<Flag>(other))))
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(app.world().get::<Level>(entity), Some(&Level(1)));

    app.world_mut().despawn(other);
    settle(&mut app);
    assert_eq!(app.world().get::<Level>(entity), Some(&Level(0)));
}

fn accent(r: &Reads) -> Accent {
    Accent(r.inherited::<Theme>().map_or(0, |theme| theme.0))
}

#[test]
fn inherited_finds_the_nearest_provider_and_follows_overrides() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Theme(1)
            Children [
                #Middle
                Children [
                    #Leaf
                    computed(accent)
                ]
            ]
        })
        .unwrap()
        .id();
    settle(&mut app);
    let leaf = app
        .world_mut()
        .query_filtered::<Entity, With<Accent>>()
        .single(app.world())
        .unwrap();
    let middle = app.world().get::<ChildOf>(leaf).unwrap().parent();
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(1)));

    // The provider's value changes.
    app.world_mut().entity_mut(root).insert(Theme(2));
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(2)));

    // A closer provider overrides it, then goes away again.
    app.world_mut().entity_mut(middle).insert(Theme(3));
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(3)));
    app.world_mut().entity_mut(root).insert(Theme(4));
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(3)));
    app.world_mut().entity_mut(middle).remove::<Theme>();
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(4)));

    // The entity's own component wins.
    app.world_mut().entity_mut(leaf).insert(Theme(5));
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(5)));
    app.world_mut().entity_mut(leaf).remove::<Theme>();
    settle(&mut app);

    // Reparenting under another provider.
    let other = app.world_mut().spawn(Theme(9)).id();
    app.world_mut().entity_mut(middle).insert(ChildOf(other));
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(9)));

    // Losing every provider.
    app.world_mut().entity_mut(middle).remove::<ChildOf>();
    settle(&mut app);
    assert_eq!(app.world().get::<Accent>(leaf), Some(&Accent(0)));
}

#[test]
fn inherited_reader_despawns_cleanly_with_its_provider() {
    let mut app = app();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Theme(1)
            Children [ computed(accent) ]
        })
        .unwrap()
        .id();
    settle(&mut app);
    app.world_mut().despawn(root);
    settle_n(&mut app, 2);
    let left = app.world_mut().query::<&Accent>().iter(app.world()).count();
    assert_eq!(left, 0);
}

fn layout_app() -> App {
    let mut app = app();
    app.init_resource::<Layout>().add_systems(
        PostUpdate,
        (|layout: Res<Layout>, mut q: Query<&mut Measured>| {
            for mut measured in &mut q {
                measured.set_if_neq(Measured(layout.0));
            }
        })
        .after(ReactiveSet::Settled)
        .before(ReactiveSet::PostLayout),
    );
    app
}

#[test]
fn layout_effects_see_layout_output_of_the_same_frame() {
    let mut app = layout_app();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Measured
            layout_effect(move |r: &Reads, _: &mut Commands| {
                log.lock().unwrap().push(r.this::<Measured>().map_or(0, |m| m.0));
            })
            // Main-phase reads of the same value lag one frame.
            computed(|r: &Reads| Level(r.this::<Measured>().map_or(0, |m| m.0)))
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(*seen.lock().unwrap(), vec![0]);

    app.world_mut().resource_mut::<Layout>().0 = 7;
    settle(&mut app);
    assert_eq!(*seen.lock().unwrap(), vec![0, 7]);
    assert_eq!(app.world().get::<Level>(entity), Some(&Level(0)));

    settle(&mut app);
    assert_eq!(*seen.lock().unwrap(), vec![0, 7], "ran without a change");
    assert_eq!(app.world().get::<Level>(entity), Some(&Level(7)));
}

#[test]
fn layout_effect_cleanup_runs_on_despawn() {
    let mut app = layout_app();
    let (cleanups, probe) = counter();
    let entity = app
        .world_mut()
        .spawn_scene(bsn! {
            Measured
            layout_effect(move |r: &Reads, _: &mut Commands| {
                let _ = r.this::<Measured>();
                let probe = probe.clone();
                move |_: &mut Commands| {
                    probe.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    app.world_mut().resource_mut::<Layout>().0 = 1;
    settle(&mut app);
    assert_eq!(cleanups.load(Ordering::Relaxed), 1, "cleanup before re-run");

    app.world_mut().despawn(entity);
    settle(&mut app);
    assert_eq!(cleanups.load(Ordering::Relaxed), 2);
    app.world_mut().resource_mut::<Layout>().0 = 2;
    settle(&mut app);
    assert_eq!(cleanups.load(Ordering::Relaxed), 2);
}
