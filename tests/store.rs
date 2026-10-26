#![cfg(feature = "view")]
//! `#[derive(Store)]`: one signal per field.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::view::*;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ViewPlugin,
    ));
    app
}

#[derive(Clone, Debug, PartialEq, Store)]
struct Player {
    name: String,
    health: u32,
}

#[derive(Clone, Debug, PartialEq, Store)]
struct Pair<A, B> {
    left: A,
    right: B,
}

fn ada() -> Player {
    Player {
        name: "Ada".into(),
        health: 10,
    }
}

#[test]
fn fields_round_trip_and_set_reports_changes() {
    let player = PlayerStore::new(ada());
    assert_eq!(player.get(), ada());
    assert_eq!(player.health.get_untracked(), 10);

    // Clones share the same signals.
    let other = player.clone();
    other.health.set(7);
    assert_eq!(player.get_untracked().health, 7);

    assert!(!player.set(Player {
        name: "Ada".into(),
        health: 7,
    }));
    assert!(player.update(|value| value.name.push('!')));
    assert_eq!(player.name.get_untracked(), "Ada!");

    let from: PlayerStore = ada().into();
    assert_eq!(from.get_untracked(), ada());
    let made = store(ada);
    assert_eq!(made.get_untracked(), ada());
}

#[test]
fn generic_structs_derive_stores() {
    let pair = PairStore::new(Pair {
        left: 1_u8,
        right: "b".to_string(),
    });
    assert!(pair.set(Pair {
        left: 2,
        right: "b".into(),
    }));
    assert_eq!(pair.left.get_untracked(), 2);
}

#[test]
fn readers_update_only_for_their_field() {
    let mut app = app();
    let player = PlayerStore::new(ada());
    let name_runs = Arc::new(AtomicUsize::new(0));
    let health_runs = Arc::new(AtomicUsize::new(0));
    let (name, health) = (player.name.clone(), player.health.clone());
    let (name_probe, health_probe) = (name_runs.clone(), health_runs.clone());
    app.world_mut()
        .spawn_scene(view! {
            effect(move |_: &mut Commands| {
                name_probe.fetch_add(1, Ordering::Relaxed);
                let _ = name.get();
            })
            effect(move |_: &mut Commands| {
                health_probe.fetch_add(1, Ordering::Relaxed);
                let _ = health.get();
            })
        })
        .unwrap();
    app.update();
    let runs = || {
        (
            name_runs.load(Ordering::Relaxed),
            health_runs.load(Ordering::Relaxed),
        )
    };
    assert_eq!(runs(), (1, 1));

    // Writing the whole value notifies only the field that changed.
    player.update(|value| value.health -= 1);
    app.update();
    assert_eq!(runs(), (1, 2));

    // An unchanged write notifies no one.
    player.set(player.get_untracked());
    app.update();
    assert_eq!(runs(), (1, 2));

    player.name.set("Grace".into());
    app.update();
    assert_eq!(runs(), (2, 2));
}
