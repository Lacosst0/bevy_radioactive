#![cfg(all(feature = "reactive", feature = "ui"))]
//! Plays through `examples/game_ui` without a window: the same game rules
//! and `rsn!` scenes, driven by `Activate` events and key presses.

#[path = "../examples/game_ui/game.rs"]
#[allow(dead_code)]
mod game;
#[path = "../examples/game_ui/ui.rs"]
mod ui;

use bevy::{
    asset::AssetPlugin, prelude::*, scene::ScenePlugin, state::app::StatesPlugin,
    ui_widgets::Activate,
};
use bevy_radioactive::reactive::*;
use game::*;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        StatesPlugin,
        ReactivePlugin,
        GamePlugin,
    ))
    .init_resource::<ButtonInput<KeyCode>>();
    app.world_mut().spawn_scene(ui::root()).unwrap();
    app.update();
    app
}

/// Every text on screen, in tree order.
fn texts(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, (With<Node>, Without<ChildOf>)>()
        .iter(world)
        .collect();
    let mut out = Vec::new();
    for root in roots {
        walk(world, root, &mut out);
    }
    out
}

fn walk(world: &World, entity: Entity, out: &mut Vec<String>) {
    if let Some(text) = world.get::<Text>(entity) {
        out.push(text.0.clone());
    }
    if let Some(children) = world.get::<Children>(entity) {
        for &child in children {
            walk(world, child, out);
        }
    }
}

fn shows(app: &mut App, text: &str) -> bool {
    texts(app).iter().any(|t| t == text)
}

/// Clicks the button (or slot) whose label is `label`.
fn click(app: &mut App, label: &str) {
    let world = app.world_mut();
    let mut texts = world.query::<(&Text, &ChildOf)>();
    let button = texts
        .iter(world)
        .find(|(text, _)| text.0 == label)
        .map_or_else(
            || panic!("no button {label:?}"),
            |(_, parent)| parent.parent(),
        );
    world.trigger(Activate { entity: button });
    app.update();
    // State transitions apply in the next frame's `StateTransition`.
    app.update();
}

fn press(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.update();
    let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    input.release(key);
    input.clear();
    app.update();
}

fn health(app: &mut App) -> Health {
    let world = app.world_mut();
    *world
        .query_filtered::<&Health, With<Player>>()
        .single(world)
        .unwrap()
}

fn set_health(app: &mut App, current: u32) {
    let world = app.world_mut();
    world
        .query_filtered::<&mut Health, With<Player>>()
        .single_mut(world)
        .unwrap()
        .current = current;
    app.update();
}

#[test]
fn menu_setting_and_start() {
    let mut app = app();
    assert_eq!(
        texts(&mut app),
        ["Radioactive Quest", "Play", "Difficulty", "Normal"]
    );
    click(&mut app, "Difficulty");
    assert!(shows(&mut app, "Hard"));
    click(&mut app, "Play");
    assert!(!shows(&mut app, "Play"));
    assert!(shows(&mut app, "100 / 100"));
    assert!(shows(&mut app, "Gold 10"));
    assert!(shows(&mut app, "Bare hands"));
}

#[test]
fn hud_follows_gameplay() {
    let mut app = app();
    click(&mut app, "Play");
    click(&mut app, "Take a hit");
    assert_eq!(health(&mut app).current, 80);
    assert!(shows(&mut app, "80 / 100"));
    assert!(!shows(&mut app, "Low health! Drink a potion."));

    set_health(&mut app, 20);
    assert!(shows(&mut app, "Low health! Drink a potion."));

    click(&mut app, "Open chest");
    assert!(shows(&mut app, "Gold 15"));
}

#[test]
fn inventory_select_use_and_equip() {
    let mut app = app();
    click(&mut app, "Play");
    press(&mut app, KeyCode::KeyI);
    assert!(shows(&mut app, "Bag (3 items)"));
    assert!(shows(&mut app, "Click an item to inspect it."));
    let bag = texts(&mut app);
    let potion = bag.iter().position(|t| t == "Potion").unwrap();
    let sword = bag.iter().position(|t| t == "Rusty sword").unwrap();
    assert!(potion < sword, "slots in bag order: {bag:?}");
    assert!(shows(&mut app, "x2"));

    set_health(&mut app, 30);
    click(&mut app, "Potion");
    assert!(shows(&mut app, "Restores 40 health."));
    click(&mut app, "Drink");
    assert_eq!(health(&mut app).current, 70);
    assert!(!shows(&mut app, "x2"), "one potion left");
    click(&mut app, "Drink");
    assert!(!shows(&mut app, "Potion"), "used up");
    assert!(shows(&mut app, "Click an item to inspect it."));

    click(&mut app, "Rusty sword");
    click(&mut app, "Equip");
    assert!(shows(&mut app, "Wielding Rusty sword"));
    assert!(shows(&mut app, "equipped"));
    assert!(shows(&mut app, "Unequip"));
    click(&mut app, "Unequip");
    assert!(shows(&mut app, "Bare hands"));
    assert!(!shows(&mut app, "equipped"));

    press(&mut app, KeyCode::KeyI);
    assert!(!shows(&mut app, "Rusty sword"));
}

#[test]
fn chest_loot_fills_the_bag() {
    let mut app = app();
    click(&mut app, "Play");
    click(&mut app, "Bag [I]");
    click(&mut app, "Open chest"); // 15 gold: junk
    click(&mut app, "Open chest"); // 20 gold: a sword
    click(&mut app, "Open chest"); // 25 gold: a potion, stacked
    let bag = texts(&mut app);
    assert!(bag.contains(&"Old boot".to_string()), "{bag:?}");
    assert!(bag.contains(&"Short sword".to_string()), "{bag:?}");
    assert!(bag.contains(&"x3".to_string()), "{bag:?}");
    click(&mut app, "Old boot");
    click(&mut app, "Sell");
    assert!(shows(&mut app, "Gold 27"));
}

#[test]
fn pause_game_over_and_restart() {
    let mut app = app();
    click(&mut app, "Play");
    press(&mut app, KeyCode::Escape);
    assert!(shows(&mut app, "Paused"));
    assert!(
        shows(&mut app, "100 / 100"),
        "the HUD stays under the pause menu"
    );
    click(&mut app, "Resume");
    assert!(!shows(&mut app, "Paused"));

    set_health(&mut app, 5);
    click(&mut app, "Take a hit");
    assert!(shows(&mut app, "You died"));
    assert!(shows(&mut app, "You had 10 gold"));
    click(&mut app, "Try again");
    assert!(shows(&mut app, "100 / 100"));

    press(&mut app, KeyCode::Escape);
    click(&mut app, "Quit to menu");
    assert!(shows(&mut app, "Play"));
}
