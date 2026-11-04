#![cfg(feature = "reactive")]
//! `rsn!`: `bsn!` with Rust control flow and `$` reads.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Count(i32);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Score(u32);

#[derive(Component, Clone, Default, Debug, PartialEq, Eq)]
struct Label(String);

#[derive(Component, Clone, Default, Debug, PartialEq)]
struct Style {
    width: f32,
    title: String,
}

#[derive(Component, Clone, Copy, Default)]
struct Row;

#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    title: String,
}

#[derive(Resource, Clone, Default)]
struct Todos(Vec<Todo>);

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Tab {
    #[default]
    Home,
    Settings,
    Profile(u32),
}

#[derive(Clone, EntityEvent)]
struct Poke {
    entity: Entity,
    by: i32,
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Score>()
    .init_resource::<Todos>()
    .init_resource::<Tab>();
    app
}

/// Labels in tree order.
fn labels(app: &mut App) -> Vec<String> {
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

fn entities_with<T: Component>(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let mut query = world.query_filtered::<Entity, With<T>>();
    query.iter(world).collect()
}

fn root(app: &mut App) -> Entity {
    entities_with::<Count>(app)[0]
}

fn set_count(app: &mut App, value: i32) {
    let root = root(app);
    app.world_mut().get_mut::<Count>(root).unwrap().0 = value;
    app.update();
}

#[test]
fn dollar_in_a_component_value_watches_it() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(1)
            Children [
                Label({ format!("count {}", $Ref<Count>(#Root).0) })
                --
                Label({ format!("score {}", $Res<Score>.0) })
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["count 1", "score 0"]);

    set_count(&mut app, 2);
    app.world_mut().resource_mut::<Score>().0 = 9;
    app.update();
    assert_eq!(labels(&mut app), ["count 2", "score 9"]);
}

#[test]
fn braced_components_patch_only_reactive_fields() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(3)
            Children [ Style { width: $Ref<Count>(#Root).0 as f32 * 2.0, title: "fixed" } ]
        })
        .unwrap();
    let style = entities_with::<Style>(&mut app)[0];
    assert_eq!(
        app.world().get::<Style>(style),
        Some(&Style {
            width: 6.0,
            title: "fixed".into()
        })
    );
    set_count(&mut app, 5);
    assert_eq!(
        app.world().get::<Style>(style),
        Some(&Style {
            width: 10.0,
            title: "fixed".into()
        })
    );
}

#[test]
fn if_else_if_else_switches_branches() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                Label("before")
                --
                if $Ref<Count>(#Root).0 > 0 {
                    Label("positive")
                } else if $Ref<Count>(#Root).0 < 0 {
                    Label("negative") -- Label("really")
                } else {
                    Label("zero")
                }
                --
                if $Ref<Count>(#Root).0 > 100 { Label("huge") }
                --
                Label("after")
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["before", "zero", "after"]);

    set_count(&mut app, 3);
    assert_eq!(labels(&mut app), ["before", "positive", "after"]);
    set_count(&mut app, -1);
    assert_eq!(labels(&mut app), ["before", "negative", "really", "after"]);
    set_count(&mut app, 101);
    assert_eq!(labels(&mut app), ["before", "positive", "huge", "after"]);
}

#[test]
fn match_on_a_resource_with_bindings() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                match *$Res<Tab> {
                    Tab::Home => Label("home"),
                    Tab::Settings => { Label("settings") -- Label("more") }
                    Tab::Profile(id) => Label({ format!("profile {id}") }) Row,
                }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["home"]);

    *app.world_mut().resource_mut::<Tab>() = Tab::Settings;
    app.update();
    assert_eq!(labels(&mut app), ["settings", "more"]);

    *app.world_mut().resource_mut::<Tab>() = Tab::Profile(7);
    app.update();
    assert_eq!(labels(&mut app), ["profile 7"]);
}

fn todo(id: u32, title: &str) -> Todo {
    Todo {
        id,
        title: title.into(),
    }
}

#[test]
fn keyed_for_keeps_rows_and_uses_the_item_more_than_once() {
    let mut app = app();
    app.world_mut().resource_mut::<Todos>().0 = vec![todo(1, "a"), todo(2, "b")];
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                #[key(todo.id)]
                for todo in $Res<Todos>.0.clone() {
                    Row
                    Label({ todo.title.clone() })
                    Children [
                        Label({ format!("#{} {}", todo.id, todo.title) })
                        --
                        if i64::from($Ref<Count>(#Root).0) > i64::from(todo.id) { Label({ format!("done {}", todo.title) }) }
                    ]
                }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["a", "#1 a", "b", "#2 b"]);
    let rows = entities_with::<Row>(&mut app);

    app.world_mut().resource_mut::<Todos>().0 = vec![todo(2, "b"), todo(1, "a"), todo(3, "c")];
    app.update();
    assert_eq!(labels(&mut app), ["b", "#2 b", "a", "#1 a", "c", "#3 c"]);
    let after = entities_with::<Row>(&mut app);
    assert!(rows.iter().all(|row| after.contains(row)), "rows kept");

    set_count(&mut app, 2);
    assert_eq!(
        labels(&mut app),
        ["b", "#2 b", "a", "#1 a", "done a", "c", "#3 c"]
    );
}

#[test]
fn unkeyed_and_plain_for_loops() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(2)
            Children [
                for i in 0..$Ref<Count>(#Root).0 { Label({ format!("row {i}") }) }
                --
                for name in ["x", "y"] { Label({ name.to_string() }) Row }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["row 0", "row 1", "x", "y"]);
    set_count(&mut app, 3);
    assert_eq!(labels(&mut app), ["row 0", "row 1", "row 2", "x", "y"]);
}

#[test]
fn on_writes_the_nearest_component() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                #Button
                on(|poke: On<Poke>| $Mut<Count>(#Root).0 += poke.by)
                --
                Label({ $Ref<Count>(#Root).0.to_string() })
            ]
        })
        .unwrap();
    let button = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Name)>();
        query
            .iter(world)
            .find(|(_, name)| name.as_str() == "Button")
            .unwrap()
            .0
    };
    app.world_mut().trigger(Poke {
        entity: button,
        by: 2,
    });
    app.world_mut().trigger(Poke {
        entity: button,
        by: 3,
    });
    app.update();
    let root = root(&mut app);
    assert_eq!(app.world().get::<Count>(root), Some(&Count(5)));
    assert_eq!(labels(&mut app), ["5"]);
}

#[test]
fn rsn_list_and_plain_bsn_items_mix() {
    let mut app = app();
    let root = app.world_mut().spawn((Name::new("Root"), Count(1))).id();
    app.world_mut()
        .entity_mut(root)
        .queue_spawn_related_scenes::<Children>(rsn_list! {
            Label("plain")
            --
            // `Root` is named outside this `rsn!`: looked up by name at runtime.
            if $Ref<Count>("Root").0 == 1 { Label("one") } else { Label("other") }
        });
    app.update();
    assert_eq!(labels(&mut app), ["plain", "one"]);
    set_count(&mut app, 2);
    assert_eq!(labels(&mut app), ["plain", "other"]);
}

fn badge(text: &'static str) -> impl Scene {
    rsn! { Label({ format!("{text} {}", $Ref<Count>("Root").0) }) }
}

#[test]
fn scene_functions_in_arms_and_nested_rsn_in_rows() {
    let mut app = app();
    let target = app.world_mut().spawn(Count(42)).id();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(1)
            Children [
                match *$Res<Tab> {
                    Tab::Home => @badge("home"),
                    _ => Label("other"),
                }
                --
                @each(move || vec![target], |&e| e, |&e| rsn! {
                    Label({ format!("source {}", $Ref<Count>(e).0) })
                })
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["home 1", "source 42"]);
    app.world_mut().get_mut::<Count>(target).unwrap().0 = 43;
    app.update();
    assert_eq!(labels(&mut app), ["home 1", "source 43"]);
}

#[derive(Component, Clone, Debug, PartialEq)]
struct Task {
    title: String,
    done: bool,
}

#[derive(Component, Clone, Copy, Default)]
struct Player;

#[test]
fn for_over_a_query_tracks_membership_and_values() {
    let mut app = app();
    let first = app
        .world_mut()
        .spawn(Task {
            title: "a".into(),
            done: false,
        })
        .id();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                for task in $Query<&Task> {
                    Row
                    Label({ format!("{} {}", task.title, if task.done { "x" } else { "-" }) })
                }
                --
                Label({ format!("{} open", $Query<&Task>.iter().filter(|t| !t.done).count()) })
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["a -", "1 open"]);

    app.world_mut().spawn(Task {
        title: "b".into(),
        done: false,
    });
    app.update();
    assert_eq!(labels(&mut app), ["a -", "b -", "2 open"]);

    app.world_mut().get_mut::<Task>(first).unwrap().done = true;
    app.update();
    assert_eq!(labels(&mut app), ["a x", "b -", "1 open"]);

    app.world_mut().entity_mut(first).despawn();
    app.update();
    assert_eq!(labels(&mut app), ["b -", "1 open"]);
}

