//! Components computed from tracked reads.

use bevy::{
    ecs::{component::Mutable, template::Template},
    prelude::*,
    scene::{EmptySceneEffect, Scene},
};

use crate::runtime::{Reads, RunFn, effect_template, forget, tracked};

/// Keeps component `C` on the current BSN entity equal to `f(reads)`.
///
/// `f` runs when the runtime first sees the entity (the same frame, before
/// UI layout) and again whenever something it read changes: ECS data read
/// through [`Reads`], including a component appearing or disappearing, or a
/// signal or memo. `C` is inserted on the first run and then written with
/// `set_if_neq`, so equal results do not mark it changed. Reads of `C` on
/// the entity itself are not dependencies.
///
/// ```ignore
/// bsn! {
///     Button
///     computed(|r: &Reads| Text(r.resource::<Count>().map_or(0, |c| c.0).to_string()))
///     computed(|r: &Reads| BackgroundColor(if r.this::<Pressed>().is_some() { PRESSED } else { IDLE }))
/// }
/// ```
pub fn computed<C, F>(f: F) -> impl Template<Output = EmptySceneEffect> + Scene
where
    C: Component<Mutability = Mutable> + PartialEq,
    F: Fn(&Reads) -> C + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let (value, mut deps) = tracked(world, target, &f);
            forget::<C>(world, &mut deps, target);
            if let Ok(mut entity) = world.get_entity_mut(target) {
                if let Some(mut current) = entity.get_mut::<C>() {
                    current.set_if_neq(value);
                } else {
                    entity.insert(value);
                }
            }
            deps.into()
        })
    })
}

/// Like [`computed`], for a component that may be absent: `Some(c)` inserts
/// or replaces `C` when it differs, and `None` removes it.
///
/// Works for immutable components too, since `C` is always replaced rather
/// than mutated.
///
/// ```ignore
/// computed_opt(|r: &Reads| r.this::<Locked>().map(|_| InteractionDisabled))
/// ```
pub fn computed_opt<C, F>(f: F) -> impl Template<Output = EmptySceneEffect> + Scene
where
    C: Component + PartialEq,
    F: Fn(&Reads) -> Option<C> + Clone + Send + Sync + 'static,
{
    effect_template((), move |()| -> RunFn {
        let f = f.clone();
        Box::new(move |world: &mut World, target| {
            let (value, mut deps) = tracked(world, target, &f);
            forget::<C>(world, &mut deps, target);
            if let Ok(mut entity) = world.get_entity_mut(target) {
                match value {
                    Some(value) if entity.get::<C>() != Some(&value) => {
                        entity.insert(value);
                    }
                    None if entity.contains::<C>() => {
                        entity.remove::<C>();
                    }
                    _ => {}
                }
            }
            deps.into()
        })
    })
}
