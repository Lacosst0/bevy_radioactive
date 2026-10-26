#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate self as bevy_radioactive;

pub mod coat;
pub mod computed;
/// Convenient imports for common reactive BSN use cases.
pub mod prelude;
/// Component and resource change publication plus BSN state watching.
pub mod reactivity;
pub(crate) mod runtime;
#[doc(hidden)]
pub mod macros {
    pub use bevy_radioactive_macros::*;
}
pub mod monitor;
pub mod select;
/// Source-preserving entity-event subscriptions backed by relationships.
pub mod subscription;
pub mod testing;
pub mod webby;
#[cfg(feature = "widgets")]
pub mod widgets;

pub use computed::{computed, computed_opt};
pub use reactivity::{
    Bind, BindFallback, OrElse, RadioactivePlugin, Reaction, ReactiveAppExt, ReactiveDiagnostics,
    ReactivePublish, ReactiveSet, ReactiveSettings, Removal, Singleton, ValueChanged, ValueRemoved,
    Watch, WatchRemoval, bind, bind_deferred, bind_or_else, watch, watch_immediate, watch_removed,
};
#[allow(deprecated)]
pub use reactivity::{bind_or, on_removed};
pub use runtime::{Cleanup, Effects, IntoCleanup, Reads};
pub use subscription::{
    Source, Subscribe, SubscribedTo, Subscribers, Subscription, SubscriptionAppExt,
    SubscriptionEvent, SubscriptionOf, Subscriptions, subscribe_to,
};
