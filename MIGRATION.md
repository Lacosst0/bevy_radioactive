# Migrating from 0.4 to 0.5

0.5 fixes leaks and crashes, makes bindings render the current value, adds effect
cleanups and derived components, and renames a few helpers. The old names still compile
with a deprecation warning for one release.

| 0.4 | 0.5 |
| --- | --- |
| `on_removed(source, handler)` | `watch_removed(source, handler)` (old name deprecated) |
| `bind_or(source, write, or_else)` | `bind_or_else(source, write, or_else)` (old name deprecated) |
| `reads.target()` | `reads.entity()` (old name deprecated); `cx.entity()` unchanged, now through `Deref` |
| `cx.use_reducer(init, reducer)` | `cx.use_reducer(reducer, init)`, as in React |
| `coat::effect(\|commands\| ..)` | `coat::effect(\|reads: &Reads, commands: &mut Commands\| ..)` |
| `bind` returns `impl Template` | `bind`/`bind_or_else` return `Bind<..>` (still a `Template` and `Scene`) |
| `Deps::Output: Copy` | `Deps::Output: Clone` |
| manual `let x = x.clone();` around `coat!` | not needed for signals read with `$` |

New: `watch_immediate`, `bind_deferred` and `Bind::deferred`, `computed` and
`computed_opt`, `Cleanup`/`IntoCleanup`, `Cx::on_unmount`, `either`/`Either`,
`#[entity]` component props, `Result<impl Scene>` component renders, `$&name` borrowed
signal reads in `coat!`, `ReactiveDiagnostics`, `ReactiveSet::Settled`, and the
`testing` module (`settle`, `settle_n`). Also new: presence-only reads
(`Reads::has`/`this_has`), hierarchy-inherited components (`Reads::inherited`),
`layout_effect` with `ReactiveSet::PostLayout`, `Selection`/`Selected` with
`SelectCommandsExt`/`SelectEntityCommandsExt`, and query monitors
(`Monitor`, `MonitorAppExt::add_monitor`). The prelude now also exports `ReactiveSet`,
`ReactiveSettings`, `ReactiveDiagnostics`, `Reads`, `Signal`, `signal`, `memo`,
`effect`, `coat!`, `coat_list!`, `Cx`, `either`, `computed` and the new helpers. It
does not export `#[component]`, which would clash with Bevy's `#[component(..)]` derive
helper; import it from `bevy_radioactive::webby`.

Behavior changes:

- `bind` and `bind_or_else` write the source's current value when the listener spawns
  (at the start of the next reactive pass), and `bind_or_else` calls the fallback when
  the source lacks the value. Use `bind_deferred` or `.deferred()` for the old
  change-only behavior. `watch` still only sees later changes; `watch_immediate`
  replays.
- `cx.use_effect` runs after the render's scene, including child components, has
  spawned; children's effects run before their parent's. An effect may return a
  cleanup (`move |commands: &mut Commands| ..`), which runs before it re-runs and when
  the component unmounts.
- `coat::effect` re-runs when ECS data it read through `Reads` changes, and may return
  a cleanup that runs before the next run and when its entity despawns.
- A `coat!` `match` region is keyed by the matched arm and the pattern bindings the arm
  uses. A change elsewhere in the scrutinee patches the arm in place instead of
  rebuilding it; a change to a used binding rebuilds it.
- Unkeyed `webby` children are matched by the call site of `@name(..)` and an
  occurrence count, not by position among all siblings. Inserting a conditional
  sibling no longer moves another component's state. Components created in a loop
  from one call site still match by position; key them.
- A component whose hooks change type at the same index (for example after a
  conditional hook) logs a warning and resets that hook instead of panicking.
- A tracked value is published only when something listens for it: a subscription, an
  entity observer on the source, or a global observer. `ReactiveDiagnostics` counts
  publications.
- Internal subscription entities are despawned when their source despawns, after the
  removal has been delivered. A raw `subscribe_to` of `ValueChanged<T>`/`ValueRemoved<T>`
  for an untracked `T` logs a warning.
- Hitting `ReactiveSettings::max_passes` logs a warning and increments
  `ReactiveDiagnostics::exhausted_frames`. Nested components and regions now settle in
  one pass, so deep trees no longer exhaust the limit.
- Reading or writing a `Signal` from inside its own `update`/`with` closure, or a memo
  from inside its own computation, panics with a message instead of deadlocking.
- `coat!` patched components mark the component changed only when a field actually
  changed.
- `coat::for_keyed` with a duplicate key logs a warning and despawns the displaced row
  instead of leaking it.
- `webby` instance trees are freed when their host despawns (the parent link is weak).
- The crate enables Bevy's `bevy_log` feature.
- `ReactiveSet` has a new `PostLayout` variant (runs `layout_effect`s after UI layout and
  transform propagation); exhaustive `match`es on `ReactiveSet` need an extra arm.
  `ReactiveDiagnostics::effects_run` also counts post-layout effects.

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
