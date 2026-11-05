//! Scene functions that take Bevy system parameters as arguments, like a
//! system, keep their logic in the body, and read values in [`rsn!`] as
//! `$name`.
//!
//! Mark a scene function with [`#[rfn]`](rfn):
//!
//! - An argument whose type is a system parameter (`Res<T>`, `ResMut<T>`,
//!   `Query<..>`, `Single<..>`, `Populated<..>`, `Local<T>`, `Commands`,
//!   `Option<Res<T>>`, `Untracked<P>` for a custom one, ..) is reactive. It is
//!   removed from the signature, so callers pass only the other arguments.
//! - `#[from("Name")] x: Ref<T>` reads component `T` on the nearest entity
//!   named `Name` (`#Name` in a scene); `#[from(entity)]` reads it on an
//!   `Entity` argument.
//! - In the body, a `let` whose value uses a reactive name is reactive too
//!   (*derived*). Everything else runs once, when the function is called.
//!
//! In `rsn!` (or `bsn!`), `$name` reads a reactive value, and whatever
//! contains it re-runs when what it read changes, exactly as with
//! `$Res<T>`. A derived value is recomputed where it is read, from the
//! parameters it needs, so nothing re-runs that doesn't depend on them. In
//! `on(..)`, `$name` is writable: `$score.0 += 1` for a `score: Res<Score>` argument.
//!
//! ```ignore
//! use bevy::{prelude::*, ui_widgets::Activate};
//! use bevy_radioactive::rfn::*;
//!
//! #[rfn]
//! fn hud(
//!     title: &'static str,
//!     score: Res<Score>,
//!     enemies: Query<&Health, With<Enemy>>,
//!     #[from("Counter")] count: Ref<Count>,
//! ) -> impl Scene {
//!     let alive = enemies.iter().filter(|hp| hp.0 > 0).count();
//!     let status = if alive == 0 { "Cleared" } else { "Fighting" };
//!     let heading = title.to_uppercase();       // plain: runs once
//!
//!     rsn! {
//!         #Counter
//!         Node Count(0)
//!         Children [
//!             Text({ heading })
//!             --
//!             Text({ format!("{} points, {} left: {}", $score.0, $alive, $status) })
//!             --
//!             Button on(|_: On<Activate>| $count.0 += 1)
//!             --
//!             if $score.0 > 100 { Text("High score!") }
//!             --
//!             for hp in $enemies { Text({ hp.0.to_string() }) }
//!         ]
//!     }
//! }
//! ```
//!
//! `#[rfn]` compiles to `rsn!`, so it uses [`ReactivePlugin`] and everything
//! in [`reactive`](crate::reactive) works in its scenes. Plain locals and
//! arguments are cloned where they are used, so they need `Clone`.

pub use crate::macros::rfn;
pub use crate::reactive::*;
