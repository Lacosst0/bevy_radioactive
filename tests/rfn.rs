#![cfg(feature = "rfn")]
//! `#[rfn]`: system parameters as arguments and derived `let`s; `Reactive<..>`
//! ones are read by name and re-run what reads them.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::rfn::*;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Count(i32);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Score(u32);

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Tab {
    #[default]
    Home,
    Settings,
}

#[derive(Component, Clone, Default, Debug, PartialEq, Eq)]
struct Label(String);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
struct Health(u32);

#[derive(Component, Clone, Copy, Default)]
struct Enemy;

#[derive(Clone, EntityEvent)]
struct Poke {
    entity: Entity,
    by: i32,
}

#[derive(Resource, Default)]
struct Calls(u32);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Score>()
    .init_resource::<Tab>()
    .init_resource::<Calls>();
    app
}

fn labels(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, (Without<ChildOf>, Without<Enemy>)>()
        .iter(world)
        .collect();
    let mut out = Vec::new();
    for root in roots {
        walk(world, root, &mut out);
    }
    out
}

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

fn named(app: &mut App, name: &str) -> Entity {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &Name)>();
    query
        .iter(world)
        .find(|(_, n)| n.as_str() == name)
        .unwrap()
        .0
}

#[rfn]
fn score_label(score: Reactive<Res<Score>>) -> impl Scene {
    rsn! { Label({ format!("score {}", score.0) }) }
}

#[test]
fn a_declared_param_is_read_by_name() {
    let mut app = app();
    app.world_mut().spawn_scene(score_label()).unwrap();
    assert_eq!(labels(&mut app), ["score 0"]);
    app.world_mut().resource_mut::<Score>().0 = 7;
    app.update();
    assert_eq!(labels(&mut app), ["score 7"]);
}

#[rfn]
fn derived(prefix: &'static str, score: Reactive<Res<Score>>) -> impl Scene {
    let doubled: Reactive<_> = score.0 * 2;
    let text: Reactive<_> = format!("{prefix} {doubled}");
    let shout = prefix.to_uppercase();
    bsn! {
        Node
        Children [
            Label({ text })
            --
            Label({ shout })
            --
            Label({ format!("{prefix} {}", doubled + 1) })
        ]
    }
}

#[test]
fn derived_lets_recompute_and_plain_lets_run_once() {
    let mut app = app();
    app.world_mut().spawn_scene(derived("x")).unwrap();
    assert_eq!(labels(&mut app), ["x 0", "X", "x 1"]);
    app.world_mut().resource_mut::<Score>().0 = 5;
    app.update();
    assert_eq!(labels(&mut app), ["x 10", "X", "x 11"]);
}

#[rfn]
fn branches(tab: Reactive<Res<Tab>>, score: Reactive<Res<Score>>) -> impl Scene {
    let big: Reactive<_> = score.0 > 10;
    rsn! {
        Node
        Children [
            match *tab {
                Tab::Home => Label("home"),
                Tab::Settings => Label("settings"),
            }
            --
            if big { Label("big") } else { Label("small") }
        ]
    }
}

#[test]
fn if_and_match_on_declared_values() {
    let mut app = app();
    app.world_mut().spawn_scene(branches()).unwrap();
    assert_eq!(labels(&mut app), ["home", "small"]);
    *app.world_mut().resource_mut::<Tab>() = Tab::Settings;
    app.world_mut().resource_mut::<Score>().0 = 11;
    app.update();
    assert_eq!(labels(&mut app), ["settings", "big"]);
}

#[rfn]
fn enemies(enemies: Reactive<Query<&Health, With<Enemy>>>) -> impl Scene {
    let alive: Reactive<_> = enemies.iter().filter(|hp| hp.0 > 0).count();
    rsn! {
        Node
        Children [
            Label({ format!("{} alive", alive) })
            --
            for hp in enemies { Label({ format!("hp {}", hp.0) }) }
        ]
    }
}

#[test]
fn queries_loop_and_derive() {
    let mut app = app();
    let a = app.world_mut().spawn((Enemy, Health(3))).id();
    app.world_mut().spawn_scene(enemies()).unwrap();
    assert_eq!(labels(&mut app), ["1 alive", "hp 3"]);
    app.world_mut().spawn((Enemy, Health(0)));
    app.world_mut().get_mut::<Health>(a).unwrap().0 = 4;
    app.update();
    let mut got = labels(&mut app);
    got[1..].sort();
    assert_eq!(got, ["1 alive", "hp 0", "hp 4"]);
}

