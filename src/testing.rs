//! Helpers for tests and headless tools that drive an [`App`] by hand.

use bevy::prelude::*;

/// Runs one frame and applies everything it queued.
///
/// Flushes pending commands (such as scenes spawned with `spawn_scene`),
/// runs `app.update()`, then flushes again, so subscription deliveries
/// queued at the end of the frame, such as removals, have run.
pub fn settle(app: &mut App) {
    app.world_mut().flush();
    app.update();
    app.world_mut().flush();
}

/// Runs [`settle`] `frames` times.
pub fn settle_n(app: &mut App, frames: usize) {
    for _ in 0..frames {
        settle(app);
    }
}
