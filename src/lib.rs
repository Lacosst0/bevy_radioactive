#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate self as bevy_radioactive;

pub mod coat;
/// Convenient imports for common reactive BSN use cases.
pub mod prelude;
/// Component and resource change publication plus BSN state watching.
pub mod reactivity;
pub(crate) mod runtime;
#[doc(hidden)]
pub mod macros {
    pub use bevy_radioactive_macros::*;
}
/// Source-preserving entity-event subscriptions backed by relationships.
pub mod subscription;
pub mod webby;

pub use reactivity::{
    RadioactivePlugin, Reaction, ReactiveAppExt, ReactivePublish, ReactiveSet, ReactiveSettings,
    Removal, Singleton, ValueChanged, ValueRemoved, Watch, WatchRemoval, bind, bind_or, on_removed,
    watch,
};
pub use subscription::{
    Source, Subscribe, SubscribedTo, Subscribers, Subscription, SubscriptionAppExt,
    SubscriptionEvent, SubscriptionOf, Subscriptions, subscribe_to,
};
