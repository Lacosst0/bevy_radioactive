#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate self as bevy_radioactive;

/// Convenient imports for common reactive BSN use cases.
pub mod prelude;
/// Component and resource change publication plus BSN state watching.
pub mod reactivity;
/// Source-preserving entity-event subscriptions backed by relationships.
pub mod subscription;

pub use reactivity::{
    Reaction, ReactiveAppExt, ReactiveSet, ValueChanged, WatchResource, watch_component,
    watch_resource,
};
pub use subscription::{SubscribedTo, Subscribers, Subscription, SubscriptionEvent, subscribe_to};
