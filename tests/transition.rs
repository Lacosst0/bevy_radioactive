#![cfg(feature = "reactive")]
//! Enter and exit transitions on region content.

use std::time::Duration;

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin, time::TimeUpdateStrategy};
use bevy_radioactive::reactive::*;

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
struct Open(bool);

#[derive(Resource, Clone, Default)]
struct Items(Vec<u32>);

#[derive(Component, Clone, Default, Debug, PartialEq)]
struct Label(String);

#[derive(Component, Clone, Copy, Default, Debug, PartialEq)]
struct Alpha(f32);

/// Each update advances time by 0.1 s.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        100,
    )))
    .init_resource::<Open>()
    .init_resource::<Items>();
    app
}

fn labels(app: &mut App) -> Vec<String> {
    fn walk(world: &World, entity: Entity, out: &mut Vec<String>) {
        if let Some(label) = world.get::<Label>(entity) {
            out.push(label.0.clone());
        }
        if let Some(children) = world.get::<Children>(entity) {
            for child in children.iter() {
                walk(world, child, out);
            }
        }
    }
    let world = app.world_mut();
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, Without<ChildOf>>()
        .iter(world)
        .collect();
    let mut out = Vec::new();
    for root in roots {
        walk(world, root, &mut out);
    }
    out
}

fn find(app: &mut App, label: &str) -> Option<Entity> {
    let world = app.world_mut();
    world
        .query::<(Entity, &Label)>()
        .iter(world)
        .find(|(_, l)| l.0 == label)
        .map(|(entity, _)| entity)
}

fn presence(app: &mut App, entity: Entity) -> Presence {
    *app.world().get::<Presence>(entity).unwrap()
}

fn set_open(app: &mut App, open: bool) {
    app.world_mut().resource_mut::<Open>().0 = open;
    app.update();
}

fn panel_scene() -> impl Scene {
    rsn! {
        Children [
            if $Res<Open>.0 {
                #Panel
                Label("panel")
                Transition::new(0.4)
                Alpha($Ref<Presence>(#Panel).value())
            } else {
                Label("closed")
            }
        ]
    }
}

#[test]
fn first_render_shows_without_entering() {
    let mut app = app();
    app.world_mut().resource_mut::<Open>().0 = true;
    app.world_mut().spawn_scene(panel_scene()).unwrap();
    app.update();
    let panel = find(&mut app, "panel").unwrap();
    assert_eq!(presence(&mut app, panel).phase(), Phase::Shown);
    assert_eq!(*app.world().get::<Alpha>(panel).unwrap(), Alpha(1.0));
}

#[test]
fn appear_enters_on_the_first_render() {
    let mut app = app();
    app.world_mut().resource_mut::<Open>().0 = true;
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                if $Res<Open>.0 { Label("panel") Transition::new(0.4).appear() }
            ]
        })
        .unwrap();
    app.update();
    let panel = find(&mut app, "panel").unwrap();
    assert_eq!(presence(&mut app, panel).phase(), Phase::Entering);
}

#[test]
fn content_enters_then_leaves_before_despawning() {
    let mut app = app();
    app.world_mut().spawn_scene(panel_scene()).unwrap();
    app.update();
    assert_eq!(labels(&mut app), ["closed"]);

    set_open(&mut app, true);
    let panel = find(&mut app, "panel").unwrap();
    assert_eq!(presence(&mut app, panel).phase(), Phase::Entering);
    // The reaction reading `Presence` follows it.
    let alpha = app.world().get::<Alpha>(panel).unwrap().0;
    assert!(alpha < 1.0, "alpha {alpha}");
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(presence(&mut app, panel).phase(), Phase::Shown);
    assert_eq!(*app.world().get::<Alpha>(panel).unwrap(), Alpha(1.0));

    // Closing: the panel stays, leaving, next to the new branch.
    set_open(&mut app, false);
    assert_eq!(labels(&mut app), ["panel", "closed"]);
    assert!(app.world().get::<Leaving>(panel).is_some());
    app.update();
    let leaving = presence(&mut app, panel);
    assert_eq!(leaving.phase(), Phase::Leaving);
    assert!(leaving.progress() < 1.0);
    assert!(app.world().get::<Alpha>(panel).unwrap().0 < 1.0);

    for _ in 0..5 {
        app.update();
    }
    assert!(app.world().get_entity(panel).is_err());
    assert_eq!(labels(&mut app), ["closed"]);
}

