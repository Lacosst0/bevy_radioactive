#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate self as bevy_radioactive;

#[cfg(feature = "hooks")]
pub mod hooks;
#[cfg(feature = "subscribe")]
pub mod subscribe;
#[cfg(feature = "view")]
pub mod view;
#[cfg(feature = "watch")]
pub mod watch;

#[doc(hidden)]
pub mod macros {
    pub use bevy_radioactive_macros::*;
}
