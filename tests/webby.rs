//! Integration coverage for React-style `webby` components.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::{
    RadioactivePlugin,
    webby::{Cx, SetState, Signal, component, either, keyed},
};

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
        RadioactivePlugin,
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
    cx.untracked()
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
    let probe = cx.untracked().resource::<Memos>().0.clone();
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
    cx.provide_context(Signal::new(String::from(value)));
    bsn! { @consumer() }
}

#[component]
fn consumer(cx: &Cx) -> impl Scene {
    let value = cx
        .use_context::<Signal<String>>()
        .map(|signal| signal.get())
        .unwrap_or_default();
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
    assert!(error.to_string().contains("RadioactivePlugin"));
}

#[derive(Resource, Clone, Default)]
struct Dispatcher(Option<bevy_radioactive::webby::Dispatch<i32>>);

#[component]
fn reducer(cx: &Cx) -> impl Scene {
    let (total, dispatch) = cx.use_reducer(|total, delta: i32| total + delta, || 0);
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

#[derive(Resource, Default)]
struct Events(Vec<String>);

#[component]
fn tracked_child(cx: &Cx, id: u32) -> impl Scene {
    cx.use_effect(id, move |id, commands| {
        let id = *id;
        commands.queue(move |world: &mut World| {
            world
                .resource_mut::<Events>()
                .0
                .push(format!("child {id} effect"));
        });
        move |commands: &mut Commands| {
            commands.queue(move |world: &mut World| {
                world
                    .resource_mut::<Events>()
                    .0
                    .push(format!("child {id} cleanup"));
            });
        }
    });
    cx.on_unmount(move |commands| {
        commands.queue(move |world: &mut World| {
            world
                .resource_mut::<Events>()
                .0
                .push(format!("child {id} unmount"));
        });
    });
    bsn! { Label({id}) }
}

#[component]
fn effect_parent(cx: &Cx) -> impl Scene {
    let ids = cx.use_resource::<Order>().unwrap_or_default().0;
    let rows = ids
        .into_iter()
        .map(|id| tracked_child(id).key(id))
        .collect::<Vec<_>>();
    cx.use_effect((), |(), commands| {
        commands.queue(|world: &mut World| {
            // Children rendered before this effect ran.
            let labels = world.query::<&Label>().iter(world).count();
            world
                .resource_mut::<Events>()
                .0
                .push(format!("parent effect sees {labels}"));
        });
    });
    bsn! { Node Children [ {rows} ] }
}

#[test]
fn effects_run_after_children_and_clean_up_on_unmount() {
    let mut app = app();
    app.init_resource::<Events>();
    app.world_mut().resource_mut::<Order>().0 = vec![1, 2];
    let root = app
        .world_mut()
        .spawn_scene(bsn! { @effect_parent() })
        .unwrap()
        .id();
    app.update();
    let events = std::mem::take(&mut app.world_mut().resource_mut::<Events>().0);
    assert_eq!(events.last().unwrap(), "parent effect sees 2");
    assert!(events.contains(&"child 1 effect".to_string()));

    // Child 1 is no longer rendered: it unmounts and cleans up its effect.
    app.world_mut().resource_mut::<Order>().0 = vec![2];
    app.update();
    let mut events = std::mem::take(&mut app.world_mut().resource_mut::<Events>().0);
    events.sort();
    assert_eq!(events, vec!["child 1 cleanup", "child 1 unmount"]);

    app.world_mut().despawn(root);
    app.update();
    let mut events = std::mem::take(&mut app.world_mut().resource_mut::<Events>().0);
    events.sort();
    assert_eq!(events, vec!["child 2 cleanup", "child 2 unmount"]);
}

// ---------------------------------------------------------------------------
// 0.5: call-site keys, `#[entity]` props, `Result` renders and `either`.
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Default)]
struct ShowExtra(bool);

#[derive(Resource, Clone, Default)]
struct Cells(Vec<(u32, Signal<i32>)>);

#[component]
fn cell(cx: &Cx, id: u32) -> impl Scene {
    let value = cx.use_signal(|| 0);
    let shared = value.clone();
    cx.use_effect((), move |(), commands| {
        let shared = shared.clone();
        commands.queue(move |world: &mut World| {
            world.resource_mut::<Cells>().0.push((id, shared));
        });
    });
    bsn! { Label({id}) Text({value.get().to_string()}) }
}

