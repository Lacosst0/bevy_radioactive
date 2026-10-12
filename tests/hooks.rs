#![cfg(feature = "hooks")]
//! Integration coverage for React-style `hooks` components.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::hooks::*;

#[derive(Resource, Clone, Debug, Default, PartialEq)]
struct Title(String);

#[derive(Resource, Clone, Default)]
struct Setter(Option<SetState<i32>>);

#[derive(Resource, Clone, Default)]
struct Order(Vec<u32>);

#[derive(Resource, Default)]
struct Renders(Arc<AtomicUsize>);

#[derive(Resource, Default)]
struct Memos(Arc<AtomicUsize>);

#[derive(Component, Clone, Default)]
struct Label(u32);

#[derive(Component, Clone, Copy, Default, PartialEq)]
struct Health(u32);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        HooksPlugin,
    ))
    .init_resource::<Title>()
    .init_resource::<Setter>()
    .init_resource::<Order>()
    .init_resource::<Renders>()
    .init_resource::<Memos>();
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

#[component]
fn counter(cx: &Cx, id: u32) -> impl Scene {
    let (count, set_count) = cx.use_state(|| 0);
    cx.use_effect((), move |(), commands| {
        if id == 0 {
            commands.insert_resource(Setter(Some(set_count.clone())));
        }
    });
    bsn! { Label({id}) Text({format!("count {count}")}) }
}

#[test]
fn use_state_setter_rerenders() {
    let mut app = app();
    app.world_mut().spawn_scene(bsn! { @counter(0) }).unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(0, "count 0".into())]);

    let setter = app.world().resource::<Setter>().0.clone().unwrap();
    setter(2);
    app.update();
    assert_eq!(texts(&mut app), vec![(0, "count 2".into())]);
}

#[component]
fn titled(cx: &Cx) -> impl Scene {
    let title = cx.use_resource::<Title>().unwrap_or_default().0;
    cx.world()
        .resource::<Renders>()
        .0
        .fetch_add(1, Ordering::Relaxed);
    bsn! {
        Node
        Children [
            Label(99) Text({title})
            --
            @counter(0)
        ]
    }
}

#[test]
fn resource_reads_rerender_and_child_state_survives() {
    let mut app = app();
    app.world_mut().spawn_scene(bsn! { @titled() }).unwrap();
    app.update();
    app.update();
    assert_eq!(
        app.world().resource::<Renders>().0.load(Ordering::Relaxed),
        1
    );

    let setter = app.world().resource::<Setter>().0.clone().unwrap();
    setter(7);
    app.update();
    app.world_mut().resource_mut::<Title>().0 = "Hello".into();
    app.update();
    assert_eq!(
        app.world().resource::<Renders>().0.load(Ordering::Relaxed),
        2
    );
    assert_eq!(
        texts(&mut app),
        vec![(0, "count 7".into()), (99, "Hello".into())]
    );
}

#[component]
fn title_length(cx: &Cx) -> impl Scene {
    let title = cx.use_resource::<Title>().unwrap_or_default().0;
    let probe = cx.world().resource::<Memos>().0.clone();
    let length = cx.use_memo(title.len(), move |len| {
        probe.fetch_add(1, Ordering::Relaxed);
        *len
    });
    bsn! { Label(1) Text({length.to_string()}) }
}

#[test]
fn use_memo_recomputes_only_when_deps_change() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { @title_length() })
        .unwrap();
    app.update();
    app.world_mut().resource_mut::<Title>().0 = "ab".into();
    app.update();
    app.world_mut().resource_mut::<Title>().0 = "cd".into();
    app.update();
    assert_eq!(app.world().resource::<Memos>().0.load(Ordering::Relaxed), 2);
    assert_eq!(texts(&mut app), vec![(1, "2".into())]);
}

#[component]
fn provider(cx: &Cx, value: &'static str) -> impl Scene {
    cx.provide_context(String::from(value));
    bsn! { @consumer() }
}

#[component]
fn consumer(cx: &Cx) -> impl Scene {
    let value = cx.use_context::<String>().unwrap_or_default();
    bsn! { Label(3) Text({value}) }
}

#[test]
fn context_reaches_descendants() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { @provider("ctx") })
        .unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(3, "ctx".into())]);
}

#[component]
fn keyed_row(cx: &Cx, id: u32) -> impl Scene {
    let (initial, _) = cx.use_state(|| id * 10);
    bsn! { Label({id}) Text({initial.to_string()}) }
}

#[component]
fn keyed_list(cx: &Cx) -> impl Scene {
    let rows = cx
        .use_resource::<Order>()
        .unwrap_or_default()
        .0
        .into_iter()
        .map(|id| keyed_row(id).key(id))
        .collect::<Vec<_>>();
    bsn! { Node Children [ {rows} ] }
}

#[test]
fn keyed_children_keep_state_when_reordered() {
    let mut app = app();
    app.world_mut().resource_mut::<Order>().0 = vec![1, 2];
    app.world_mut().spawn_scene(bsn! { @keyed_list() }).unwrap();
    app.update();
    app.world_mut().resource_mut::<Order>().0 = vec![2, 1, 3];
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(1, "10".into()), (2, "20".into()), (3, "30".into())]
    );
}

#[component]
fn health_label(cx: &Cx, player: Entity) -> impl Scene {
    let hp = cx.use_component::<Health>(player).map_or(0, |hp| hp.0);
    bsn! { Label(5) Text({format!("HP {hp}")}) }
}

#[test]
fn entity_props_take_scene_references() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Node
            Children [
                #Player Health(100)
                --
                @health_label(#Player)
                --
                @keyed(7, keyed_row(7))
            ]
        })
        .unwrap();
    app.update();
    assert_eq!(
        texts(&mut app),
        vec![(5, "HP 100".into()), (7, "70".into())]
    );

    let mut players = app.world_mut().query_filtered::<Entity, With<Health>>();
    let player = players.single(app.world()).unwrap();
    app.world_mut().get_mut::<Health>(player).unwrap().0 = 40;
    app.update();
    assert_eq!(texts(&mut app)[0], (5, "HP 40".into()));
}

#[test]
fn closures_mount_as_components() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! { component(|_cx| bsn! { Label(8) Text("inline") }) })
        .unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(8, "inline".into())]);
}

#[test]
fn components_require_the_runtime() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    let Err(error) = app.world_mut().spawn_scene(bsn! { @counter(1) }) else {
        panic!("component resolved without runtime");
    };
    assert!(error.to_string().contains("HooksPlugin"));
}

#[derive(Resource, Clone, Default)]
struct Dispatcher(Option<Dispatch<i32>>);

#[component]
fn reducer(cx: &Cx) -> impl Scene {
    let (total, dispatch) = cx.use_reducer(|| 0, |total, delta: i32| total + delta);
    cx.use_effect((), move |(), commands| {
        commands.insert_resource(Dispatcher(Some(dispatch.clone())));
    });
    bsn! { Label(4) Text({total.to_string()}) }
}

#[test]
fn use_reducer_applies_actions_to_latest_state() {
    let mut app = app();
    app.init_resource::<Dispatcher>();
    app.world_mut().spawn_scene(bsn! { @reducer() }).unwrap();
    app.update();
    let dispatch = app.world().resource::<Dispatcher>().0.clone().unwrap();
    dispatch(2);
    dispatch(3);
    app.update();
    assert_eq!(texts(&mut app), vec![(4, "5".into())]);
}
