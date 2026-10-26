//! Integration coverage for two-way widget bindings (feature `widgets`).

use bevy::{
    asset::AssetPlugin,
    prelude::*,
    scene::ScenePlugin,
    ui::Checked,
    ui_widgets::{
        Checkbox, RadioButton, RadioGroup, SelectedTab, Slider, SliderValue, Tab, TabList,
        ValueChange,
    },
};
use bevy_radioactive::{
    RadioactivePlugin, Reads,
    coat::{Signal, signal},
    testing::{settle, settle_n},
    widgets::{bind_radio, bind_slider, bind_tabs, bind_widget, bind_widget_with},
};

#[derive(Resource, Clone, Copy, Default)]
struct Volume(f32);

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

fn request<T: Clone + Send + Sync + 'static>(
    app: &mut App,
    source: Entity,
    value: T,
    final_: bool,
) {
    app.world_mut().trigger(ValueChange {
        source,
        value,
        is_final: final_,
    });
    app.world_mut().flush();
}

fn slider_value(app: &App, entity: Entity) -> f32 {
    app.world().get::<SliderValue>(entity).unwrap().0
}

#[test]
fn slider_follows_its_signal_and_writes_it_back() {
    let mut app = app();
    let volume = signal(|| 0.25_f32);
    let model = volume.clone();
    let slider = app
        .world_mut()
        .spawn_scene(bsn! { Slider bind_slider(model.clone()) })
        .unwrap()
        .id();
    settle(&mut app);
    assert!((slider_value(&app, slider) - 0.25).abs() < f32::EPSILON);

    // The widget asks for a value: the signal takes it, then the state.
    request(&mut app, slider, 0.75_f32, false);
    assert!((volume.get_untracked() - 0.75).abs() < f32::EPSILON);
    settle(&mut app);
    assert!((slider_value(&app, slider) - 0.75).abs() < f32::EPSILON);

    // The app changes the model: the widget follows.
    volume.set(0.5);
    settle(&mut app);
    assert!((slider_value(&app, slider) - 0.5).abs() < f32::EPSILON);
}

#[test]
fn generic_binding_and_commit_on_final_with_ecs_state() {
    let mut app = app();
    app.insert_resource(Volume(0.1));
    let slider = app
        .world_mut()
        .spawn_scene(bsn! {
            Slider
            bind_widget_with::<Slider, _, _>(
                |r: &Reads| r.resource::<Volume>().map(|v| v.0),
                |world: &mut World, v| world.resource_mut::<Volume>().0 = v,
            ).commit_on_final()
        })
        .unwrap()
        .id();
    settle(&mut app);
    assert!((slider_value(&app, slider) - 0.1).abs() < f32::EPSILON);

    // Mid-drag values move the widget but not the model.
    request(&mut app, slider, 0.4_f32, false);
    settle(&mut app);
    assert!((slider_value(&app, slider) - 0.4).abs() < f32::EPSILON);
    assert!((app.world().resource::<Volume>().0 - 0.1).abs() < f32::EPSILON);

    // The final value commits.
    request(&mut app, slider, 0.6_f32, true);
    settle(&mut app);
    assert!((app.world().resource::<Volume>().0 - 0.6).abs() < f32::EPSILON);
    assert!((slider_value(&app, slider) - 0.6).abs() < f32::EPSILON);

    app.world_mut().resource_mut::<Volume>().0 = 0.9;
    settle(&mut app);
    assert!((slider_value(&app, slider) - 0.9).abs() < f32::EPSILON);
}

#[test]
fn checkbox_binding_inserts_and_removes_checked() {
    let mut app = app();
    let muted: Signal<bool> = signal(|| false);
    let model = muted.clone();
    let checkbox = app
        .world_mut()
        .spawn_scene(bsn! { Checkbox bind_widget::<Checkbox, _>(model.clone()) })
        .unwrap()
        .id();
    settle(&mut app);
    assert!(!app.world().entity(checkbox).contains::<Checked>());

    request(&mut app, checkbox, true, true);
    settle(&mut app);
    assert!(muted.get_untracked());
    assert!(app.world().entity(checkbox).contains::<Checked>());

    muted.set(false);
    settle(&mut app);
    assert!(!app.world().entity(checkbox).contains::<Checked>());
}

#[test]
fn radio_binding_moves_checked_between_buttons() {
    let mut app = app();
    let choice: Signal<Option<Entity>> = signal(|| None);
    let model = choice.clone();
    let group = app
        .world_mut()
        .spawn_scene(bsn! {
            RadioGroup
            bind_radio(model.clone())
            Children [ RadioButton -- RadioButton ]
        })
        .unwrap()
        .id();
    settle(&mut app);
    let buttons = app.world().get::<Children>(group).unwrap().to_vec();
    let checked = |app: &App| {
        buttons
            .iter()
            .map(|&button| app.world().entity(button).contains::<Checked>())
            .collect::<Vec<_>>()
    };
    assert_eq!(checked(&app), [false, false]);

    request(&mut app, group, buttons[1], true);
    settle(&mut app);
    assert_eq!(choice.get_untracked(), Some(buttons[1]));
    assert_eq!(checked(&app), [false, true]);

    choice.set(Some(buttons[0]));
    settle(&mut app);
    assert_eq!(checked(&app), [true, false]);

    // A button added later picks up the state.
    let late = app.world_mut().spawn((RadioButton, ChildOf(group))).id();
    choice.set(Some(late));
    settle(&mut app);
    assert!(app.world().entity(late).contains::<Checked>());
    assert_eq!(checked(&app), [false, false]);
}

#[test]
fn despawned_widgets_leave_no_bindings_behind() {
    let mut app = app();
    let volume = signal(|| 0.3_f32);
    let model = volume.clone();
    settle(&mut app);
    let before = app.world().entities().count_spawned();
    let slider = app
        .world_mut()
        .spawn_scene(bsn! { Slider bind_slider(model.clone()) })
        .unwrap()
        .id();
    settle(&mut app);
    assert!(
        app.world().entities().count_spawned() > before + 1,
        "observer entity"
    );

    app.world_mut().despawn(slider);
    settle_n(&mut app, 2);
    assert_eq!(app.world().entities().count_spawned(), before);

    // The model keeps working without its widget.
    volume.set(0.8);
    settle(&mut app);
    assert!((volume.get_untracked() - 0.8).abs() < f32::EPSILON);
}

#[test]
fn tab_list_binding_writes_the_selected_tab() {
    let mut app = app();
    let tab: Signal<Option<Entity>> = signal(|| None);
    let model = tab.clone();
    let list = app
        .world_mut()
        .spawn_scene(bsn! { TabList bind_tabs(model.clone()) Children [ Tab -- Tab ] })
        .unwrap()
        .id();
    settle(&mut app);
    let tabs = app.world().get::<Children>(list).unwrap().to_vec();
    assert_eq!(
        app.world().get::<SelectedTab>(list),
        Some(&SelectedTab(None))
    );

    request(&mut app, list, Some(tabs[1]), true);
    settle(&mut app);
    assert_eq!(tab.get_untracked(), Some(tabs[1]));
    assert_eq!(
        app.world().get::<SelectedTab>(list),
        Some(&SelectedTab(Some(tabs[1])))
    );

    tab.set(None);
    settle(&mut app);
    assert_eq!(
        app.world().get::<SelectedTab>(list),
        Some(&SelectedTab(None))
    );
}