#[rfn]
fn counter(count: Reactive<Ref<Count>, Of<Counter>>, score: Reactive<Res<Score>>) -> impl Scene {
    rsn! {
        #Counter
        Count(0)
        Children [
            #Button
            on(|poke: On<Poke>| {
                count.0 += poke.by;
                score.0 += 1;
            })
            --
            Label({ format!("{} / {}", count.0, score.0) })
        ]
    }
}

#[test]
fn handlers_write_declared_values() {
    let mut app = app();
    app.world_mut().spawn_scene(counter()).unwrap();
    let button = named(&mut app, "Button");
    app.world_mut().trigger(Poke {
        entity: button,
        by: 2,
    });
    app.world_mut().trigger(Poke {
        entity: button,
        by: 3,
    });
    app.update();
    assert_eq!(labels(&mut app), ["5 / 2"]);
}

#[rfn]
fn health_of(player: Entity, hp: Reactive<Ref<Health>, Of<player>>) -> impl Scene {
    let low: Reactive<_> = hp.0 < 10;
    rsn! {
        Label({ format!("{}{}", hp.0, if low { "!" } else { "" }) })
    }
}

#[test]
fn ref_of_an_entity_argument() {
    let mut app = app();
    let player = app.world_mut().spawn(Health(20)).id();
    app.world_mut().spawn_scene(health_of(player)).unwrap();
    assert_eq!(labels(&mut app), ["20"]);
    app.world_mut().get_mut::<Health>(player).unwrap().0 = 5;
    app.update();
    assert_eq!(labels(&mut app), ["5!"]);
}

#[rfn]
fn counts_calls(
    score: Reactive<Res<Score>>,
    tab: Reactive<Res<Tab>>,
    mut calls: ResMut<Calls>,
) -> impl Scene {
    rsn! {
        Node
        Children [
            Label({ format!("{:?}", *tab) })
            --
            Label({
                calls.0 += 1;
                format!("{}", score.0)
            })
        ]
    }
}

#[test]
fn only_reads_that_changed_re_run() {
    let mut app = app();
    app.world_mut().spawn_scene(counts_calls()).unwrap();
    assert_eq!(app.world().resource::<Calls>().0, 1);
    *app.world_mut().resource_mut::<Tab>() = Tab::Settings;
    app.update();
    assert_eq!(labels(&mut app), ["Settings", "0"]);
    assert_eq!(app.world().resource::<Calls>().0, 1);
    app.world_mut().resource_mut::<Score>().0 = 1;
    app.update();
    assert_eq!(app.world().resource::<Calls>().0, 2);
}

#[rfn]
fn strings(name: String, score: Reactive<Res<Score>>) -> impl Scene {
    let greeting = format!("hi {name}");
    rsn! {
        Node
        Children [
            Label({ greeting.clone() })
            --
            Label({ format!("{greeting}, {name}: {}", score.0) })
            --
            if score.0 > 0 { Label({ name.clone() }) }
        ]
    }
}

#[test]
fn plain_values_can_be_used_many_times() {
    let mut app = app();
    app.world_mut().spawn_scene(strings("ann".into())).unwrap();
    assert_eq!(labels(&mut app), ["hi ann", "hi ann, ann: 0"]);
    app.world_mut().resource_mut::<Score>().0 = 1;
    app.update();
    assert_eq!(labels(&mut app), ["hi ann", "hi ann, ann: 1", "ann"]);
}

#[derive(Resource, Default)]
struct Bonus(Option<u32>);

/// A custom `SystemParam`, read through `Untracked`.
#[derive(bevy::ecs::system::SystemParam)]
struct Totals<'w> {
    score: Res<'w, Score>,
}

#[rfn]
fn optional(
    bonus: Reactive<Option<Res<Bonus>>>,
    score: Reactive<Res<Score>>,
    totals: Untracked<Totals>,
) -> impl Scene {
    let extra: Reactive<_> = bonus.as_ref().and_then(|b| b.0).unwrap_or(0);
    rsn! { Label({ format!("{} + {} ({})", score.0, extra, totals.score.0) }) }
}

