//! Integration coverage for automatically tracked `webby` effects.

use bevy::{
    asset::AssetPlugin, ecs::relationship::RelationshipTarget, prelude::*, scene::ScenePlugin,
};
use bevy_radioactive::{RadioactivePlugin, webby::*};

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Health(u32);

#[derive(Resource, Clone, Debug, Default, PartialEq)]
struct Items(Vec<(u32, &'static str)>);

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Low(bool);

#[derive(Component, Clone, Copy, Default)]
struct Label;

#[derive(Component, Clone, Copy, Default)]
struct List;

#[derive(Component, Clone, Copy, Default)]
struct Row(u32);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        RadioactivePlugin,
    ))
    .init_resource::<Items>();
    app
}

fn single<T: Component>(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<T>>()
        .single(app.world())
        .unwrap()
}

fn label_text(app: &mut App) -> String {
    let label = single::<Label>(app);
    app.world().get::<Text>(label).unwrap().0.clone()
}

#[test]
fn text_and_prop_render_immediately_and_follow_changes() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #Player Health(100)
                --
                Label
                Text("")
                text(#Player, |cx, player| format!("HP {}", cx.get::<Health>(player).map_or(0, |hp| hp.0)))
                prop(#Player, |cx, player| Low(cx.get::<Health>(player).is_some_and(|hp| hp.0 < 30)))
            ]
        })
        .unwrap();
    app.update();
    assert_eq!(label_text(&mut app), "HP 100");

    let player = single::<Health>(&mut app);
    app.world_mut().get_mut::<Health>(player).unwrap().0 = 20;
    app.update();
    assert_eq!(label_text(&mut app), "HP 20");
    let label = single::<Label>(&mut app);
    assert_eq!(app.world().get::<Low>(label), Some(&Low(true)));

    app.world_mut().entity_mut(player).remove::<Health>();
    app.update();
    assert_eq!(label_text(&mut app), "HP 0");
}

#[test]
fn effects_without_changes_do_not_rerun() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Label
            Text("")
            text((), |cx, ()| format!("{}", cx.resource::<Items>().map_or(0, |i| i.0.len())))
        })
        .unwrap();
    app.update();
    let label = single::<Label>(&mut app);
    let tick = app
        .world()
        .entity(label)
        .get_change_ticks::<Text>()
        .unwrap()
        .changed;
    app.update();
    app.update();
    let again = app
        .world()
        .entity(label)
        .get_change_ticks::<Text>()
        .unwrap()
        .changed;
    assert_eq!(tick, again);
}

#[test]
fn each_is_keyed_and_when_toggles_children() {
    let mut app = app();
    app.world_mut().resource_mut::<Items>().0 = vec![(1, "a"), (2, "b")];
    app.world_mut()
        .spawn_scene(bsn! {
            Children [
                #Player Health(1)
                --
                List
                each(
                    (),
                    |cx, ()| cx.resource::<Items>().map(|i| i.0.clone()).unwrap_or_default(),
                    |item: &(u32, &'static str)| item.0,
                    |item, ()| { let id = item.0; bsn! { Row({id}) } },
                )
                --
                Label
                when(#Player, |cx, player| cx.get::<Health>(player).is_none(), |_| bsn! { Row(99) })
            ]
        })
        .unwrap();
    app.update();

    let rows = |app: &mut App| -> Vec<u32> {
        let list = single::<List>(app);
        app.world()
            .get::<Children>(list)
            .map(|children| {
                children
                    .iter()
                    .map(|c| app.world().get::<Row>(c).unwrap().0)
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(rows(&mut app), vec![1, 2]);
    let list = single::<List>(&mut app);
    let first = app.world().get::<Children>(list).unwrap()[0];

    app.world_mut().resource_mut::<Items>().0 = vec![(2, "b"), (3, "c"), (1, "a")];
    app.update();
    assert_eq!(rows(&mut app), vec![2, 3, 1]);
    assert_eq!(app.world().get::<Children>(list).unwrap()[2], first);

    let label = single::<Label>(&mut app);
    assert!(
        app.world()
            .get::<Children>(label)
            .is_none_or(RelationshipTarget::is_empty)
    );
    let player = single::<Health>(&mut app);
    app.world_mut().despawn(player);
    app.update();
    assert_eq!(
        app.world()
            .get::<Children>(label)
            .map(RelationshipTarget::len),
        Some(1)
    );
}

#[test]
fn effects_require_the_runtime() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin));
    let Err(error) = app.world_mut().spawn_scene(bsn! {
        Text("")
        text((), |_, ()| "x")
    }) else {
        panic!("effect resolved without runtime");
    };
    assert!(error.to_string().contains("RadioactivePlugin"));
}
