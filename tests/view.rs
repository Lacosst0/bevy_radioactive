#![cfg(feature = "view")]
//! Integration coverage for `view!` templates and signals.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::view::*;

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
        ViewPlugin,
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
    let scene = {
        let (open, count) = (open.clone(), count.clone());
        view! {
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
        }
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
    let scene = {
        let rows = rows.clone();
        view! {
            Root
            Children [
                #[key(row.id)]
                for row in $rows {
                    Label({row.id}) Text({row.name})
                }
            ]
        }
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
    let scene = {
        let (tab, title) = (tab.clone(), title.clone());
        view! {
            Root
            Children [
                Label(0) Text($(title.get()))
                --
                match $tab {
                    Tab::Home => Label(1) Text("home"),
                    Tab::Settings => { Label(2) Text("settings") },
                }
            ]
        }
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
        .spawn_scene(view! {
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
    count.update(|count| *count += 1);
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
    let scene = {
        let open = open.clone();
        view! {
            Root
            on(move |_: On<Poke>| open.set(!$open))
            Children [
                if $open { Label(1) Text("open") }
            ]
        }
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
    let scene = {
        let count = count.clone();
        view! {
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
        }
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    let reset = labels(&mut app)[0].1;
    app.world_mut().trigger(Poke { entity: reset });
    app.update();
    assert_eq!(count.get(), 0);
    assert_eq!(texts(&mut app), Vec::new());
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
struct Volume(u32);

#[test]
fn effect_reruns_when_a_signal_it_read_changes() {
    let volume = signal(|| 1_u32);
    let mut app = app();
    app.init_resource::<Volume>();
    let scene = {
        let volume = volume.clone();
        bsn! {
            effect(move |commands: &mut Commands| commands.insert_resource(Volume(volume.get())))
        }
    };
    app.world_mut().spawn_scene(scene).unwrap();
    app.update();
    assert_eq!(*app.world().resource::<Volume>(), Volume(1));
    volume.set(4);
    app.update();
    assert_eq!(*app.world().resource::<Volume>(), Volume(4));
}

#[test]
fn views_require_the_plugin() {
    let open = signal(|| true);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    let Err(error) = app.world_mut().spawn_scene(view! {
        Root
        Children [ if $open { Label(1) } ]
    }) else {
        panic!("view resolved without its plugin");
    };
    assert!(error.to_string().contains("ViewPlugin"));
}