#[component]
fn cells(cx: &Cx) -> impl Scene {
    let show = cx.use_resource::<ShowExtra>().unwrap_or_default().0;
    bsn! {
        Node Children [
            {show.then(|| bsn_list! { @cell(1) })}
            --
            @cell(2)
        ]
    }
}

#[test]
fn a_conditional_sibling_does_not_take_another_components_state() {
    let mut app = app();
    app.init_resource::<ShowExtra>().init_resource::<Cells>();
    app.world_mut().spawn_scene(bsn! { @cells() }).unwrap();
    app.update();
    let second = app.world().resource::<Cells>().0[0].clone();
    assert_eq!(second.0, 2);
    second.1.set(42);
    app.update();
    assert_eq!(texts(&mut app), vec![(2, "42".into())]);

    // Both components hold a `Signal<i32>`, so a positional key would hand
    // cell 2's state to the new cell 1 without any type mismatch.
    app.world_mut().resource_mut::<ShowExtra>().0 = true;
    app.update();
    assert_eq!(texts(&mut app), vec![(1, "0".into()), (2, "42".into())]);
}

#[component]
fn party_hp(
    cx: &Cx,
    #[entity] leader: Option<Entity>,
    #[entity] members: Vec<Entity>,
) -> impl Scene {
    let hp = |entity: Entity| cx.use_component::<Health>(entity).map_or(0, |hp| hp.0);
    let leader = leader.map_or(0, hp);
    let total: u32 = members.into_iter().map(hp).sum();
    bsn! { Label(6) Text({format!("{leader}/{total}")}) }
}

#[test]
fn entity_attribute_props_accept_options_and_lists() {
    let mut app = app();
    let a = app.world_mut().spawn(Health(10)).id();
    let b = app.world_mut().spawn(Health(5)).id();
    app.world_mut()
        .spawn_scene(bsn! { @party_hp(Some(a), vec![a, b]) })
        .unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(6, "10/15".into())]);

    app.world_mut().get_mut::<Health>(b).unwrap().0 = 1;
    app.update();
    assert_eq!(texts(&mut app), vec![(6, "10/11".into())]);
}

static RENDER_ERRORS: AtomicUsize = AtomicUsize::new(0);

fn count_errors(_: BevyError, _: bevy::ecs::error::ErrorContext) {
    RENDER_ERRORS.fetch_add(1, Ordering::Relaxed);
}

#[component]
fn fallible(cx: &Cx) -> Result<impl Scene> {
    if cx.use_resource::<ShowExtra>().unwrap_or_default().0 {
        return Err("no scene".into());
    }
    Ok(bsn! { Label(9) Text("ok") })
}

#[component]
fn switched(cx: &Cx) -> impl Scene {
    let show = cx.use_resource::<ShowExtra>().unwrap_or_default().0;
    either(
        show,
        || bsn! { Label(3) Text("left") },
        || bsn! { Node Children [ Label(4) Text("right") ] },
    )
}

#[test]
fn result_renders_report_errors_and_either_switches_scene_types() {
    let mut app = app();
    app.init_resource::<ShowExtra>()
        .insert_resource(bevy::ecs::error::FallbackErrorHandler(count_errors));
    app.world_mut()
        .spawn_scene(bsn! { Node Children [ @fallible() -- @switched() ] })
        .unwrap();
    app.update();
    assert_eq!(texts(&mut app), vec![(4, "right".into()), (9, "ok".into())]);
    assert_eq!(RENDER_ERRORS.load(Ordering::Relaxed), 0);

    app.world_mut().resource_mut::<ShowExtra>().0 = true;
    app.update();
    assert_eq!(texts(&mut app), vec![(3, "left".into())]);
    assert_eq!(RENDER_ERRORS.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// Child retention: a parent re-render keeps child component hosts and skips
// children whose props did not change.
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Default)]
struct Step(u32);

#[derive(Resource, Clone, Default)]
struct RenderLog(Arc<std::sync::Mutex<Vec<u32>>>);

