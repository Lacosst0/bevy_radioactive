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
