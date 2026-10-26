//! Two-way binding for `bevy_ui_widgets` controlled widgets (feature
//! `widgets`).
//!
//! Headless widgets are controlled: a slider does not move itself. It emits
//! a `ValueChange<T>` request, and the app writes the state component
//! (`SliderValue`, `Checked`, `SelectedTab`, ...). A binding does both
//! halves for one widget entity:
//!
//! - **model → widget**: an effect writes the widget's state component from
//!   the model whenever the model changes, and once when the widget spawns;
//! - **widget → model**: an observer on the widget consumes its
//!   `ValueChange` and writes the model.
//!
//! The model is a [`Signal`] or any ECS state, through
//! [`bind_widget_with`]. Writes on both sides skip equal values, so a change
//! does not echo back and forth.
//!
//! ```ignore
//! use bevy::ui_widgets::{Checkbox, Slider};
//! use bevy_radioactive::{coat::signal, widgets::*};
//!
//! let volume = signal(|| 0.5);
//! bsn! {
//!     Node Children [
//!         // a `Signal<f32>`
//!         Slider bind_slider(volume.clone())
//!         --
//!         // a resource, written only when the drag ends
//!         Slider
//!         bind_widget_with::<Slider, _, _>(
//!             |r: &Reads| r.resource::<Volume>().map(|v| v.0),
//!             |world: &mut World, v| world.resource_mut::<Volume>().0 = v,
//!         ).commit_on_final()
//!         --
//!         Checkbox bind_checkbox(muted)
//!     ]
//! }
//! ```
//!
//! Do not also add the widget's `*_self_update` observer (such as
//! `slider_self_update`), or its state is written twice.

use std::marker::PhantomData;

use bevy::{
    ecs::template::{Template, TemplateContext},
    prelude::*,
    scene::{EmptySceneEffect, ResolveContext, ResolveSceneError, ResolvedScene, Scene},
    ui::Checked,
    ui_widgets::{
        Checkbox, RadioButton, RadioGroup, SelectedTab, Slider, SliderValue, TabList, ValueChange,
    },
};

use crate::{
    coat::Signal,
    runtime::{Reads, attach, ensure_runtime, tracked},
};

/// A controlled widget: the value it edits, the `ValueChange` payload it
/// emits, and how to write its state component.
///
/// Implemented for [`Slider`] (`f32`), [`Checkbox`] (`bool`), [`TabList`]
/// (`Option<Entity>`, the selected tab) and [`RadioGroup`]
/// (`Option<Entity>`, the checked button). Implement it for other widgets
/// that follow the same pattern.
pub trait ControlledWidget: Component {
    /// The value the widget edits.
    type Value: Clone + PartialEq + Send + Sync + 'static;

    /// Payload of the `ValueChange` events the widget emits.
    type Change: Clone + Send + Sync + 'static;

    /// The value a requested change asks for.
    fn requested(change: &Self::Change) -> Self::Value;

    /// Writes `value` into the widget's state on `entity`, if it differs.
    fn write_state(entity: &mut EntityWorldMut, value: &Self::Value);

    /// Records reads the state write depends on besides the model, such as
    /// the children of a radio group. Does nothing by default.
    fn track(reads: &Reads, entity: Entity) {
        let _ = (reads, entity);
    }
}

impl ControlledWidget for Slider {
    type Value = f32;
    type Change = f32;

    fn requested(change: &f32) -> f32 {
        *change
    }

    fn write_state(entity: &mut EntityWorldMut, value: &f32) {
        if entity.get::<SliderValue>() != Some(&SliderValue(*value)) {
            entity.insert(SliderValue(*value));
        }
    }
}

impl ControlledWidget for Checkbox {
    type Value = bool;
    type Change = bool;

    fn requested(change: &bool) -> bool {
        *change
    }

    fn write_state(entity: &mut EntityWorldMut, value: &bool) {
        set_checked(entity, *value);
    }
}

