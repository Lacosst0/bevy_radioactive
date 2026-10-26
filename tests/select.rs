//! Integration coverage for `Selection` marker moves.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, Reads, computed,
    select::{SelectCommandsExt, SelectEntityCommandsExt, Selected, Selection},
    testing::settle,
};

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Lit(bool);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Hovered;

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

/// A list with `Selection` and three rows; returns the list, its rows, and
/// per-row run counters of a `computed` reading the marker.
fn list(app: &mut App) -> (Entity, Vec<Entity>, Vec<Arc<AtomicUsize>>) {
    let runs: Vec<Arc<AtomicUsize>> = (0..3).map(|_| Arc::default()).collect();
    let row = |runs: &Arc<AtomicUsize>| {
        let runs = runs.clone();
        bsn! {
            computed(move |r: &Reads| {
                runs.fetch_add(1, Ordering::Relaxed);
                Lit(r.this_has::<Selected>())
            })
        }
    };
    let (a, b, c) = (row(&runs[0]), row(&runs[1]), row(&runs[2]));
    let list = app
        .world_mut()
        .spawn_scene(bsn! { Selection Children [ {a} -- {b} -- {c} ] })
        .unwrap()
        .id();
    settle(app);
    let rows = app.world().get::<Children>(list).unwrap().to_vec();
    (list, rows, runs)
}

fn lit(app: &App, rows: &[Entity]) -> Vec<bool> {
    rows.iter()
        .map(|&row| app.world().get::<Lit>(row).unwrap().0)
        .collect()
}

fn counts(runs: &[Arc<AtomicUsize>]) -> Vec<usize> {
    runs.iter()
        .map(|runs| runs.load(Ordering::Relaxed))
        .collect()
}

fn current(app: &App, list: Entity) -> Option<Entity> {
    app.world().get::<Selection>(list).unwrap().current()
}

#[test]
fn selecting_moves_the_marker_and_wakes_only_two_rows() {
    let mut app = app();
    let (list, rows, runs) = list(&mut app);
    assert_eq!(counts(&runs), [1, 1, 1]);

    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[0]));
    settle(&mut app);
    assert_eq!(lit(&app, &rows), [true, false, false]);
    assert_eq!(current(&app, list), Some(rows[0]));
    assert_eq!(counts(&runs), [2, 1, 1]);

    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[2]));
    settle(&mut app);
    assert_eq!(lit(&app, &rows), [false, false, true]);
    assert_eq!(counts(&runs), [3, 1, 2], "row 1 was not involved");

    // Selecting the selected entity again is a no-op.
    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[2]));
    settle(&mut app);
    assert_eq!(counts(&runs), [3, 1, 2]);

    app.world_mut().commands().select::<Selected>(list, None);
    settle(&mut app);
    assert_eq!(lit(&app, &rows), [false, false, false]);
    assert_eq!(current(&app, list), None);
}

#[test]
fn entity_commands_select_in_the_nearest_ancestor_and_toggle() {
    let mut app = app();
    let (list, rows, _) = list(&mut app);

    app.world_mut()
        .commands()
        .entity(rows[1])
        .select::<Selected>();
    settle(&mut app);
    assert_eq!(current(&app, list), Some(rows[1]));

    app.world_mut()
        .commands()
        .entity(rows[1])
        .toggle_select::<Selected>();
    settle(&mut app);
    assert_eq!(current(&app, list), None);
    assert_eq!(lit(&app, &rows), [false, false, false]);

    app.world_mut()
        .commands()
        .entity(rows[0])
        .toggle_select::<Selected>();
    settle(&mut app);
    assert_eq!(current(&app, list), Some(rows[0]));
}

#[test]
fn selection_is_a_reactive_source() {
    let mut app = app();
    let (list, rows, _) = list(&mut app);
    let reader = app
        .world_mut()
        .spawn_scene(bsn! {
            computed(move |r: &Reads| {
                Lit(r.get::<Selection>(list).and_then(Selection::current).is_some())
            })
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert_eq!(app.world().get::<Lit>(reader), Some(&Lit(false)));

    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[1]));
    settle(&mut app);
    assert_eq!(app.world().get::<Lit>(reader), Some(&Lit(true)));
}

#[test]
fn despawning_the_selected_entity_clears_the_selection() {
    let mut app = app();
    let (list, rows, _) = list(&mut app);
    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[1]));
    settle(&mut app);

    app.world_mut().despawn(rows[1]);
    settle(&mut app);
    assert_eq!(current(&app, list), None);

    // Removing the marker by hand also clears it.
    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[0]));
    settle(&mut app);
    app.world_mut().entity_mut(rows[0]).remove::<Selected>();
    settle(&mut app);
    assert_eq!(current(&app, list), None);
}

#[test]
fn removing_the_selection_releases_the_marker() {
    let mut app = app();
    let (list, rows, _) = list(&mut app);
    app.world_mut()
        .commands()
        .select::<Selected>(list, Some(rows[2]));
    settle(&mut app);

    app.world_mut().entity_mut(list).remove::<Selection>();
    settle(&mut app);
    assert!(!app.world().entity(rows[2]).contains::<Selected>());
    assert_eq!(lit(&app, &rows), [false, false, false]);

    // Despawning the whole list leaves nothing behind.
    app.world_mut().despawn(list);
    settle(&mut app);
    let markers = app
        .world_mut()
        .query::<&Selected>()
        .iter(app.world())
        .count();
    assert_eq!(markers, 0);
}

#[test]
fn custom_markers_select_independently() {
    let mut app = app();
    let list = app
        .world_mut()
        .spawn((
            Selection::<Selected>::default(),
            Selection::<Hovered>::default(),
        ))
        .id();
    let a = app.world_mut().spawn(ChildOf(list)).id();
    let b = app.world_mut().spawn(ChildOf(list)).id();
    settle(&mut app);

    app.world_mut().commands().entity(a).select::<Selected>();
    app.world_mut().commands().entity(b).select::<Hovered>();
    settle(&mut app);
    assert!(app.world().entity(a).contains::<Selected>());
    assert!(!app.world().entity(a).contains::<Hovered>());
    assert!(app.world().entity(b).contains::<Hovered>());

    app.world_mut().commands().entity(a).select::<Hovered>();
    settle(&mut app);
    assert!(app.world().entity(a).contains::<Hovered>());
    assert!(!app.world().entity(b).contains::<Hovered>());
    assert!(app.world().entity(a).contains::<Selected>());
}
