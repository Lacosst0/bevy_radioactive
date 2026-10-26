# Runtime benchmarks

`benches/runtime.rs` measures what one frame of the reactive runtime costs. Each
scenario spawns its effect hosts, settles for three frames, then times 200 runs of
`PostUpdate` (the schedule that holds `ReactiveSet::Publish` and
`ReactiveSet::PostLayout`) and reports the median, minimum and maximum. "effects" is
`ReactiveDiagnostics::effects_run` of the last frame.

```sh
cargo bench --bench runtime            # 200 frames per scenario
cargo bench --bench runtime -- --quick # 30 frames
```

Scenarios:

- **empty app**: no hosts; the fixed cost of the runtime's systems.
- **idle, N hosts x M signals / components / presence**: every host reads M signals,
  M mutable components on other entities (`Reads::get`), or M presence flags
  (`Reads::has`). Nothing changes.
- **idle, memo diamond**: one signal feeds two memos that feed a third, read by every
  host. Nothing changes.
- **idle, 10 chains x depth 100**: 1000 hosts nested 100 deep under `ChildOf`, each
  reading one signal. Nothing changes.
- **one signal changes**: every host reads its own signal; one signal is set per frame.
- **one component changes**: every host reads a component on its own entity; one
  component is mutated through `Mut<T>` per frame, outside the runtime.
- **fan-out**: one signal read by 1000 hosts and set every frame, so every host runs.

Numbers are microseconds per frame on an Intel Core i7-13650HX, Windows 11, Rust
1.99.0, release profile. They vary by a few percent between runs.

## Baseline: per-frame polling (0.5.0 before push invalidation)

Every main pass collected every host, sorted them by hierarchy depth (walking `ChildOf`
for each), and checked every dependency of every effect.

| scenario                                 |  median us |     min us |     max us | effects |
|------------------------------------------|------------|------------|------------|---------|
| empty app                                |        0.6 |        0.5 |       10.2 |       0 |
| idle, 1000 hosts x 4 signals             |       39.0 |       36.2 |       96.3 |       0 |
| idle, 10000 hosts x 1 signals            |      349.5 |      307.4 |      818.1 |       0 |
| idle, 1000 hosts x 4 components          |       52.4 |       50.2 |      105.0 |       0 |
| idle, 1000 hosts x 4 presence            |       42.9 |       40.9 |      141.9 |       0 |
| idle, memo diamond -> 1000 hosts         |       89.7 |       85.4 |      239.2 |       0 |
| idle, 10 chains x depth 100              |      509.1 |      497.3 |      780.3 |       0 |
| one signal changes, 1000 hosts           |       65.5 |       58.3 |      171.1 |       1 |
| one signal changes, 10000 hosts          |      773.1 |      642.9 |     1228.0 |       1 |
| one component changes, 1000 hosts        |       82.9 |       72.5 |      181.7 |       1 |
| one component changes, 10000 hosts       |      874.9 |      794.0 |     1196.3 |       1 |
| fan-out, 1 signal -> 1000 hosts          |      181.7 |      162.8 |      354.8 |    1000 |

Idle cost grows linearly with hosts times dependencies, and the depth sort makes deep
trees cost O(hosts x depth) per frame even when nothing changes.

## Push invalidation

Writes queue only the hosts that read what they wrote (see the performance model in
the README). Signals and memos mark their readers on `set`, `Insert`/`Remove` observers
mark presence and immutable-component readers, and the change ticks of the mutable
(entity, component) pairs and resources that some effect read are scanned once per
pass, since `Mut<T>` writes have no hook.

| scenario                                 | before (us) | after (us) | speedup |
|------------------------------------------|------------:|-----------:|--------:|
| empty app                                |         0.6 |        0.6 |      1x |
| idle, 1000 hosts x 4 signals             |        39.0 |        0.6 |     65x |
| idle, 10000 hosts x 1 signals            |       349.5 |        0.6 |    580x |
| idle, 1000 hosts x 4 components          |        52.4 |       17.1 |      3x |
| idle, 1000 hosts x 4 presence            |        42.9 |        0.8 |     54x |
| idle, memo diamond -> 1000 hosts         |        89.7 |        0.6 |    150x |
| idle, 10 chains x depth 100              |       509.1 |        0.6 |    850x |
| one signal changes, 1000 hosts           |        65.5 |        1.3 |     50x |
| one signal changes, 10000 hosts          |       773.1 |        1.4 |    550x |
| one component changes, 1000 hosts        |        82.9 |       13.7 |      6x |
| one component changes, 10000 hosts       |       874.9 |      134.2 |    6.5x |
| fan-out, 1 signal -> 1000 hosts          |       181.7 |      190.0 |   0.95x |

Medians; the full run:

| scenario                                 |  median us |     min us |     max us | effects |
|------------------------------------------|------------|------------|------------|---------|
| empty app                                |        0.6 |        0.6 |        5.0 |       0 |
| idle, 1000 hosts x 4 signals             |        0.6 |        0.5 |        1.2 |       0 |
| idle, 10000 hosts x 1 signals            |        0.6 |        0.5 |        1.9 |       0 |
| idle, 1000 hosts x 4 components          |       17.1 |       15.8 |       52.6 |       0 |
| idle, 1000 hosts x 4 presence            |        0.8 |        0.6 |        1.6 |       0 |
| idle, memo diamond -> 1000 hosts         |        0.6 |        0.6 |        1.6 |       0 |
| idle, 10 chains x depth 100              |        0.6 |        0.5 |        1.3 |       0 |
| one signal changes, 1000 hosts           |        1.3 |        1.2 |       10.5 |       1 |
| one signal changes, 10000 hosts          |        1.4 |        1.2 |        4.0 |       1 |
| one component changes, 1000 hosts        |       13.7 |       12.2 |      174.0 |       1 |
| one component changes, 10000 hosts       |      134.2 |      113.3 |      473.4 |       1 |
| fan-out, 1 signal -> 1000 hosts          |      190.0 |      172.2 |      359.0 |    1000 |

Idle frames with signal, memo and presence dependencies now cost the same as an empty
app at any host count or depth. Mutable-component dependencies remain O(distinct
(entity, component) pairs read) per pass, about 4 ns per pair, because Bevy has no hook
for `Mut<T>` writes; Bevy 0.20's `summary_tick` would let a scan skip unchanged
columns, but it is opt-in per component type, so user components rarely have it, and
the scan looks pairs up per entity anyway. Make a component `#[component(immutable)]`
(changed by re-inserting it) to move its readers onto observers and out of the scan.
When every host runs (fan-out), the cost is the effects themselves; a run that reads
the same sources as its previous run keeps its subscriptions instead of re-subscribing.