#[test]
fn optional_and_custom_params() {
    let mut app = app();
    app.world_mut().spawn_scene(optional()).unwrap();
    assert_eq!(labels(&mut app), ["0 + 0 (0)"]);
    app.insert_resource(Bonus(Some(3)));
    app.world_mut().resource_mut::<Score>().0 = 2;
    app.update();
    assert_eq!(labels(&mut app), ["2 + 3 (2)"]);
}

#[derive(Component, Clone, Copy, Default)]
struct Spawned;

#[rfn]
fn untracked(
    mut commands: Commands,
    score: Res<Score>,
    mut count: Ref<Count, Of<Root>>,
    tab: Reactive<Res<Tab>>,
) -> impl Scene {
    rsn! {
        #Root
        Count(0)
        Children [
            // Untracked: read when this label renders, but a change doesn't re-run it.
            Label({ format!("{:?} {}", *tab, score.0) })
            --
            #Button
            on(|_: On<Poke>| {
                commands.spawn(Spawned);
                count.0 += 1;
            })
        ]
    }
}

#[test]
fn unmarked_params_are_untracked() {
    let mut app = app();
    app.world_mut().resource_mut::<Score>().0 = 1;
    app.world_mut().spawn_scene(untracked()).unwrap();
    assert_eq!(labels(&mut app), ["Home 1"]);

    // `score` isn't tracked: nothing re-runs.
    app.world_mut().resource_mut::<Score>().0 = 2;
    app.update();
    assert_eq!(labels(&mut app), ["Home 1"]);

    // `tab` is: the label re-runs and reads the current `score` too.
    *app.world_mut().resource_mut::<Tab>() = Tab::Settings;
    app.update();
    assert_eq!(labels(&mut app), ["Settings 2"]);

    let button = named(&mut app, "Button");
    app.world_mut().trigger(Poke {
        entity: button,
        by: 0,
    });
    app.update();
    let world = app.world_mut();
    assert_eq!(world.query::<&Spawned>().iter(world).count(), 1);
    let root = named(&mut app, "Root");
    assert_eq!(app.world().get::<Count>(root), Some(&Count(1)));
}

#[derive(Resource, Default)]
struct Spawned2(u32);

/// `#[rfn]` on a system: parameters the body uses stay system parameters.
#[rfn]
fn setup_ui(mut commands: Commands, score: Reactive<Res<Score>>, mut spawned: ResMut<Spawned2>) {
    let start = score.0;
    spawned.0 += 1;
    commands.spawn_scene(rsn! {
        Node
        Children [
            Label({ format!("start {start}, now {}", score.0) })
            --
            #Button
            on(|_: On<Poke>| { commands.spawn(Spawned); })
        ]
    });
}

#[test]
fn rfn_systems_keep_the_parameters_their_body_uses() {
    let mut app = app();
    app.init_resource::<Spawned2>()
        .add_systems(Startup, setup_ui);
    app.world_mut().resource_mut::<Score>().0 = 3;
    app.update();
    app.update();
    assert_eq!(app.world().resource::<Spawned2>().0, 1);
    assert_eq!(labels(&mut app), ["start 3, now 3"]);
    app.world_mut().resource_mut::<Score>().0 = 4;
    app.update();
    assert_eq!(labels(&mut app), ["start 3, now 4"]);

    let button = named(&mut app, "Button");
    app.world_mut().trigger(Poke {
        entity: button,
        by: 0,
    });
    app.update();
    let world = app.world_mut();
    assert_eq!(world.query::<&Spawned>().iter(world).count(), 1);
}

#[rfn]
fn by_runtime_name(count: Reactive<Ref<Count>, Of<"Lookup">>) -> impl Scene {
    rsn! {
        #Lookup
        Count(4)
        Children [ Label({ format!("count {}", count.0) }) ]
    }
}

#[test]
fn of_a_string_looks_the_name_up_at_runtime() {
    let mut app = app();
    app.world_mut().spawn_scene(by_runtime_name()).unwrap();
    assert_eq!(labels(&mut app), ["count 4"]);
}
