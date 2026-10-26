//! Frame cost of the reactive runtime.
//!
//! Run with `cargo bench --bench runtime`. Each scenario spawns its hosts,
//! settles, then times `PostUpdate` (the schedule that holds the runtime)
//! and prints the median, minimum and maximum per frame. `--quick` runs
//! fewer frames. See `BENCHMARKS.md` for recorded numbers.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin, Reads,
    coat::{Signal, effect, memo},
    testing::settle_n,
};

#[derive(Component, Clone, Copy, Default, PartialEq)]
struct Value(u32);

#[derive(Component, Clone, Copy, Default)]
struct Marker;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ));
    app
}

/// Spawns one effect host, optionally as a child of `parent`.
fn host<F>(app: &mut App, parent: Option<Entity>, f: F) -> Entity
where
    F: Fn(&Reads) + Clone + Send + Sync + 'static,
{
    let mut entity = app
        .world_mut()
        .spawn_scene(bsn! { effect(move |r: &Reads, _: &mut Commands| f(r)) })
        .expect("host scene spawns");
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    entity.id()
}

struct Stats {
    median: Duration,
    min: Duration,
    max: Duration,
}

/// Times `frames` runs of `PostUpdate`, calling `before` (untimed) first.
fn measure(app: &mut App, frames: usize, mut before: impl FnMut(&mut App)) -> Stats {
    settle_n(app, 3);
    let mut samples = Vec::with_capacity(frames);
    for _ in 0..frames {
        before(app);
        let start = Instant::now();
        app.world_mut().run_schedule(PostUpdate);
        samples.push(start.elapsed());
        black_box(&*app);
    }
    samples.sort_unstable();
    Stats {
        median: samples[samples.len() / 2],
        min: samples[0],
        max: samples[samples.len() - 1],
    }
}

fn report(name: &str, stats: &Stats, effects: usize) {
    println!(
        "| {name:<40} | {:>10.1} | {:>10.1} | {:>10.1} | {effects:>7} |",
        micros(stats.median),
        micros(stats.min),
        micros(stats.max),
    );
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e6
}

fn effects_run(app: &App) -> usize {
    app.world()
        .resource::<bevy_radioactive::ReactiveDiagnostics>()
        .effects_run
}

/// An app with no hosts: the fixed cost of the runtime's systems.
fn empty(frames: usize) {
    let mut app = app();
    let stats = measure(&mut app, frames, |_| {});
    report("empty app", &stats, effects_run(&app));
}

/// `hosts` hosts reading `deps` signals each; nothing changes.
fn idle_signals(frames: usize, hosts: usize, deps: usize) {
    let mut app = app();
    let signals: Vec<Signal<u32>> = (0..deps).map(|_| Signal::new(0)).collect();
    for _ in 0..hosts {
        let signals = signals.clone();
        host(&mut app, None, move |_| {
            for signal in &signals {
                black_box(signal.get());
            }
        });
    }
    let stats = measure(&mut app, frames, |_| {});
    report(
        &format!("idle, {hosts} hosts x {deps} signals"),
        &stats,
        effects_run(&app),
    );
}

/// `hosts` hosts reading a mutable component on `deps` source entities
/// each; nothing changes.
fn idle_components(frames: usize, hosts: usize, deps: usize) {
    let mut app = app();
    let sources: Vec<Entity> = (0..hosts * deps)
        .map(|_| app.world_mut().spawn(Value(0)).id())
        .collect();
    for chunk in sources.chunks(deps) {
        let chunk = chunk.to_vec();
        host(&mut app, None, move |r| {
            for &source in &chunk {
                black_box(r.get::<Value>(source));
            }
        });
    }
    let stats = measure(&mut app, frames, |_| {});
    report(
        &format!("idle, {hosts} hosts x {deps} components"),
        &stats,
        effects_run(&app),
    );
}

/// `hosts` hosts with `deps` presence reads each; nothing changes.
fn idle_presence(frames: usize, hosts: usize, deps: usize) {
    let mut app = app();
    let sources: Vec<Entity> = (0..hosts * deps)
        .map(|_| app.world_mut().spawn(Marker).id())
        .collect();
    for chunk in sources.chunks(deps) {
        let chunk = chunk.to_vec();
        host(&mut app, None, move |r| {
            for &source in &chunk {
                black_box(r.has::<Marker>(source));
            }
        });
    }
    let stats = measure(&mut app, frames, |_| {});
    report(
        &format!("idle, {hosts} hosts x {deps} presence"),
        &stats,
        effects_run(&app),
    );
}

