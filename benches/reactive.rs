//! Frame-time benchmarks for `reactive`: large keyed lists and many watchers.
//!
//! `cargo bench --bench reactive` prints the median time of each case.
//! Pass a number to scale the cases (default 1000 rows or watchers), and a
//! name fragment to run only matching cases:
//! `cargo bench --bench reactive -- 5000 edit`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    reason = "benchmark setup"
)]

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Debug, Default, PartialEq)]
struct Task {
    title: String,
    done: bool,
}

#[derive(Component, Clone, Copy, Default)]
struct Count(i32);

#[derive(Component, Clone, Copy, Default)]
struct Noise(u32);

#[derive(Resource, Clone, Copy, Default)]
struct Score(u32);

#[derive(Component, Clone, Default)]
struct Label(String);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Score>();
    app
}

fn spawn_tasks(app: &mut App, n: usize) -> Vec<Entity> {
    (0..n)
        .map(|i| {
            app.world_mut()
                .spawn(Task {
                    title: format!("task {i}"),
                    done: false,
                })
                .id()
        })
        .collect()
}

/// Unrelated entities, so scans can't get away with tiny worlds.
fn spawn_noise(app: &mut App, n: usize) -> Vec<Entity> {
    (0..n)
        .map(|i| app.world_mut().spawn(Noise(i as u32)).id())
        .collect()
}

fn edit(app: &mut App, tasks: &[Entity], i: usize) {
    let task = tasks[i % tasks.len()];
    app.world_mut().get_mut::<Task>(task).unwrap().title = format!("edited {i}");
}

/// `for task in $Query<&Task> { .. }`: rows hold a copy of the item.
fn value_rows(n: usize) -> (App, Vec<Entity>) {
    let mut app = app();
    let tasks = spawn_tasks(&mut app, n);
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                for task in $Query<&Task> {
                    Label({ format!("{} {}", task.title, task.done) })
                }
            ]
        })
        .unwrap();
    app.update();
    (app, tasks)
}

/// `for task in $Query<Entity, With<Task>> { .. $Ref<Task>(task) .. }`:
/// rows update in place.
fn entity_rows(n: usize) -> (App, Vec<Entity>) {
    let mut app = app();
    let tasks = spawn_tasks(&mut app, n);
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                for task in $Query<Entity, With<Task>> {
                    Label({ $Ref<Task>(task).title.clone() })
                }
            ]
        })
        .unwrap();
    app.update();
    (app, tasks)
}

/// `n` watchers reading `Count` from an ancestor `depth` levels up.
fn nearest_watchers(n: usize, depth: usize) -> (App, Entity) {
    let mut app = app();
    let root = app.world_mut().spawn(Count(0)).id();
    for _ in 0..n {
        let mut parent = root;
        for _ in 0..depth {
            parent = app.world_mut().spawn(ChildOf(parent)).id();
        }
        app.world_mut()
            .spawn_scene(bsn! {
                Label
                ChildOf({parent})
                watch(|count: Ref<Count>| Label(count.0.to_string()))
            })
            .unwrap();
    }
    app.update();
    (app, root)
}

/// `watchers` closures each reading all of `n` tasks with `$Query`.
fn query_watchers(n: usize, watchers: usize) -> (App, Vec<Entity>) {
    let mut app = app();
    let tasks = spawn_tasks(&mut app, n);
    for _ in 0..watchers {
        app.world_mut()
            .spawn_scene(rsn! {
                Label({ format!("{} open", $Query<&Task>.iter().filter(|t| !t.done).count()) })
            })
            .unwrap();
    }
    app.update();
    (app, tasks)
}

fn res_watchers(n: usize) -> App {
    let mut app = app();
    for _ in 0..n {
        app.world_mut()
            .spawn_scene(rsn! { Label({ $Res<Score>.0.to_string() }) })
            .unwrap();
    }
    app.update();
    app
}