impl ControlledWidget for TabList {
    type Value = Option<Entity>;
    type Change = Option<Entity>;

    fn requested(change: &Option<Entity>) -> Option<Entity> {
        *change
    }

    fn write_state(entity: &mut EntityWorldMut, value: &Option<Entity>) {
        if entity.get::<SelectedTab>().map(|tab| tab.0) != Some(*value) {
            entity.insert(SelectedTab(*value));
        }
    }
}

impl ControlledWidget for RadioGroup {
    type Value = Option<Entity>;
    type Change = Entity;

    fn requested(change: &Entity) -> Option<Entity> {
        Some(*change)
    }

    fn write_state(entity: &mut EntityWorldMut, value: &Option<Entity>) {
        let Some(children) = entity.get::<Children>().map(|children| children.to_vec()) else {
            return;
        };
        entity.world_scope(|world| {
            for child in children {
                if let Ok(mut button) = world.get_entity_mut(child)
                    && button.contains::<RadioButton>()
                {
                    set_checked(&mut button, *value == Some(child));
                }
            }
        });
    }

    fn track(reads: &Reads, entity: Entity) {
        // Buttons added later get their state too.
        let _ = reads.get::<Children>(entity);
    }
}

fn set_checked(entity: &mut EntityWorldMut, checked: bool) {
    match (checked, entity.contains::<Checked>()) {
        (true, false) => {
            entity.insert(Checked);
        }
        (false, true) => {
            entity.remove::<Checked>();
        }
        _ => {}
    }
}

/// State a widget is bound to: read with tracking, written on change
/// requests. Implemented for [`Signal`] and for the closures of
/// [`bind_widget_with`].
pub trait WidgetModel<V>: Clone + Send + Sync + 'static {
    /// The current value, or `None` to leave the widget's state alone.
    fn read(&self, reads: &Reads) -> Option<V>;

    /// Stores a value the widget asked for.
    fn write(&self, world: &mut World, value: V);
}

impl<V: Clone + PartialEq + Send + Sync + 'static> WidgetModel<V> for Signal<V> {
    fn read(&self, _reads: &Reads) -> Option<V> {
        Some(self.get())
    }

    fn write(&self, _world: &mut World, value: V) {
        self.set_if_neq(value);
    }
}

/// A [`WidgetModel`] made of a tracked reader and a writer, built by
/// [`bind_widget_with`].
#[derive(Clone)]
pub struct Lens<R, W> {
    read: R,
    write: W,
}

impl<V, R, W> WidgetModel<V> for Lens<R, W>
where
    R: Fn(&Reads) -> Option<V> + Clone + Send + Sync + 'static,
    W: Fn(&mut World, V) + Clone + Send + Sync + 'static,
{
    fn read(&self, reads: &Reads) -> Option<V> {
        (self.read)(reads)
    }

    fn write(&self, world: &mut World, value: V) {
        (self.write)(world, value);
    }
}

/// BSN template returned by [`bind_widget`] and its shorthands.
pub struct BindWidget<W, M> {
    model: M,
    commit_on_final: bool,
    marker: PhantomData<fn() -> W>,
}

impl<W, M> BindWidget<W, M> {
    /// Writes the model only when an interaction ends (`is_final`), such
    /// as when a slider drag is released. Intermediate values move the
    /// widget itself without touching the model.
    #[must_use]
    pub fn commit_on_final(mut self) -> Self {
        self.commit_on_final = true;
        self
    }
}

/// Binds widget `W` on the current BSN entity to `model` in both
/// directions.
///
/// The widget's state follows the model, starting with the model's value
/// when the widget spawns. Each `ValueChange` the widget emits writes the
/// model, which then writes the widget's state, as a controlled widget
/// expects.
///
/// ```ignore
/// let volume = signal(|| 0.5);
/// bsn! { Slider bind_widget::<Slider, _>(volume) }
/// ```
pub fn bind_widget<W, M>(model: M) -> BindWidget<W, M>
where
    W: ControlledWidget,
    M: WidgetModel<W::Value>,
{
    BindWidget {
        model,
        commit_on_final: false,
        marker: PhantomData,
    }
}

