//! Integration coverage for the `webby` `use_task` hook.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    coat::{Signal, signal},
    testing::settle_n,
    webby::{Cx, TaskState, component},
};

/// A future that finishes when its gate is opened, and counts its drops.
#[derive(Clone, Default)]
struct Gate {
    value: Arc<Mutex<(Option<u32>, Option<Waker>)>>,
    dropped: Arc<AtomicUsize>,
}

impl Gate {
    fn open(&self, value: u32) {
        let mut state = self.value.lock().unwrap();
        state.0 = Some(value);
        if let Some(waker) = state.1.take() {
            waker.wake();
        }
    }

    fn wait(&self) -> Waiting {
        Waiting(self.clone())
    }
}

struct Waiting(Gate);

impl Future for Waiting {
    type Output = u32;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        let mut state = self.0.value.lock().unwrap();
        if let Some(value) = state.0 {
            Poll::Ready(value)
        } else {
            state.1 = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
struct Shown(Option<u32>);

#[derive(Resource, Clone)]
struct Gates(Vec<Gate>);

#[component]
fn loader(cx: &Cx, id: Signal<usize>) -> impl Scene {
    let id = id.get();
    let gate = cx.untracked().resource::<Gates>().0[id].clone();
    let state = cx.use_task(id, move |_| async move { gate.wait().await * 10 });
    let value = state.ready();
    bsn! { Shown({value}) }
}

fn app(gates: usize) -> (App, Vec<Gate>) {
    let gates: Vec<Gate> = (0..gates).map(|_| Gate::default()).collect();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ))
    .insert_resource(Gates(gates.clone()));
    (app, gates)
}

fn shown(app: &mut App) -> Vec<Option<u32>> {
    app.world_mut()
        .query::<&Shown>()
        .iter(app.world())
        .map(|shown| shown.0)
        .collect()
}

#[test]
fn use_task_renders_pending_then_ready() {
    let (mut app, gates) = app(1);
    let id = signal(|| 0);
    app.world_mut()
        .spawn_scene(bsn! { @loader(id.clone()) })
        .unwrap();
    settle_n(&mut app, 2);
    assert_eq!(shown(&mut app), [None]);

    gates[0].open(4);
    settle_n(&mut app, 2);
    assert_eq!(shown(&mut app), [Some(40)]);
    assert_eq!(TaskState::Ready(1).ready(), Some(1));
    assert!(TaskState::<u32>::Pending.is_pending());
}

#[test]
fn changing_deps_cancels_the_running_task() {
    let (mut app, gates) = app(2);
    let id = signal(|| 0);
    app.world_mut()
        .spawn_scene(bsn! { @loader(id.clone()) })
        .unwrap();
    settle_n(&mut app, 2);

    id.set(1);
    settle_n(&mut app, 2);
    assert_eq!(
        gates[0].dropped.load(Ordering::Relaxed),
        1,
        "stale task dropped"
    );
    assert_eq!(shown(&mut app), [None]);

    // The stale task finishing late has no effect.
    gates[0].open(7);
    settle_n(&mut app, 2);
    assert_eq!(shown(&mut app), [None]);

    gates[1].open(2);
    settle_n(&mut app, 2);
    assert_eq!(shown(&mut app), [Some(20)]);
}

#[test]
fn despawning_the_host_cancels_the_task() {
    let (mut app, gates) = app(1);
    let id = signal(|| 0);
    let host = app
        .world_mut()
        .spawn_scene(bsn! { @loader(id.clone()) })
        .unwrap()
        .id();
    settle_n(&mut app, 2);
    assert_eq!(gates[0].dropped.load(Ordering::Relaxed), 0);

    app.world_mut().despawn(host);
    settle_n(&mut app, 2);
    assert_eq!(gates[0].dropped.load(Ordering::Relaxed), 1);
    gates[0].open(1);
    settle_n(&mut app, 2);
    assert_eq!(shown(&mut app), Vec::<Option<u32>>::new());
}
