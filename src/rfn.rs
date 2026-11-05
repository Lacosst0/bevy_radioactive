//! Scene functions that take Bevy system parameters as arguments, like a
//! system, keep their logic in the body, and read values in [`rsn!`]. What is
//! reactive says so in its type, [`Reactive<..>`](Reactive), and is read as
//! `$name`.
//!
//! Mark a scene function with [`#[rfn]`](rfn):
//!
//! - An argument whose type is a system parameter (`Res<T>`, `ResMut<T>`,
//!   `Query<..>`, `Single<..>`, `Populated<..>`, `Local<T>`, `Commands`,
//!   `Option<Res<T>>`, `Untracked<P>` for a custom one, ..) is supplied by the
//!   runtime. If only the scene uses it, it is removed from the signature, so
//!   callers pass only the other arguments; if the body uses it too
//!   (`commands.spawn_scene(..)`), it stays an argument, so a system can be an
//!   `#[rfn]`.
//! - `score: Reactive<Res<Score>>` is tracked: the scene reads it as
//!   `$score`, and whatever does re-runs when it changes. Without `Reactive`
//!   (`mut commands: Commands`), it is untracked: the scene uses it as
//!   `commands` and never re-runs because of it.
//! - `count: Reactive<Ref<T>, Of<Name>>` reads component `T` on the nearest
//!   entity named `#Name` (checked at compile time against the scene),
//!   `Of<entity>` on an `Entity` argument, and `Of<"Name">` on a name looked
//!   up at runtime. Without `Reactive` (`Ref<T, Of<Name>>`), it only works in
//!   `on(..)`.
//! - In the body, `let x: Reactive<_> = ..;` is a derived value, recomputed
//!   where the scene reads it (`$x`). Everything else, other `let`s included,
//!   runs once, when the function is called.
//!
//! In `rsn!` (or `bsn!`), `$name` reads a reactive value, and whatever
//! contains it re-runs when what it read changes, exactly as with
//! `$Res<T>`. A derived value is recomputed where it is read, from the
//! parameters it needs, so nothing re-runs that doesn't depend on them. In
//! `on(..)`, `$name` is writable: `$score.0 += 1` for a
//! `score: Reactive<Res<Score>>` argument.
//!
//! ```ignore
//! use bevy::{prelude::*, ui_widgets::Activate};
//! use bevy_radioactive::rfn::*;
//!
//! #[rfn]
//! fn hud(
//!     title: &'static str,
//!     score: Reactive<Res<Score>>,
//!     enemies: Reactive<Query<&Health, With<Enemy>>>,
//!     count: Reactive<Ref<Count>, Of<Counter>>,
//!     mut commands: Commands,
//! ) -> impl Scene {
//!     let alive: Reactive<_> = enemies.iter().filter(|hp| hp.0 > 0).count();
//!     let status: Reactive<_> = if alive == 0 { "Cleared" } else { "Fighting" };
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
//!             Button on(|_: On<Activate>| {
//!                 $count.0 += 1;
//!                 commands.spawn(Enemy);
//!             })
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

use std::marker::PhantomData;

/// Marks an `#[rfn]` argument or `let` as reactive: `score: Reactive<Res<Score>>`,
/// `count: Reactive<Ref<Count>, Of<Counter>>`, `let x: Reactive<_> = ..;`.
///
/// `#[rfn]` removes it while expanding; it never exists at runtime.
pub struct Reactive<P, O = ()>(PhantomData<fn() -> (P, O)>);

/// Says which entity a `Ref<T>` argument of an `#[rfn]` reads:
/// `Of<Counter>` for the nearest `#Counter`, `Of<player>` for an `Entity`
/// argument, `Of<"Counter">` for a name looked up at runtime.
///
/// `#[rfn]` removes it while expanding; it never exists at runtime.
pub struct Of<T>(PhantomData<fn() -> T>);
