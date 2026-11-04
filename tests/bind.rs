#![cfg(all(feature = "reactive", feature = "widgets"))]
//! `bind`: two-way binding between widgets and resources or components.
#![allow(clippy::float_cmp, reason = "the values are exact")]

use bevy::{
    asset::AssetPlugin,
    prelude::*,
    scene::ScenePlugin,
    text::{EditableText, TextEdit},
    ui::Checked,
    ui_widgets::{Activate, Button, Checkbox, Slider, SliderValue, TextInput, ValueChange},
};
use bevy_radioactive::reactive::*;

#[derive(Resource, Clone, Copy, Default, PartialEq, Debug)]
struct Volume(f32);

#[derive(Resource, Clone, Default, PartialEq, Debug)]
struct Settings {
    muted: bool,
    name: String,
}

#[derive(Component, Clone, Copy, Default, PartialEq, Debug)]
struct Brightness(f32);

#[derive(Component, Clone, Copy, Default, PartialEq, Debug)]
struct Clicks(u32);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Volume>()
    .init_resource::<Settings>();
    app
}

fn single<C: Component>(app: &mut App) -> Entity {
    let world = app.world_mut();
    world
        .query_filtered::<Entity, With<C>>()
        .single(world)
        .unwrap()
}

fn change<T: Send + Sync + 'static>(app: &mut App, source: Entity, value: T) {
    app.world_mut().trigger(ValueChange {
        source,
        value,
        is_final: true,
    });
    app.update();
}

#[test]
fn slider_follows_a_resource_and_writes_it_back() {
    let mut app = app();
    app.world_mut().resource_mut::<Volume>().0 = 0.25;
    app.world_mut()
        .spawn_scene(rsn! {
            Node
            Children [
                Slider
                bind($Res<Volume>.0)
            ]
        })
        .unwrap();
    app.update();
    let slider = single::<Slider>(&mut app);
    assert_eq!(
        app.world().get::<SliderValue>(slider),
        Some(&SliderValue(0.25))
    );

    app.world_mut().resource_mut::<Volume>().0 = 0.5;
    app.update();
    assert_eq!(
        app.world().get::<SliderValue>(slider),
        Some(&SliderValue(0.5))
    );

    change(&mut app, slider, 0.75_f32);
    assert_eq!(app.world().resource::<Volume>().0, 0.75);
    assert_eq!(
        app.world().get::<SliderValue>(slider),
        Some(&SliderValue(0.75))
    );
}

#[test]
fn an_unchanged_value_is_not_written() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! { Slider bind($ResMut<Volume>.0) })
        .unwrap();
    app.update();
    let slider = single::<Slider>(&mut app);
    let before = app.world().resource_ref::<Volume>().last_changed();
    change(&mut app, slider, 0.0_f32);
    let after = app.world().resource_ref::<Volume>().last_changed();
    assert_eq!(
        before, after,
        "writing the same value marked the resource changed"
    );
}

#[test]
fn checkbox_binds_a_resource_field() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! { Checkbox bind($Res<Settings>.muted) })
        .unwrap();
    app.update();
    let checkbox = single::<Checkbox>(&mut app);
    assert!(!app.world().entity(checkbox).contains::<Checked>());

    app.world_mut().resource_mut::<Settings>().muted = true;
    app.update();
    assert!(app.world().entity(checkbox).contains::<Checked>());

    change(&mut app, checkbox, false);
    assert!(!app.world().resource::<Settings>().muted);
    assert!(!app.world().entity(checkbox).contains::<Checked>());
}

#[test]
fn slider_binds_a_named_component() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Lamp
            Brightness(0.5)
            Children [
                Slider bind($Ref<Brightness>(#Lamp).0)
            ]
        })
        .unwrap();
    app.update();
    let slider = single::<Slider>(&mut app);
    let lamp = single::<Brightness>(&mut app);
    assert_eq!(
        app.world().get::<SliderValue>(slider),
        Some(&SliderValue(0.5))
    );

    change(&mut app, slider, 0.9_f32);
    assert_eq!(app.world().get::<Brightness>(lamp), Some(&Brightness(0.9)));

    app.world_mut().get_mut::<Brightness>(lamp).unwrap().0 = 0.1;
    app.update();
    assert_eq!(
        app.world().get::<SliderValue>(slider),
        Some(&SliderValue(0.1))
    );
}