/// Like [`bind_widget`], for state read through tracked [`Reads`] and
/// written with world access, such as a resource or another entity's
/// component.
///
/// ```ignore
/// Checkbox
/// bind_widget_with::<Checkbox, _, _>(
///     |r: &Reads| r.resource::<Settings>().map(|s| s.vsync),
///     |world: &mut World, on| world.resource_mut::<Settings>().vsync = on,
/// )
/// ```
pub fn bind_widget_with<W, R, F>(read: R, write: F) -> BindWidget<W, Lens<R, F>>
where
    W: ControlledWidget,
    R: Fn(&Reads) -> Option<W::Value> + Clone + Send + Sync + 'static,
    F: Fn(&mut World, W::Value) + Clone + Send + Sync + 'static,
{
    bind_widget(Lens { read, write })
}

/// [`bind_widget`] for a [`Slider`]: `bsn! { Slider bind_slider(volume) }`.
pub fn bind_slider<M: WidgetModel<f32>>(model: M) -> BindWidget<Slider, M> {
    bind_widget(model)
}

/// [`bind_widget`] for a [`Checkbox`]: `bsn! { Checkbox bind_checkbox(muted) }`.
pub fn bind_checkbox<M: WidgetModel<bool>>(model: M) -> BindWidget<Checkbox, M> {
    bind_widget(model)
}

/// [`bind_widget`] for a [`TabList`], bound to the selected tab entity.
pub fn bind_tabs<M: WidgetModel<Option<Entity>>>(model: M) -> BindWidget<TabList, M> {
    bind_widget(model)
}

/// [`bind_widget`] for a [`RadioGroup`], bound to the checked button
/// entity.
pub fn bind_radio<M: WidgetModel<Option<Entity>>>(model: M) -> BindWidget<RadioGroup, M> {
    bind_widget(model)
}

impl<W, M> Template for BindWidget<W, M>
where
    W: ControlledWidget,
    M: WidgetModel<W::Value>,
{
    type Output = EmptySceneEffect;

    fn build_template(&self, context: &mut TemplateContext) -> Result<EmptySceneEffect> {
        ensure_runtime(context)?;
        let model = self.model.clone();
        attach(
            context,
            Box::new(move |world: &mut World, target| {
                let (value, deps) = tracked(world, target, |reads| {
                    W::track(reads, target);
                    model.read(reads)
                });
                if let Some(value) = value
                    && let Ok(mut entity) = world.get_entity_mut(target)
                {
                    W::write_state(&mut entity, &value);
                }
                deps.into()
            }),
        );
        let model = self.model.clone();
        let commit_on_final = self.commit_on_final;
        context.entity.observe(
            move |change: On<ValueChange<W::Change>>, mut commands: Commands| {
                let value = W::requested(&change.value);
                let (source, preview) = (change.source, commit_on_final && !change.is_final);
                let model = model.clone();
                commands.queue(move |world: &mut World| {
                    if preview {
                        if let Ok(mut entity) = world.get_entity_mut(source) {
                            W::write_state(&mut entity, &value);
                        }
                    } else {
                        model.write(world, value);
                    }
                });
            },
        );
        Ok(EmptySceneEffect)
    }

    fn clone_template(&self) -> Self {
        Self {
            model: self.model.clone(),
            commit_on_final: self.commit_on_final,
            marker: PhantomData,
        }
    }
}

impl<W, M> Scene for BindWidget<W, M>
where
    W: ControlledWidget,
    M: WidgetModel<W::Value>,
{
    fn resolve(
        self,
        _context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        scene.push_template(self);
        Ok(())
    }
}
