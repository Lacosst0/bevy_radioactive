//! A small game UI written only with `rsn!`: a main menu with a setting, an
//! in-game HUD (health bar, gold, weapon), an inventory with selectable
//! slots, a pause menu and a game over screen.
//!
//! The game rules in `game.rs` are plain Bevy: states, resources,
//! components, systems and observers. The scenes in `ui.rs` read that state
//! with `$` and update themselves; there are no marker components or sync
//! systems. Keys: `I` opens the bag, `Esc` pauses.

mod game;
mod ui;

use bevy::prelude::*;
use bevy_radioactive::reactive::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, ReactivePlugin, game::GamePlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands.spawn_scene(ui::root());
}
