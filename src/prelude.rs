//! Convenient imports for BSN code using `bevy_radioactive`.
//!
//! Brings in the watching, binding and subscription helpers, `computed`,
//! the runtime's settings and system sets, selections, monitors, and the
//! constructors of the `coat` and `webby` layers. The `#[component]` attribute is not included,
//! because it would make Bevy's `#[component(..)]` derive helper ambiguous;
//! import it from [`webby`] where components are defined.
//!
//! ```
//! use bevy::prelude::*;
//! use bevy_radioactive::prelude::*;
//!
//! #[derive(Component, Clone, Copy, PartialEq)]
//! #[component(immutable)]
//! struct Lit(bool);
//!
//! fn lamp() -> impl Scene {
//!     let on = signal(|| false);
//!     bsn! {
//!         computed_opt(move |_: &Reads| on.get().then_some(Lit(true)))
//!         effect(|_: &Reads, _: &mut Commands| {})
//!     }
//! }
//!
//! fn either_scene(cx: &Cx) -> impl Scene + use<> {
//!     either(cx.entity() == Entity::PLACEHOLDER, || bsn! { Node }, || bsn! { Text("x") })
//! }
//! ```

pub use crate::{
    RadioactivePlugin, Reaction, ReactiveAppExt, ReactiveDiagnostics, ReactiveSet,
    ReactiveSettings, Reads, Removal, Singleton, Subscription, SubscriptionAppExt, ValueChanged,
    ValueRemoved, bind, bind_deferred, bind_or_else, computed, computed_opt, subscribe_to, watch,
    watch_immediate, watch_removed,
};
#[allow(deprecated)]
pub use crate::{bind_or, on_removed};

pub use crate::coat::{
    CopySignal, Signal, SignalCommandsExt, Value, coat, coat_list, copy_signal, effect,
    layout_effect, memo, owns, signal,
};
pub use crate::monitor::{Monitor, MonitorAppExt};
pub use crate::select::{SelectCommandsExt, SelectEntityCommandsExt, Selected, Selection};
pub use crate::webby::{Cx, Either, TaskState, either};
#[cfg(feature = "widgets")]
pub use crate::widgets::{
    ControlledWidget, bind_checkbox, bind_radio, bind_slider, bind_tabs, bind_widget,
    bind_widget_with,
};
