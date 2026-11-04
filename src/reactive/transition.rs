//! Enter and exit transitions for region content.
//!
//! Put a [`Transition`] on an entity that a region shows (the root of an
//! `if`/`match` arm or a `for` row in `rsn!`, or of a `when`/`switch`/`each`
//! scene) and it fades in and out instead of popping:
//!
//! - When it appears, its [`Presence`] counts up from 0 to 1 over
//!   [`Transition::enter`] seconds.
//! - When the region removes it, it is not despawned right away. It gets
//!   [`Leaving`], stays in place, and its [`Presence`] counts down to 0 over
//!   [`Transition::exit`] seconds; the frame after it reaches 0, it
//!   despawns.
//!
//! The animation itself is ordinary reactive code that reads [`Presence`]:
//!
//! ```ignore
//! if $Res<Menu>.open {
//!     #Panel
//!     Node
//!     Transition::new(0.25).ease(EaseFunction::CubicOut)
//!     BackgroundColor(Color::BLACK.with_alpha(0.8 * $Ref<Presence>(#Panel).value()))
//! }
//! ```
//!
//! Content that comes back while it is still leaving (the condition flips
//! back, or a removed `for` row returns with the same key and item) turns
//! around from where it is instead of being rebuilt. A region's first
//! content doesn't animate in unless its root's transition says
//! [`appear`](Transition::appear).

use bevy::{curve::Curve, prelude::*};

/// Animates an entity in when a region shows it and out when the region
/// removes it, by driving its [`Presence`]. Only roots of region content
/// (what an `if` arm, `match` arm or `for` row spawns at its top level) are
/// animated; deeper entities go with their root.
///
/// ```ignore
/// Transition::new(0.2)                       // 0.2 s in and out, linear
/// Transition::new(0.2).exit(0.4)             // slower out
/// Transition::new(0.3).ease(EaseFunction::CubicOut).appear()
/// ```
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Presence)]
pub struct Transition {
    /// Seconds to animate in. 0 shows it at once.
    pub enter: f32,
    /// Seconds to animate out before despawning. 0 despawns it at once.
    pub exit: f32,
    /// Easing applied to [`Presence::value`].
    pub ease: EaseFunction,
    /// Whether the region's first content animates in too. Off by default,
    /// so a scene's initial state shows without playing its transitions.
    pub appear: bool,
}

impl Default for Transition {
    fn default() -> Self {
        Self::new(0.2)
    }
}

impl Transition {
    /// `seconds` in and out, linear.
    #[must_use]
    pub const fn new(seconds: f32) -> Self {
        Self {
            enter: seconds,
            exit: seconds,
            ease: EaseFunction::Linear,
            appear: false,
        }
    }

    /// Sets the seconds to animate in.
    #[must_use]
    pub const fn enter(mut self, seconds: f32) -> Self {
        self.enter = seconds;
        self
    }

    /// Sets the seconds to animate out.
    #[must_use]
    pub const fn exit(mut self, seconds: f32) -> Self {
        self.exit = seconds;
        self
    }

    /// Sets the easing of [`Presence::value`].
    #[must_use]
    pub const fn ease(mut self, ease: EaseFunction) -> Self {
        self.ease = ease;
        self
    }

    /// Animates in on the region's first render too.
    #[must_use]
    pub const fn appear(mut self) -> Self {
        self.appear = true;
        self
    }

    fn presence(&self, phase: Phase, progress: f32) -> Presence {
        let progress = progress.clamp(0.0, 1.0);
        Presence {
            phase,
            progress,
            value: self.ease.sample_clamped(progress),
        }
    }
}

/// Where an entity with a [`Transition`] is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Animating in.
    Entering,
    /// Fully shown.
    #[default]
    Shown,
    /// Animating out; it despawns when this ends.
    Leaving,
}

/// How present an entity with a [`Transition`] is: 0 when hidden, 1 when
/// fully shown. Read it to animate anything: `$Ref<Presence>(#Name).value()`
/// in `rsn!`, or `Ref<Presence>` in `watch`, which also finds it from the
/// entity's descendants.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Presence {
    phase: Phase,
    progress: f32,
    value: f32,
}

impl Default for Presence {
    fn default() -> Self {
        Self {
            phase: Phase::Shown,
            progress: 1.0,
            value: 1.0,
        }
    }
}

impl Presence {
    /// The eased presence, from 0 (hidden) to 1 (shown). Some easings
    /// overshoot, so it can briefly leave that range.
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// The linear presence, from 0 (hidden) to 1 (shown).
    #[must_use]
    pub const fn progress(&self) -> f32 {
        self.progress
    }

