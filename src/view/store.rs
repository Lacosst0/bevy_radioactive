//! Field-level stores: one [`Signal`] per field of a struct.

/// A struct split into one [`Signal`](super::Signal) per field, so a reader
/// of one field does not re-run when another field changes.
///
/// Derive it with `#[derive(Store)]` on a struct with named fields. The
/// derive generates `{Name}Store`, a struct with the same fields wrapped in
/// `Signal`s, plus:
///
/// - `{Name}Store::new(value)` and `From<{Name}>`, which create the signals;
/// - `get()` and `get_untracked()`, which clone the whole value back out
///   (`get` tracks every field);
/// - `set(value)`, which writes every field with
///   [`set_if_neq`](super::Signal::set_if_neq), notifying only the readers
///   of fields whose value changed;
/// - `update(f)`, which mutates a copy of the value and writes it back with
///   `set`.
///
/// Every field type must be `Clone + PartialEq + Send + Sync + 'static`.
/// The store is `Clone`; clones share the same signals.
///
/// ```ignore
/// #[derive(Clone, PartialEq, Store)]
/// struct Player {
///     name: String,
///     health: u32,
/// }
///
/// let player = PlayerStore::new(Player { name: "Ada".into(), health: 10 });
/// let health = player.health.clone();
/// view! {
///     // Patched when `health` changes, not when `name` does.
///     Text($(format!("HP {}", $health)))
/// }
/// ```
pub trait Store: Sized {
    /// The generated struct holding one signal per field.
    type Fields: Clone + Send + Sync + 'static;

    /// Moves every field into its own signal.
    fn into_store(self) -> Self::Fields;
}

/// Creates the per-field signals of the value `init` returns, like
/// [`signal`](super::signal) for a [`Store`].
pub fn store<T: Store>(init: impl FnOnce() -> T) -> T::Fields {
    init().into_store()
}
