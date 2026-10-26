//! `CopySignal`: `Copy` handles owned by entities.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    coat::{OwnedSignals, SignalCell, SignalOf},
    prelude::*,
    webby::component,
};

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

#[test]
fn handles_are_copied_into_closures_and_die_with_their_owner() {
    let mut world = World::new();
    let owner = world.spawn_empty().id();
    let count = world.spawn_signal(owner, 1_u32);
    let add = move || count.increment();
    let double = move || count.get() * 2;
    add();
    assert_eq!(double(), 4);

    let cell = count.cell().unwrap();
    assert_eq!(world.get::<SignalOf>(cell), Some(&SignalOf(owner)));
    assert_eq!(world.get::<SignalCell<u32>>(cell).unwrap().handle(), count);
    assert_eq!(
        world.get::<OwnedSignals>(owner).map(|cells| cells.len()),
        Some(1)
    );

    world.despawn(owner);
    assert!(world.get_entity(cell).is_err());
    assert!(!count.is_alive());
    assert_eq!(count.try_get(), None);
    assert!(!count.try_set(5));
    assert_eq!(count.try_update(|value| *value += 1), None);

    // A new signal never answers to the dead handle.
    let next = CopySignal::new(9_u32);
    assert_ne!(next, count);
    assert_eq!(count.try_get(), None);
    next.dispose();
}

#[test]
#[should_panic(expected = "`CopySignal<u32>` used after it was disposed")]
fn get_after_the_owner_despawned_panics_with_a_message() {
    let mut world = World::new();
    let owner = world.spawn_empty().id();
    let count = world.spawn_signal(owner, 1_u32);
    world.despawn(owner);
    let _ = count.get();
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
struct Shown(u32);

#[test]
fn scenes_own_signals_and_effects_track_them() {
    let mut app = app();
    let runs = Arc::new(AtomicUsize::new(0));
    let count = copy_signal(|| 0_u32);
    let probe = runs.clone();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            owns(count)
            // Two closures capture the same handle; neither needs a clone.
            computed(move |_: &Reads| Shown(count.get()))
            effect(move |_: &Reads, _: &mut Commands| {
                probe.fetch_add(1, Ordering::Relaxed);
                let _ = count.get();
            })
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(runs.load(Ordering::Relaxed), 1);
    assert_eq!(app.world().get::<Shown>(root), Some(&Shown(0)));

    count.set(3);
    app.update();
    assert_eq!(runs.load(Ordering::Relaxed), 2);
    assert_eq!(app.world().get::<Shown>(root), Some(&Shown(3)));
    app.update();
    assert_eq!(runs.load(Ordering::Relaxed), 2);
    assert_eq!(
        app.world().get::<SignalOf>(count.cell().unwrap()),
        Some(&SignalOf(root))
    );

    app.world_mut().despawn(root);
    assert!(!count.is_alive());
}

#[test]
fn commands_spawn_owned_signals() {
    let mut app = app();
    let owner = app.world_mut().spawn_empty().id();
    let mut queue = bevy::ecs::world::CommandQueue::default();
    let signal = Commands::new(&mut queue, app.world()).spawn_signal(owner, "a".to_string());
    // Usable before the commands apply.
    assert_eq!(signal.get(), "a");
    assert_eq!(signal.cell(), None);
    queue.apply(app.world_mut());
    assert!(signal.cell().is_some());
    app.world_mut().despawn(owner);
    assert_eq!(signal.try_get(), None);
}

#[derive(Resource, Clone, Default)]
struct Handle(Option<CopySignal<i32>>);

#[derive(Component, Clone, Default)]
struct Label(String);

#[test]
fn use_copy_signal_persists_across_renders_and_is_owned_by_the_host() {
    let mut app = app();
    app.init_resource::<Handle>();
    let host = app
        .world_mut()
        .spawn_scene(bsn! {
            component(|cx| {
                let count = cx.use_copy_signal(|| 1);
                cx.use_effect((), move |(), commands| {
                    commands.insert_resource(Handle(Some(count)));
                });
                bsn! { Label({count.get().to_string()}) }
            })
        })
        .unwrap()
        .id();
    app.update();
    let count = app.world().resource::<Handle>().0.unwrap();
    count.set(7);
    app.update();
    let label = |app: &mut App| {
        app.world_mut()
            .query::<&Label>()
            .single(app.world())
            .unwrap()
            .0
            .clone()
    };
    assert_eq!(label(&mut app), "7");
    assert_eq!(app.world().resource::<Handle>().0, Some(count));
    let cell = count.cell().unwrap();
    assert_eq!(app.world().get::<SignalOf>(cell), Some(&SignalOf(host)));

    app.world_mut().despawn(host);
    app.update();
    assert!(!count.is_alive());
}

#[test]
fn coat_reads_copy_signals_with_dollar() {
    let mut app = app();
    let count = copy_signal(|| 1_u32);
    app.world_mut()
        .spawn_scene(coat! {
            owns(count)
            Text($(format!("n = {}", $count)))
        })
        .unwrap();
    app.update();
    let text = |app: &mut App| {
        app.world_mut()
            .query::<&Text>()
            .single(app.world())
            .unwrap()
            .0
            .clone()
    };
    assert_eq!(text(&mut app), "n = 1");
    count.set(2);
    app.update();
    assert_eq!(text(&mut app), "n = 2");
}