#[test]
fn content_coming_back_while_leaving_turns_around() {
    let mut app = app();
    app.world_mut().resource_mut::<Open>().0 = true;
    app.world_mut().spawn_scene(panel_scene()).unwrap();
    app.update();
    let panel = find(&mut app, "panel").unwrap();

    set_open(&mut app, false);
    app.update();
    let progress = presence(&mut app, panel).progress();
    assert!(progress < 1.0);

    set_open(&mut app, true);
    // The same entity, entering from where it was.
    assert_eq!(find(&mut app, "panel"), Some(panel));
    assert!(app.world().get::<Leaving>(panel).is_none());
    let back = presence(&mut app, panel);
    assert_eq!(back.phase(), Phase::Entering);
    // It kept leaving for that frame's animation step, then turned around.
    assert!(back.progress() > 0.0 && back.progress() <= progress);
    // The `else` branch it replaced is leaving without a transition: gone.
    assert_eq!(labels(&mut app), ["panel"]);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(presence(&mut app, panel).phase(), Phase::Shown);
}

#[test]
fn content_without_a_transition_despawns_at_once() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Children [ if $Res<Open>.0 { Label("panel") } ]
        })
        .unwrap();
    set_open(&mut app, true);
    assert_eq!(labels(&mut app), ["panel"]);
    set_open(&mut app, false);
    assert!(labels(&mut app).is_empty());
}

fn list_scene() -> impl Scene {
    rsn! {
        Children [
            #[key(*id)]
            for id in $Res<Items>.0.clone() {
                Label({ id.to_string() })
                Transition::new(0.2)
            }
        ]
    }
}

fn set_items(app: &mut App, items: &[u32]) {
    app.world_mut().resource_mut::<Items>().0 = items.to_vec();
    app.update();
}

#[test]
fn removed_rows_leave_in_place() {
    let mut app = app();
    app.world_mut().resource_mut::<Items>().0 = vec![1, 2, 3];
    app.world_mut().spawn_scene(list_scene()).unwrap();
    app.update();
    let two = find(&mut app, "2").unwrap();
    assert_eq!(presence(&mut app, two).phase(), Phase::Shown);

    // 2 leaves where it was while 4 enters at the end and 3 moves first.
    set_items(&mut app, &[3, 1, 4]);
    assert_eq!(labels(&mut app), ["3", "1", "2", "4"]);
    assert!(app.world().get::<Leaving>(two).is_some());
    let four = find(&mut app, "4").unwrap();
    assert_eq!(presence(&mut app, four).phase(), Phase::Entering);

    for _ in 0..4 {
        app.update();
    }
    assert_eq!(labels(&mut app), ["3", "1", "4"]);
    assert_eq!(presence(&mut app, four).phase(), Phase::Shown);
}

#[test]
fn a_row_returning_while_leaving_keeps_its_entity() {
    let mut app = app();
    app.world_mut().resource_mut::<Items>().0 = vec![1, 2];
    app.world_mut().spawn_scene(list_scene()).unwrap();
    app.update();
    let two = find(&mut app, "2").unwrap();

    set_items(&mut app, &[1]);
    assert!(app.world().get::<Leaving>(two).is_some());
    set_items(&mut app, &[1, 2]);
    assert_eq!(find(&mut app, "2"), Some(two));
    assert!(app.world().get::<Leaving>(two).is_none());
    assert_eq!(presence(&mut app, two).phase(), Phase::Entering);
    assert_eq!(labels(&mut app), ["1", "2"]);
}

#[test]
fn transitions_work_with_plain_functions() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                @when(|open: Res<Open>| open.0, || bsn! { Label("panel") Transition::new(0.2) })
            ]
        })
        .unwrap();
    app.update();
    set_open(&mut app, true);
    let panel = find(&mut app, "panel").unwrap();
    set_open(&mut app, false);
    assert!(app.world().get::<Leaving>(panel).is_some());
    for _ in 0..3 {
        app.update();
    }
    assert!(app.world().get_entity(panel).is_err());
}