    /// Whether it is entering, shown or leaving.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Linearly maps the eased value from `hidden` to `shown`:
    /// `presence.lerp(-40.0, 0.0)` slides from 40 px above into place.
    #[must_use]
    pub fn lerp(&self, hidden: f32, shown: f32) -> f32 {
        hidden + (shown - hidden) * self.value
    }
}

/// On region content that was removed and is animating out. Filter it out
/// with `Without<Leaving>` (to ignore clicks on a closing menu, say), or
/// observe `On<Add<Leaving>>` to start an animation of your own. It
/// despawns when [`Presence`] reaches 0.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Leaving;

/// Advances every [`Presence`] that is entering or leaving, and despawns
/// leaving entities one frame after they reach 0.
pub(super) fn animate(
    time: Option<Res<Time>>,
    mut entities: Query<(Entity, &Transition, &mut Presence)>,
    mut commands: Commands,
) {
    // Without a clock, transitions finish in one frame.
    let delta = time.map_or(f32::INFINITY, |time| time.delta_secs());
    for (entity, transition, mut presence) in &mut entities {
        let (seconds, sign) = match presence.phase {
            Phase::Shown => continue,
            // Done after a frame at 0, so the hidden end is shown too.
            Phase::Leaving if presence.progress <= 0.0 => {
                commands.entity(entity).despawn();
                continue;
            }
            Phase::Entering => (transition.enter, 1.0),
            Phase::Leaving => (transition.exit, -1.0),
        };
        let step = if seconds > 0.0 {
            delta / seconds
        } else {
            f32::INFINITY
        };
        let progress = (presence.progress + sign * step).clamp(0.0, 1.0);
        let phase = match presence.phase {
            Phase::Entering if progress >= 1.0 => Phase::Shown,
            phase => phase,
        };
        *presence = transition.presence(phase, progress);
    }
}

/// Why region content is being spawned.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    /// The region's first render: enters only with [`Transition::appear`].
    Initial,
    /// New content: enters.
    New,
    /// A keyed row whose item changed, rebuilt in place: shown at once.
    Replace,
}

/// Starts newly spawned region content: entering, or shown at once.
pub(super) fn start(world: &mut World, roots: &[Entity], start: Start) {
    for &root in roots {
        let Ok(mut entity) = world.get_entity_mut(root) else {
            continue;
        };
        let Some(&transition) = entity.get::<Transition>() else {
            continue;
        };
        let enters = match start {
            Start::Initial => transition.appear,
            Start::New => true,
            Start::Replace => false,
        };
        let presence = if enters && transition.enter > 0.0 {
            transition.presence(Phase::Entering, 0.0)
        } else {
            transition.presence(Phase::Shown, 1.0)
        };
        entity.insert(presence);
    }
}

/// Removes region content: roots with an exit transition start leaving,
/// the rest despawn now. Returns whether any root is left animating out.
pub(super) fn remove(world: &mut World, roots: &[Entity]) -> bool {
    let mut kept = false;
    for &root in roots {
        let Ok(mut entity) = world.get_entity_mut(root) else {
            continue;
        };
        match entity.get::<Transition>().copied() {
            Some(transition) if transition.exit > 0.0 => {
                let progress = entity.get::<Presence>().map_or(1.0, Presence::progress);
                entity.insert((transition.presence(Phase::Leaving, progress), Leaving));
                kept = true;
            }
            _ => entity.despawn(),
        }
    }
    kept
}

/// Despawns content at once, leaving or not.
pub(super) fn despawn(world: &mut World, roots: &[Entity]) {
    crate::shared::despawn_all(world, roots);
}

/// Whether every root still exists and is leaving, so the content can come
/// back by [`revive`].
pub(super) fn is_leaving(world: &World, roots: &[Entity]) -> bool {
    roots.iter().all(|&root| {
        world
            .get_entity(root)
            .is_ok_and(|entity| entity.contains::<Leaving>())
    })
}

/// Whether any root still exists.
pub(super) fn any_alive(world: &World, roots: &[Entity]) -> bool {
    roots.iter().any(|&root| world.get_entity(root).is_ok())
}

/// Turns leaving content around: it animates back in from where it is.
pub(super) fn revive(world: &mut World, roots: &[Entity]) {
    for &root in roots {
        let Ok(mut entity) = world.get_entity_mut(root) else {
            continue;
        };
        entity.remove::<Leaving>();
        if let (Some(&transition), Some(&presence)) =
            (entity.get::<Transition>(), entity.get::<Presence>())
        {
            entity.insert(transition.presence(Phase::Entering, presence.progress));
        }
    }
}