impl RenderLog {
    fn take(app: &App) -> Vec<u32> {
        let mut log = std::mem::take(&mut *app.world().resource::<Self>().0.lock().unwrap());
        log.sort_unstable();
        log
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
struct Host(u32);

#[derive(EntityEvent)]
struct Ping {
    entity: Entity,
}

fn retention_app() -> App {
    let mut app = app();
    app.init_resource::<Step>()
        .init_resource::<RenderLog>()
        .init_resource::<Cells>();
    app
}

/// Logs each render and exposes its state signal through `Cells`.
#[component]
fn row(cx: &Cx, id: u32, label: String) -> impl Scene {
    cx.untracked()
        .resource::<RenderLog>()
        .0
        .lock()
        .unwrap()
        .push(id);
    let clicks = cx.use_signal(|| 0);
    let shared = clicks.clone();
    cx.use_effect((), move |(), commands| {
        let shared = shared.clone();
        commands.queue(move |world: &mut World| {
            world.resource_mut::<Cells>().0.push((id, shared));
        });
    });
    bsn! { Label({id}) Text({format!("{label} {}", clicks.get())}) }
}

#[component]
fn rows_parent(cx: &Cx) -> impl Scene {
    let step = cx.use_resource::<Step>().unwrap_or_default().0;
    let rows = cx
        .use_resource::<Order>()
        .unwrap_or_default()
        .0
        .into_iter()
        .map(|id| {
            // Row 2's label follows the parent's state; the others are fixed.
            let label = if id == 2 {
                format!("step{step}")
            } else {
                "row".into()
            };
            row(id, label).key(id)
        })
        .collect::<Vec<_>>();
    bsn! {
        Node Children [
            Label(100) Text({format!("parent {step}")})
            --
            Node Children [ {rows} ]
        ]
    }
}

/// `(id, rendered entity, host entity)` of every rendered row.
fn rows(app: &mut App) -> Vec<(u32, Entity, Entity)> {
    let mut rows: Vec<_> = app
        .world_mut()
        .query_filtered::<(Entity, &Label, &ChildOf), With<Text>>()
        .iter(app.world())
        .filter(|(_, label, _)| label.0 < 100)
        .map(|(entity, label, parent)| (label.0, entity, parent.parent()))
        .collect();
    rows.sort();
    rows
}

fn cell_signal(app: &App, id: u32) -> Signal<i32> {
    app.world()
        .resource::<Cells>()
        .0
        .iter()
        .find(|(cell, _)| *cell == id)
        .map(|(_, signal)| signal.clone())
        .unwrap()
}

#[test]
fn a_parent_re_render_keeps_children_and_skips_unchanged_ones() {
    let mut app = retention_app();
    app.world_mut().resource_mut::<Order>().0 = vec![1, 2, 3];
    app.world_mut()
        .spawn_scene(bsn! { @rows_parent() })
        .unwrap();
    app.update();
    assert_eq!(RenderLog::take(&app), vec![1, 2, 3]);
    cell_signal(&app, 1).set(5);
    cell_signal(&app, 2).set(7);
    app.update();
    assert_eq!(RenderLog::take(&app), vec![1, 2]);
    let before = rows(&mut app);

    app.world_mut().resource_mut::<Step>().0 = 1;
    app.update();
    // Only row 2, whose `label` prop changed, rendered again.
    assert_eq!(RenderLog::take(&app), vec![2]);
    assert_eq!(
        texts(&mut app),
        vec![
            (1, "row 5".into()),
            (2, "step1 7".into()),
            (3, "row 0".into()),
            (100, "parent 1".into()),
        ]
    );
    let after = rows(&mut app);
    // Every host is the same entity; unchanged rows kept their rendered
    // subtree, and row 2 kept its hook state.
    for ((id, old_rendered, old_host), (_, new_rendered, new_host)) in before.iter().zip(&after) {
        assert_eq!(old_host, new_host, "row {id} host");
        assert_eq!(
            *id == 2,
            old_rendered != new_rendered,
            "row {id} rendered entity"
        );
    }

    // The kept hosts were moved into the parent's new scene, in order.
    let list = app.world().get::<ChildOf>(after[0].2).unwrap().parent();
    let hosts: Vec<Entity> = after.iter().map(|row| row.2).collect();
    assert_eq!(app.world().get::<Children>(list).unwrap().to_vec(), hosts);
    let labels = app.world_mut().query::<&Label>().iter(app.world()).count();
    assert_eq!(labels, 4);
}

#[test]
fn children_that_went_away_are_despawned_and_others_keep_their_order() {
    let mut app = retention_app();
    app.world_mut().resource_mut::<Order>().0 = vec![1, 2, 3];
    app.world_mut()
        .spawn_scene(bsn! { @rows_parent() })
        .unwrap();
    app.update();
    RenderLog::take(&app);
    let before = rows(&mut app);
    let host = |id: u32| before.iter().find(|row| row.0 == id).unwrap().2;

    app.world_mut().resource_mut::<Order>().0 = vec![3, 1];
    app.update();
    assert_eq!(RenderLog::take(&app), Vec::<u32>::new());
    assert!(app.world().get_entity(host(2)).is_err());
    let after = rows(&mut app);
    assert_eq!(after, vec![before[0], before[2]]);
    let list = app.world().get::<ChildOf>(host(3)).unwrap().parent();
    assert_eq!(
        app.world().get::<Children>(list).unwrap().to_vec(),
        vec![host(3), host(1)]
    );
    // Nothing from the previous render is left behind.
    let labels = app.world_mut().query::<&Label>().iter(app.world()).count();
    assert_eq!(labels, 3);
}

/// A prop type without `PartialEq`.
#[derive(Clone)]
struct Opaque(u32);

#[component]
fn opaque_row(cx: &Cx, value: Opaque) -> impl Scene {
    cx.untracked()
        .resource::<RenderLog>()
        .0
        .lock()
        .unwrap()
        .push(value.0);
    let (count, _) = cx.use_state(|| value.0 * 10);
    bsn! { Label(1) Text({count.to_string()}) }
}

#[component]
fn opaque_parent(cx: &Cx) -> impl Scene {
    let step = cx.use_resource::<Step>().unwrap_or_default().0;
    bsn! { Node Children [ Label(100) Text({step.to_string()}) -- @opaque_row(Opaque(4)) ] }
}

#[test]
fn props_without_partial_eq_re_render_but_keep_the_host() {
    let mut app = retention_app();
    app.world_mut()
        .spawn_scene(bsn! { @opaque_parent() })
        .unwrap();
    app.update();
    let before = rows(&mut app);
    app.world_mut().resource_mut::<Step>().0 = 1;
    app.update();
    assert_eq!(RenderLog::take(&app), vec![4, 4]);
    let after = rows(&mut app);
    assert_eq!(before[0].2, after[0].2);
    assert_eq!(texts(&mut app)[0], (1, "40".into()));
}

#[derive(Resource, Clone, Default)]
struct Hits(Arc<AtomicUsize>);

#[component]
fn decorated_parent(cx: &Cx) -> impl Scene {
    let step = cx.use_resource::<Step>().unwrap_or_default().0;
    let hits = cx.untracked().resource::<Hits>().0.clone();
    // The kept row is the root of this scene, with a component and an
    // observer declared beside it.
    bsn! {
        @row(1, String::from("row"))
        Host({step})
        on(move |_: On<Ping>| {
            hits.fetch_add(1, Ordering::Relaxed);
        })
    }
}

#[test]
fn what_the_parent_declares_beside_a_kept_child_is_replaced() {
    let mut app = retention_app();
    app.init_resource::<Hits>();
    let root = app
        .world_mut()
        .spawn_scene(bsn! { @decorated_parent() })
        .unwrap()
        .id();
    app.update();
    let host = rows(&mut app)[0].2;
    assert_eq!(app.world().get::<Host>(host), Some(&Host(0)));

    for step in 1..=2 {
        app.world_mut().resource_mut::<Step>().0 = step;
        app.update();
    }
    assert_eq!(RenderLog::take(&app), vec![1]);
    assert_eq!(rows(&mut app)[0].2, host);
    assert_eq!(app.world().get::<Host>(host), Some(&Host(2)));
    assert_eq!(app.world().get::<ChildOf>(host).unwrap().parent(), root);

    // One observer, not one per parent render.
    app.world_mut().trigger(Ping { entity: host });
    app.world_mut().flush();
    assert_eq!(app.world().resource::<Hits>().0.load(Ordering::Relaxed), 1);
}
