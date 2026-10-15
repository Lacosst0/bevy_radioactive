#![cfg(feature = "view")]
//! Integration coverage for `view!` templates and `Watch` parameters.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::view::*;

#[derive(Component, Clone, Default)]
struct Label(u32);

#[derive(Component, Clone, Default)]
struct Root;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Home,
    Settings,
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CurrentTab(Tab);

#[derive(Clone, Debug, PartialEq)]
struct Row {
    id: u32,
    name: &'static str,
}

#[derive(Resource, Clone, Debug, Default, PartialEq)]
struct Rows(Vec<Row>);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
struct Volume(u32);

#[derive(EntityEvent, Clone)]
struct Poke {
    #[event_target]
    entity: Entity,
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

fn root(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<Root>>()
        .single(app.world())
        .unwrap()
}

fn poke(app: &mut App, entity: Entity) {
    app.world_mut().trigger(Poke { entity });
    app.update();
}

#[component]
fn toggle(#[init(1)] count: Watch<Local<u32>>, open: Watch<Local<bool>>) -> impl Scene {
    view! {
        Root
        on(move |_: On<Poke>| {
            open.set(!$open);
            count.update(|count| *count += 1);
        })
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
}

#[test]
fn local_state_drives_if_and_patched_text() {
    let mut app = app();
    app.world_mut().spawn_scene(bsn! { @toggle() }).unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "count 1".into()), (2, "closed".into())]
    );

    let root = root(&mut app);
    poke(&mut app, root);
    assert_eq!(
        texts(&mut app),
        vec![(0, "count 2".into()), (1, "open".into())]
    );
}

#[test]
fn each_instance_has_its_own_local_state() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { Children [ @toggle() -- @toggle() ] })
        .unwrap();
    app.update();
    let first = app
        .world_mut()
        .query_filtered::<Entity, With<Root>>()
        .iter(app.world())
        .next()
        .unwrap();
    poke(&mut app, first);
    let mut texts = texts(&mut app);
    texts.retain(|(label, _)| *label == 0);
    assert_eq!(texts, vec![(0, "count 1".into()), (0, "count 2".into())]);
}

#[component]
fn row_list(rows: Watch<Res<Rows>>) -> impl Scene {
    view! {
        Root
        Children [
            #[key(row.id)]
            for row in $rows.0 {
                Label({row.id}) Text({row.name})
            }
        ]
    }
}

#[test]
fn resource_watch_drives_keyed_for_and_reuses_rows() {
    let mut app = app();
    app.insert_resource(Rows(vec![
        Row { id: 1, name: "a" },
        Row { id: 2, name: "b" },
    ]));
    app.world_mut().spawn_scene(bsn! { @row_list() }).unwrap();
    app.update();
    let before = labels(&mut app);
    assert_eq!(before.len(), 2);

    app.world_mut().resource_mut::<Rows>().0 = vec![
        Row { id: 2, name: "b" },
        Row { id: 3, name: "c" },
        Row { id: 1, name: "a" },
    ];
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

#[component]
fn tabs(tab: Watch<Res<CurrentTab>>) -> impl Scene {
    view! {
        Root
        Children [
            Label(0) Text($(format!("{:?}", $tab.0)))
            --
            match $tab.0 {
                Tab::Home => Label(1) Text("home"),
                Tab::Settings => { Label(2) Text("settings") },
            }
        ]
    }
}

#[test]
fn resource_watch_drives_match() {
    let mut app = app();
    app.init_resource::<CurrentTab>();
    app.world_mut().spawn_scene(bsn! { @tabs() }).unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "Home".into()), (1, "home".into())]
    );

    // Equal writes are skipped by `set_if_neq` and cause no rebuild.
    let home = labels(&mut app)[1].1;
    app.world_mut()
        .resource_mut::<CurrentTab>()
        .set_if_neq(CurrentTab(Tab::Home));
    app.update();
    assert_eq!(labels(&mut app)[1].1, home);

    app.world_mut().resource_mut::<CurrentTab>().0 = Tab::Settings;
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(0, "Settings".into()), (2, "settings".into())]
    );
}

#[component]
fn maybe_volume(volume: Watch<Option<Res<Volume>>>) -> impl Scene {
    view! {
        Root
        Label(0)
        Text($(volume_text($volume)))
    }
}

fn volume_text(volume: Option<Volume>) -> String {
    volume.map_or_else(
        || "no volume".into(),
        |volume| format!("volume {}", volume.0),
    )
}

#[test]
fn optional_resource_watch_follows_insert_and_remove() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { @maybe_volume() })
        .unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(0, "no volume".into())]);

    app.insert_resource(Volume(3));
    app.update();
    assert_eq!(texts(&mut app), vec![(0, "volume 3".into())]);

    app.world_mut().remove_resource::<Volume>();
    app.update();
    assert_eq!(texts(&mut app), vec![(0, "no volume".into())]);
}

#[component]
fn labelled(id: u32, name: &'static str, #[init(id * 10)] score: Watch<Local<u32>>) -> impl Scene {
    view! {
        Label({id})
        Text($(format!("{name} {}", $score)))
    }
}

#[test]
fn props_and_init_from_props() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { Children [ @labelled(1, "a") -- @labelled(2, "b") ] })
        .unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(1, "a 10".into()), (2, "b 20".into())]
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
fn local_watch_methods() {
    let count = Watch::<Local<i32>>::new(1);
    count.update(|count| *count += 1);
    assert_eq!(count.get(), 2);
    assert!(!count.set_if_neq(2));
    assert!(count.set_if_neq(3));
    assert_eq!(count.with(|value| value * 2), 6);
    assert_eq!(Watch::<Local<i32>>::default().get(), 0);
}

#[component]
fn reset(count: Watch<Local<u32>>) -> impl Scene {
    view! {
        Root
        on(move |_: On<Poke>| count.set(5))
        Children [
            if $count > 0 {
                Label(1)
                Text("reset")
                on(move |_: On<Poke>| count.set(0))
            }
        ]
    }
}

#[test]
fn handlers_inside_regions_share_the_watch() {
    let mut app = app();
    app.world_mut().spawn_scene(bsn! { @reset() }).unwrap();
    app.update();
    assert_eq!(texts(&mut app), Vec::new());

    let root = root(&mut app);
    poke(&mut app, root);
    assert_eq!(texts(&mut app), vec![(1, "reset".into())]);

    let button = labels(&mut app)[0].1;
    poke(&mut app, button);
    assert_eq!(texts(&mut app), Vec::new());
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
struct Saved(u32);

#[component]
fn saver(volume: Watch<Res<Volume>>) -> impl Scene {
    bsn! {
        effect(move |commands: &mut Commands| commands.insert_resource(Saved(volume.get().0 * 2)))
    }
}

#[test]
fn effect_reruns_when_a_watch_it_read_changes() {
    let mut app = app();
    app.insert_resource(Volume(1));
    app.world_mut().spawn_scene(bsn! { @saver() }).unwrap();
    app.update();
    assert_eq!(*app.world().resource::<Saved>(), Saved(2));
    app.world_mut().resource_mut::<Volume>().0 = 4;
    app.update();
    assert_eq!(*app.world().resource::<Saved>(), Saved(8));
}

#[test]
fn views_require_the_plugin() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    app.init_resource::<CurrentTab>();
    let Err(error) = app.world_mut().spawn_scene(bsn! { @tabs() }) else {
        panic!("view resolved without its plugin");
    };
    assert!(error.to_string().contains("ViewPlugin"));
}