/// `hosts` hosts with their own signal each; one signal changes per frame.
fn single_change(frames: usize, hosts: usize) {
    let mut app = app();
    let signals: Vec<Signal<u32>> = (0..hosts).map(|_| Signal::new(0)).collect();
    for signal in &signals {
        let signal = signal.clone();
        host(&mut app, None, move |_| {
            black_box(signal.get());
        });
    }
    let mut next = 0;
    let stats = measure(&mut app, frames, |_| {
        signals[next % hosts].update(|value| *value += 1);
        next += 1;
    });
    report(
        &format!("one signal changes, {hosts} hosts"),
        &stats,
        effects_run(&app),
    );
}

/// `hosts` hosts reading a component on their own source each; a system
/// outside the runtime mutates one source per frame through `Mut<T>`.
fn single_component_change(frames: usize, hosts: usize) {
    let mut app = app();
    let sources: Vec<Entity> = (0..hosts)
        .map(|_| app.world_mut().spawn(Value(0)).id())
        .collect();
    for &source in &sources {
        host(&mut app, None, move |r| {
            black_box(r.get::<Value>(source));
        });
    }
    let mut next = 0;
    let stats = measure(&mut app, frames, |app| {
        let source = sources[next % hosts];
        app.world_mut().get_mut::<Value>(source).unwrap().0 += 1;
        next += 1;
    });
    report(
        &format!("one component changes, {hosts} hosts"),
        &stats,
        effects_run(&app),
    );
}

/// `chains` chains of `depth` nested hosts reading one signal each;
/// nothing changes.
fn deep_tree(frames: usize, chains: usize, depth: usize) {
    let mut app = app();
    let signal = Signal::new(0_u32);
    for _ in 0..chains {
        let mut parent = None;
        for _ in 0..depth {
            let signal = signal.clone();
            parent = Some(host(&mut app, parent, move |_| {
                black_box(signal.get());
            }));
        }
    }
    let stats = measure(&mut app, frames, |_| {});
    report(
        &format!("idle, {chains} chains x depth {depth}"),
        &stats,
        effects_run(&app),
    );
}

/// One signal read by `hosts` hosts, changed every frame: every host runs.
fn fan_out(frames: usize, hosts: usize) {
    let mut app = app();
    let signal = Signal::new(0_u32);
    for _ in 0..hosts {
        let signal = signal.clone();
        host(&mut app, None, move |_| {
            black_box(signal.get());
        });
    }
    let stats = measure(&mut app, frames, |_| signal.update(|value| *value += 1));
    report(
        &format!("fan-out, 1 signal -> {hosts} hosts"),
        &stats,
        effects_run(&app),
    );
}

/// One signal behind a diamond of memos, read by `hosts` hosts.
fn memo_diamond(frames: usize, hosts: usize) {
    let mut app = app();
    let source = Signal::new(0_u32);
    let left = memo({
        let source = source.clone();
        move || source.get() + 1
    });
    let right = memo({
        let source = source.clone();
        move || source.get() * 2
    });
    let sum = memo(move || left.get() + right.get());
    for _ in 0..hosts {
        let sum = sum.clone();
        host(&mut app, None, move |_| {
            black_box(sum.get());
        });
    }
    let stats = measure(&mut app, frames, |_| {});
    report(
        &format!("idle, memo diamond -> {hosts} hosts"),
        &stats,
        effects_run(&app),
    );
}

fn main() {
    let quick = std::env::args().any(|arg| arg == "--quick");
    let frames = if quick { 30 } else { 200 };
    println!(
        "| {:<40} | {:>10} | {:>10} | {:>10} | {:>7} |",
        "scenario", "median us", "min us", "max us", "effects"
    );
    println!(
        "|{:-<42}|{:->12}|{:->12}|{:->12}|{:->9}|",
        "", "", "", "", ""
    );
    empty(frames);
    idle_signals(frames, 1_000, 4);
    idle_signals(frames, 10_000, 1);
    idle_components(frames, 1_000, 4);
    idle_presence(frames, 1_000, 4);
    memo_diamond(frames, 1_000);
    deep_tree(frames, 10, 100);
    single_change(frames, 1_000);
    single_change(frames, 10_000);
    single_component_change(frames, 1_000);
    single_component_change(frames, 10_000);
    fan_out(frames, 1_000);
}