#[test]
fn query_rows_read_their_entity_with_dollar() {
    let mut app = app();
    let task = app
        .world_mut()
        .spawn(Task {
            title: "a".into(),
            done: false,
        })
        .id();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                // Entity rows: the row is never rebuilt, `$Ref<Task>(task)` reads its entity.
                for task in $Query<Entity, With<Task>> {
                    Label({ $Ref<Task>(task).title.clone() })
                }
            ]
        })
        .unwrap();
    let rows = entities_with::<Label>(&mut app);
    app.world_mut().get_mut::<Task>(task).unwrap().title = "b".into();
    app.update();
    assert_eq!(labels(&mut app), ["b"]);
    assert_eq!(entities_with::<Label>(&mut app), rows, "updated in place");
}

#[test]
fn explicit_sources_entity_and_resource() {
    let mut app = app();
    let player = app.world_mut().spawn((Player, Count(10))).id();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(1)
            Children [
                Label({ format!("{} {} {}", $Ref<Count>(#Root).0, $Ref<Count>(player).0, $Res<Score>.0) })
                --
                Label({ format!("single {}", $Single<&Count, With<Player>>.0) })
                --
                #Button
                on(|poke: On<Poke>| $Mut<Count>(player).0 += poke.by)
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["1 10 0", "single 10"]);

    let button = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Name)>();
        query
            .iter(world)
            .find(|(_, name)| name.as_str() == "Button")
            .unwrap()
            .0
    };
    app.world_mut().trigger(Poke {
        entity: button,
        by: 5,
    });
    app.world_mut().resource_mut::<Score>().0 = 3;
    app.update();
    assert_eq!(labels(&mut app), ["1 15 3", "single 15"]);
}

#[test]
fn bevy_style_params_res_res_mut_and_local() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                #Button
                on(|poke: On<Poke>| {
                    $ResMut<Score>.0 += 1;
                    *$Local<i32> += poke.by;
                    $Mut<Count>(#Root).0 = *$Local<i32>;
                })
                --
                Label({ format!("{} {}", $Res<Score>.0, $Ref<Count>(#Root).0) })
            ]
        })
        .unwrap();
    let button = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Name)>();
        query
            .iter(world)
            .find(|(_, name)| name.as_str() == "Button")
            .unwrap()
            .0
    };
    for by in [2, 3] {
        app.world_mut().trigger(Poke { entity: button, by });
    }
    app.update();
    assert_eq!(
        labels(&mut app),
        ["2 5"],
        "Local keeps its value between runs"
    );
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Wings(u32);

fn bird_counter(start: i32) -> impl Scene {
    rsn! {
        #Counter
        Count({start})
        Children [ Label({ format!("{}", $Ref<Count>(#Counter).0) }) ]
    }
}

#[test]
fn named_reads_find_the_nearest_entity_with_that_name() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                #Bird
                Wings(2)
                --
                // A sibling's subtree, not an ancestor: still found.
                Label({ format!("wings {}", $Ref<Wings>(#Bird).0) })
                --
                @bird_counter(1)
                --
                @bird_counter(7)
            ]
        })
        .unwrap();
    assert_eq!(
        labels(&mut app),
        ["wings 2", "1", "7"],
        "each counter reads its own"
    );

    let bird = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<Wings>>();
        query.single(world).unwrap()
    };
    app.world_mut().get_mut::<Wings>(bird).unwrap().0 = 4;
    app.update();
    assert_eq!(labels(&mut app), ["wings 4", "1", "7"]);
}

impl Label {
    fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    fn suffixed(mut self, suffix: &str) -> Self {
        self.0.push_str(suffix);
        self
    }
}

fn label(text: String) -> Label {
    Label(text)
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Edges {
    left: f32,
    right: f32,
}

#[derive(Component, Clone, Default, Debug, PartialEq)]
struct Boxed {
    edges: Edges,
    width: f32,
}

#[test]
fn dollar_in_constructors_functions_and_method_chains() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(1)
            Children [
                Label::new({ format!("new {}", $Res<Score>.0) }) Row,
                Label::new($Ref<Count>(#Root).0.to_string()).suffixed("!"),
                Label::new("chain").suffixed(if $Res<Score>.0 > 0 { "+" } else { "0" }),
                label(format!("fn {}", $Res<Score>.0))
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["new 0", "1!", "chain0", "fn 0"]);

    set_count(&mut app, 4);
    app.world_mut().resource_mut::<Score>().0 = 2;
    app.update();
    assert_eq!(labels(&mut app), ["new 2", "4!", "chain+", "fn 2"]);
}

#[test]
fn nested_struct_values_patch_only_the_fields_they_name() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Boxed { edges: Edges { left: $Res<Score>.0 as f32, right: 2.0 }, width: 1.0 }
        })
        .unwrap();
    let boxed = entities_with::<Boxed>(&mut app)[0];
    let expected = |left| Boxed {
        edges: Edges { left, right: 2.0 },
        width: 1.0,
    };
    assert_eq!(app.world().get::<Boxed>(boxed), Some(&expected(0.0)));
    app.world_mut().resource_mut::<Score>().0 = 5;
    app.update();
    assert_eq!(app.world().get::<Boxed>(boxed), Some(&expected(5.0)));
}

#[test]
fn comma_separated_lists_with_control_flow() {
    let mut app = app();
    app.world_mut().spawn(Task {
        title: "a".into(),
        done: false,
    });
    app.world_mut().spawn((
        Task {
            title: "hidden".into(),
            done: false,
        },
        Row,
    ));
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                Label("first"),
                if $Ref<Count>(#Root).0 > 0 { Label("on") } else { Label("off") },
                for task in $Query<&Task, Without<Row>> { Label({ task.title.clone() }) },
                match $Ref<Count>(#Root).0 { 0 => Label("zero"), _ => Label("some") }
                Label("last")
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["first", "off", "a", "zero", "last"]);
    set_count(&mut app, 1);
    assert_eq!(labels(&mut app), ["first", "on", "a", "some", "last"]);
}

// ---------------------------------------------------------------------------
// `$` in scene inputs
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Default)]
struct Title(String);

#[allow(clippy::needless_pass_by_value)]
fn greeting(name: String, score: u32) -> impl Scene {
    rsn! { Label({ format!("{name}: {score}") }) }
}

#[test]
fn dollar_in_scene_arguments_rebuilds_the_scene() {
    let mut app = app();
    app.insert_resource(Title("ann".into()));
    app.world_mut()
        .spawn_scene(rsn! {
            #Root
            Count(0)
            Children [
                // Owned values are cloned out of the read, so no `.clone()`.
                @greeting($Res<Title>.0, $Res<Score>.0),
                @greeting("bob".to_string(), 1) Row,
                Row @greeting({ format!("#{}", $Ref<Count>(#Root).0) }, 2)
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["ann: 0", "bob: 1", "#0: 2"]);
    let first = entities_with::<Label>(&mut app);

    app.world_mut().resource_mut::<Score>().0 = 3;
    app.update();
    assert_eq!(labels(&mut app), ["ann: 3", "bob: 1", "#0: 2"]);
    let after = entities_with::<Label>(&mut app);
    assert_eq!(
        after.iter().filter(|e| first.contains(e)).count(),
        2,
        "only the scene whose input changed is rebuilt"
    );
    assert_eq!(
        entities_with::<Row>(&mut app).len(),
        2,
        "the rest of the entity comes back"
    );

    app.insert_resource(Title("cy".into()));
    set_count(&mut app, 5);
    assert_eq!(labels(&mut app), ["cy: 3", "bob: 1", "#5: 2"]);
}

#[test]
fn dollar_scene_arguments_inside_control_flow_and_loops() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                for i in 0..2u32 {
                    @greeting(i.to_string(), $Res<Score>.0 + i)
                },
                if $Res<Score>.0 > 0 {
                    @greeting("big".to_string(), $Res<Score>.0 * 10)
                }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["0: 0", "1: 1"]);
    app.world_mut().resource_mut::<Score>().0 = 2;
    app.update();
    assert_eq!(labels(&mut app), ["0: 2", "1: 3", "big: 20"]);
}

/// A scene component: `value` is a field, so `$` patches it in place, and the
/// scene reads it like any component. `@prefix` is a prop: the scene function
/// takes it, so a `$` there rebuilds the scene.
#[derive(SceneComponent, Clone, Default)]
#[scene(Meter::scene(MeterProps))]
struct Meter {
    value: u32,
    max: u32,
}

#[derive(Default)]
struct MeterProps {
    prefix: String,
}

impl Meter {
    fn scene(props: MeterProps) -> impl Scene {
        let prefix = props.prefix;
        rsn! {
            #Meter
            Children [
                Label({ format!("{prefix}{}/{}", $Ref<Meter>(#Meter).value, $Ref<Meter>(#Meter).max) })
            ]
        }
    }
}

#[test]
fn dollar_in_scene_component_fields_patches_without_rebuilding() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                @Meter { value: $Res<Score>.0, max: 10 },
                @Meter { @prefix: { "hp ".to_string() }, value: 1, max: $Res<Score>.0 + 1 }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["0/10", "hp 1/1"]);
    let before = entities_with::<Label>(&mut app);

    app.world_mut().resource_mut::<Score>().0 = 4;
    app.update();
    assert_eq!(labels(&mut app), ["4/10", "hp 1/5"]);
    assert_eq!(
        entities_with::<Label>(&mut app),
        before,
        "fields patch in place"
    );
}

#[test]
fn dollar_in_scene_component_props_rebuilds() {
    let mut app = app();
    app.insert_resource(Title("a ".into()));
    app.world_mut()
        .spawn_scene(rsn! {
            Children [
                @Meter { @prefix: $Res<Title>.0, value: $Res<Score>.0, max: 9 }
            ]
        })
        .unwrap();
    assert_eq!(labels(&mut app), ["a 0/9"]);
    let before = entities_with::<Label>(&mut app);

    app.world_mut().resource_mut::<Score>().0 = 1;
    app.update();
    assert_eq!(labels(&mut app), ["a 1/9"]);
    assert_eq!(
        entities_with::<Label>(&mut app),
        before,
        "a field change patches"
    );

    app.insert_resource(Title("b ".into()));
    app.update();
    assert_eq!(labels(&mut app), ["b 1/9"]);
    assert_ne!(
        entities_with::<Label>(&mut app),
        before,
        "a prop change rebuilds"
    );
}

#[derive(SceneComponent, Clone, Default)]
#[scene(Badge::scene)]
struct Badge(u32);

impl Badge {
    fn scene() -> impl Scene {
        rsn! {
            #Badge
            Children [ Label({ format!("badge {}", $Ref<Badge>(#Badge).0) }) ]
        }
    }
}

#[test]
fn dollar_in_tuple_scene_components_rebuilds_the_component() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! { @Badge($Res<Score>.0 * 2) })
        .unwrap();
    assert_eq!(labels(&mut app), ["badge 0"]);
    let before = entities_with::<Label>(&mut app);
    app.world_mut().resource_mut::<Score>().0 = 3;
    app.update();
    assert_eq!(labels(&mut app), ["badge 6"]);
    assert_eq!(entities_with::<Label>(&mut app), before);
}