#[test]
fn bound_rows_in_a_query_loop() {
    let mut app = app();
    let a = app.world_mut().spawn(Brightness(0.1)).id();
    let b = app.world_mut().spawn(Brightness(0.2)).id();
    app.world_mut()
        .spawn_scene(rsn! {
            Node
            Children [
                for lamp in $Query<Entity, With<Brightness>> {
                    Slider bind($Ref<Brightness>(lamp).0)
                }
            ]
        })
        .unwrap();
    app.update();
    let sliders: Vec<(Entity, f32)> = {
        let world = app.world_mut();
        world
            .query::<(Entity, &SliderValue)>()
            .iter(world)
            .map(|(entity, value)| (entity, value.0))
            .collect()
    };
    assert_eq!(sliders.len(), 2);
    let (slider_b, _) = sliders.iter().find(|(_, value)| *value == 0.2).unwrap();
    change(&mut app, *slider_b, 0.8_f32);
    assert_eq!(app.world().get::<Brightness>(b), Some(&Brightness(0.8)));
    assert_eq!(app.world().get::<Brightness>(a), Some(&Brightness(0.1)));
}

#[test]
fn text_input_binds_a_string() {
    let mut app = app();
    app.add_plugins(bevy::text::TextPlugin);
    app.world_mut().resource_mut::<Settings>().name = "Ada".into();
    app.world_mut()
        .spawn_scene(rsn! { TextInput bind($Res<Settings>.name) })
        .unwrap();
    app.update();
    let input = single::<TextInput>(&mut app);
    let text = |app: &App| {
        app.world()
            .get::<EditableText>(input)
            .unwrap()
            .value()
            .to_string()
    };
    assert_eq!(text(&app), "Ada");

    app.world_mut()
        .get_mut::<EditableText>(input)
        .unwrap()
        .queue_edit(TextEdit::Insert("!".into()));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<Settings>().name, "Ada!");

    app.world_mut().resource_mut::<Settings>().name = "Grace".into();
    app.update();
    assert_eq!(text(&app), "Grace");
}

#[test]
fn plain_bind_takes_a_getter_and_a_setter() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(bsn! {
            Slider
            bind(
                |volume: Res<Volume>| volume.0 * 4.0,
                |value: &f32, mut volume: ResMut<Volume>| volume.0 = *value / 4.0,
            )
        })
        .unwrap();
    app.world_mut().resource_mut::<Volume>().0 = 0.25;
    app.update();
    let slider = single::<Slider>(&mut app);
    assert_eq!(
        app.world().get::<SliderValue>(slider),
        Some(&SliderValue(1.0))
    );
    change(&mut app, slider, 2.0_f32);
    assert_eq!(app.world().resource::<Volume>().0, 0.5);
}

/// A custom widget registered with `register_bindable`.
#[derive(Component, Clone, Default)]
struct Stepper;

#[derive(Component, Clone, Copy, Default, PartialEq, Debug)]
struct StepperShown(i32);

impl Bindable for Stepper {
    type Value = i32;
    type Event = ValueChange<i32>;

    fn show(entity: &mut EntityWorldMut, value: &i32) {
        entity.insert(StepperShown(*value));
    }

    fn value(event: &ValueChange<i32>, _: EntityRef) -> Option<i32> {
        Some(event.value)
    }
}

#[derive(Resource, Clone, Copy, Default)]
struct Lives(i32);

#[test]
fn custom_widgets_register_with_the_app() {
    let mut app = app();
    app.register_bindable::<Stepper>().insert_resource(Lives(3));
    app.world_mut()
        .spawn_scene(rsn! { Stepper bind($Res<Lives>.0) })
        .unwrap();
    app.update();
    let stepper = single::<Stepper>(&mut app);
    assert_eq!(
        app.world().get::<StepperShown>(stepper),
        Some(&StepperShown(3))
    );
    change(&mut app, stepper, 5_i32);
    assert_eq!(app.world().resource::<Lives>().0, 5);
    assert_eq!(
        app.world().get::<StepperShown>(stepper),
        Some(&StepperShown(5))
    );
}

#[test]
fn button_handlers_write_components_and_resources() {
    let mut app = app();
    app.world_mut()
        .spawn_scene(rsn! {
            #Counter
            Clicks(0)
            Children [
                Button
                on(|_: On<Activate>| {
                    $Mut<Clicks>(#Counter).0 += 1;
                    $ResMut<Volume>.0 += 0.1;
                })
            ]
        })
        .unwrap();
    app.update();
    let button = single::<Button>(&mut app);
    app.world_mut().trigger(Activate { entity: button });
    app.world_mut().trigger(Activate { entity: button });
    app.update();
    let counter = single::<Clicks>(&mut app);
    assert_eq!(app.world().get::<Clicks>(counter), Some(&Clicks(2)));
    assert!((app.world().resource::<Volume>().0 - 0.2).abs() < 1e-6);
}
