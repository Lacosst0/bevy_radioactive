# Migrating from 0.3 to 0.4

Resources are singleton components in Bevy 0.20, so resource and component APIs are
unified. Event subscriptions now support multiple sources per listener.

| 0.3 | 0.4 |
| --- | --- |
| `app.track_resource::<T>()` / `app.track_component::<T>()` | `app.track::<T>()` |
| `app.observe_resource(o)` / `app.observe_component(o)` | `app.observe_value(o)` |
| `watch_resource(handler)` | `watch(Singleton, handler)` |
| `watch_component(#Source, handler)` | `watch(#Source, handler)` |
| one source per event type per listener | any number; also `[#A, #B]` |
| publishers in `Last` | runtime in `PostUpdate`, before UI layout |

New: `bind`, `bind_or`, `on_removed`, `ValueRemoved<T>`, `Removal<T>`,
`RadioactivePlugin`, `ReactiveSettings`, the `webby` module (React-style components and
hooks), and the `coat` module (`coat!` templates and signals, in the new
`bevy_radioactive_macros` crate).

Behavior changes:

- `ReactiveSet::Publish` moved from `Last` to `PostUpdate`. Systems in `Last` that
  mutated watched values before publication must move earlier, or accept next-frame
  delivery.
- The same type may no longer be rejected for being both a resource and a component;
  a resource is just the singleton entity holding `T`, and `Singleton` selects it.
- BSN helpers place each subscription on an internal entity linked to the listener by
  `SubscriptionOf`. `SubscriptionEvent::target()` still reports the listener;
  `Subscription::observer()` now reports the internal subscription entity's observer.
- `app.track::<T>()` is optional: `watch`, `bind` and `on_removed` track their type on
  first use. Without `RadioactivePlugin` they report a scene error naming the plugin.

Older guides (0.1 → 0.2 and earlier) are in the git history.