/// Median of `iterations` runs of `f`.
fn measure(iterations: usize, mut f: impl FnMut(usize)) -> Duration {
    for i in 0..3 {
        f(i);
    }
    let mut times: Vec<Duration> = (0..iterations)
        .map(|i| {
            let start = Instant::now();
            f(i + 3);
            start.elapsed()
        })
        .collect();
    times.sort();
    times[times.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let n: usize = args.iter().find_map(|a| a.parse().ok()).unwrap_or(1000);
    let filter: Vec<&String> = args
        .iter()
        .filter(|a| a.parse::<usize>().is_err())
        .collect();
    let run = |name: &str| filter.is_empty() || filter.iter().any(|f| name.contains(f.as_str()));
    let report =
        |name: &str, time: Duration| println!("{name:<44} {:>10.1} µs", time.as_secs_f64() * 1e6);
    let iterations = 30;

    println!("n = {n}");

    if run("value_rows/spawn") {
        report(
            "value_rows/spawn",
            measure(5, |_| drop(black_box(value_rows(n)))),
        );
    }
    if run("value_rows/idle") {
        let (mut app, _) = value_rows(n);
        spawn_noise(&mut app, n * 10);
        report("value_rows/idle", measure(iterations, |_| app.update()));
    }
    if run("value_rows/edit_one") {
        let (mut app, tasks) = value_rows(n);
        report(
            "value_rows/edit_one",
            measure(iterations, |i| {
                edit(&mut app, &tasks, i);
                app.update();
            }),
        );
    }
    if run("entity_rows/spawn") {
        report(
            "entity_rows/spawn",
            measure(5, |_| drop(black_box(entity_rows(n)))),
        );
    }
    if run("entity_rows/idle") {
        let (mut app, _) = entity_rows(n);
        report("entity_rows/idle", measure(iterations, |_| app.update()));
    }
    if run("entity_rows/edit_one") {
        let (mut app, tasks) = entity_rows(n);
        report(
            "entity_rows/edit_one",
            measure(iterations, |i| {
                edit(&mut app, &tasks, i);
                app.update();
            }),
        );
    }
    if run("entity_rows/add_one") {
        let (mut app, _) = entity_rows(n);
        report(
            "entity_rows/add_one",
            measure(iterations, |_| {
                spawn_tasks(&mut app, 1);
                app.update();
            }),
        );
    }
    if run("nearest_watchers/idle") {
        let (mut app, _) = nearest_watchers(n, 8);
        report(
            "nearest_watchers/idle (depth 8)",
            measure(iterations, |_| app.update()),
        );
    }
    if run("nearest_watchers/change") {
        let (mut app, root) = nearest_watchers(n, 8);
        report(
            "nearest_watchers/change (depth 8)",
            measure(iterations, |i| {
                app.world_mut().get_mut::<Count>(root).unwrap().0 = i as i32;
                app.update();
            }),
        );
    }
    if run("query_watchers/idle") {
        let (mut app, _) = query_watchers(n, 100);
        spawn_noise(&mut app, n * 10);
        report(
            "query_watchers/idle (100 watchers)",
            measure(iterations, |_| app.update()),
        );
    }
    if run("query_watchers/edit_one") {
        let (mut app, tasks) = query_watchers(n, 100);
        report(
            "query_watchers/edit_one (100 watchers)",
            measure(iterations, |i| {
                edit(&mut app, &tasks, i);
                app.update();
            }),
        );
    }
    if run("query_watchers/edit_noise") {
        let (mut app, _) = query_watchers(n, 100);
        let noise = spawn_noise(&mut app, n * 10);
        report(
            "query_watchers/edit_noise (100 watchers)",
            measure(iterations, |i| {
                app.world_mut()
                    .get_mut::<Noise>(noise[i % noise.len()])
                    .unwrap()
                    .0 += 1;
                app.update();
            }),
        );
    }
    if run("res_watchers/idle") {
        let mut app = res_watchers(n);
        report("res_watchers/idle", measure(iterations, |_| app.update()));
    }
    if run("res_watchers/change") {
        let mut app = res_watchers(n);
        report(
            "res_watchers/change",
            measure(iterations, |i| {
                app.world_mut().resource_mut::<Score>().0 = i as u32;
                app.update();
            }),
        );
    }
    if run("baseline/empty_app") {
        let mut app = app();
        report("baseline/empty_app", measure(iterations, |_| app.update()));
    }
}
