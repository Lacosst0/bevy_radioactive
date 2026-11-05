#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate self as bevy_radioactive;

#[cfg(feature = "hooks")]
pub mod hooks;
#[cfg(feature = "reactive")]
pub mod reactive;
#[cfg(feature = "rfn")]
pub mod rfn;
#[cfg(any(feature = "reactive", feature = "hooks", feature = "view"))]
mod shared;
#[cfg(feature = "subscribe")]
pub mod subscribe;
#[cfg(feature = "view")]
pub mod view;

#[doc(hidden)]
pub mod macros {
    pub use bevy_radioactive_macros::*;
}
