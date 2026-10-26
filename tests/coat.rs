//! Integration coverage for `coat!` templates and signals.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    coat::{Reads, Signal, coat, effect, memo, signal},
};

#[derive(Component, Clone, Default)]
struct Label(u32);

#[derive(Component, Clone, Default)]
struct Root;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Home,
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    id: u32,
    name: &'static str,
}

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

fn texts(app: &mut App) -> Vec<(u32, String)> {
    let mut texts: Vec<_> = app
        .world_mut()
        .query::<(&Label, &Text)>()
        .iter(app.world())
        .map(|(label, text)| (label.0, text.0.clone()))
        .collect();
    texts.sort();
    texts
}

fn labels(app: &mut App) -> Vec<(u32, Entity)> {
    let mut labels: Vec<_> = app
        .world_mut()
        .query::<(Entity, &Label)>()
        .iter(app.world())
        .map(|(entity, label)| (label.0, entity))
        .collect();
    labels.sort();
    labels
}

#[test]
fn reactive_if_and_patched_text() {
    let open = signal(|| true);
    let count = signal(|| 1);
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            Label(0)
            Text($(format!("count {}", $count)))
            --
            if $open {
                Label(1) Text("open")
            } else {
                Label(2) Text("closed")
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "count 1".into()), (1, "open".into())]
    );

    count.set(5);
    open.set(false);
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "count 5".into()), (2, "closed".into())]
    );
}

#[test]
fn keyed_for_reuses_unchanged_rows() {
    let rows = signal(|| vec![Row { id: 1, name: "a" }, Row { id: 2, name: "b" }]);
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            #[key(row.id)]
            for row in $rows {
                Label({row.id}) Text({row.name})
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    let before = labels(&mut app);

    rows.set(vec![
        Row { id: 2, name: "b" },
        Row { id: 3, name: "c" },
        Row { id: 1, name: "a" },
    ]);
    app.update();
    let after = labels(&mut app);
    assert_eq!(after.len(), 3);
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1], before[1]);
    assert_eq!(
        texts(&mut app),
        vec![(1, "a".into()), (2, "b".into()), (3, "c".into())]
    );
}

#[test]
fn reactive_match_and_memo() {
    let tab = signal(|| Tab::Home);
    let title = {
        let tab = tab.clone();
        memo(move || format!("{:?}", tab.get()))
    };
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            Label(0) Text($(title.get()))
            --
            match $tab {
                Tab::Home => Label(1) Text("home"),
                Tab::Settings => { Label(2) Text("settings") },
            }
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "Home".into()), (1, "home".into())]
    );

    tab.set(Tab::Settings);
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "Settings".into()), (2, "settings".into())]
    );
}

#[test]
fn static_control_flow_and_let() {
    let show = true;
    let items = [4_u32, 5];
    let mut app = app();
    app.world_mut()
        .spawn_scene(coat! {
            Root
            Children [
                let base = 10_u32;
                if show { Label(1) Text("shown") } else { Label(2) Text("hidden") }
                --
                for item in items { Label({base + item}) Text("item") }
            ]
        })
        .unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![
            (1, "shown".into()),
            (14, "item".into()),
            (15, "item".into())
        ]
    );
}

#[test]
fn signal_methods() {
    let count: Signal<i32> = Signal::new(1);
    count.increment();
    count.increment();
    count.decrement();
    assert_eq!(count.get(), 2);
    assert!(!count.set_if_neq(2));
    assert!(count.set_if_neq(3));
    assert_eq!(count.with(|value| value * 2), 6);
}

#[derive(EntityEvent, Clone)]
struct Poke {
    #[event_target]
    entity: Entity,
}

#[test]
fn event_handlers_capture_signals_by_clone() {
    let open = signal(|| false);
    let mut app = app();
    let scene = coat! {
        Root
        on(move |_: On<Poke>| open.set(!$open))
        Children [
            if $open { Label(1) Text("open") }
        ]
    };
    let root = app.world_mut().spawn_scene(scene).unwrap().id();
    app.update();
    assert_eq!(texts(&mut app), Vec::new());
    app.world_mut().trigger(Poke { entity: root });
    app.update();
    assert_eq!(texts(&mut app), vec![(1, "open".into())]);
    assert!(open.get());
}

#[test]
fn bare_signal_uses_inside_regions_are_cloned() {
    let count = signal(|| 1);
    let mut app = app();
    let scene = coat! {
        Root
        Children [
            if $count > 0 {
                Label(1)
                Text("reset")
                on(move |_: On<Poke>| count.set(0))
            }
            --
            Label(2)
            on(move |_: On<Poke>| count.set(5))
        ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    let reset = labels(&mut app)[0].1;
    app.world_mut().trigger(Poke { entity: reset });
    app.update();
    assert_eq!(count.get(), 0);
    assert_eq!(texts(&mut app), Vec::new());
}

#[derive(Clone, Debug, PartialEq)]
enum Load {
    Loading(u32),
    Done(&'static str),
}

#[test]
fn match_regions_rebuild_only_when_the_arm_or_its_bindings_change() {
    let state = signal(|| Load::Loading(10));
    let mut app = app();
    let scene = coat! {
    Root
    Children [
        match $state {
            Load::Loading(_) => Label(1) Text($(format!("{:?}", $state))),
            Load::Done(name) => Label(2) Text({name}),
        }
    ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    let loading = labels(&mut app)[0].1;
    assert_eq!(texts(&mut app), vec![(1, "Loading(10)".into())]);

    // Same arm, no bindings used: patched in place, not rebuilt.
    state.set(Load::Loading(50));
    app.update();
    assert_eq!(labels(&mut app)[0].1, loading);
    assert_eq!(texts(&mut app), vec![(1, "Loading(50)".into())]);

    state.set(Load::Done("a"));
    app.update();
    let done = labels(&mut app)[0].1;
    assert_eq!(texts(&mut app), vec![(2, "a".into())]);

    // A binding the arm uses changed: rebuilt.
    state.set(Load::Done("b"));
    app.update();
    assert_ne!(labels(&mut app)[0].1, done);
    assert_eq!(texts(&mut app), vec![(2, "b".into())]);
}

#[test]
fn borrowed_reads_do_not_clone() {
    let rows = signal(|| vec![1_u32, 2, 3]);
    let mut app = app();
    let scene = coat! {
    Root
    Children [
        Label(0) Text($(format!("{} rows", $&rows.len())))
        --
        if $&rows.is_empty() { Label(1) Text("empty") }
    ]
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(0, "3 rows".into())]);
    rows.set(Vec::new());
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "0 rows".into()), (1, "empty".into())]
    );
}

#[derive(Resource, Clone, Copy, Default)]
struct Level(u32);

#[test]
fn effects_track_ecs_reads_and_clean_up() {
    let mut app = app();
    app.init_resource::<Level>();
    let root = app
        .world_mut()
        .spawn_scene(bsn! {
            Root
            effect(|reads: &Reads, commands: &mut Commands| {
                let level = reads.resource::<Level>().map_or(0, |level| level.0);
                let label = commands.spawn(Label(level)).id();
                move |commands: &mut Commands| commands.entity(label).despawn()
            })
        })
        .unwrap()
        .id();
    app.update();
    assert_eq!(labels(&mut app).len(), 1);

    app.world_mut().resource_mut::<Level>().0 = 3;
    app.update();
    let now = labels(&mut app);
    assert_eq!(now.len(), 1, "the previous run was cleaned up");
    assert_eq!(now[0].0, 3);

    app.world_mut().despawn(root);
    app.world_mut().flush();
    assert!(labels(&mut app).is_empty(), "despawn runs the cleanup");
}
