//! A settings panel whose widgets are bound to a resource and a component
//! with `bind(..)`, plus a button handler written with `on(..)`.
//!
//! The example runs headless: it plays the part of the user by sending the
//! events the widgets send when they are edited.

use bevy::{
    asset::AssetPlugin,
    prelude::*,
    scene::ScenePlugin,
    ui_widgets::{Activate, Button, Checkbox, Slider, SliderRange, TextInput, ValueChange},
};
use bevy_radioactive::reactive::*;

#[derive(Resource, Clone, Default, Debug)]
struct Settings {
    volume: f32,
    muted: bool,
    name: String,
}

#[derive(Component, Clone, Default)]
struct Brightness(f32);

#[derive(Component, Clone, Default)]
struct Label(String);

/// Marks the sliders, so `main` can find them.
#[derive(Component, Clone, Default)]
struct VolumeSlider;

#[derive(Component, Clone, Default)]
struct BrightnessSlider;

fn settings() -> impl Scene {
    rsn! {
        #Panel
        Brightness(0.5)
        Children [
            // Shows `volume` and writes the slider's edits back to it.
            Slider SliderRange::new(0.0, 1.0) VolumeSlider bind($Res<Settings>.volume)
            --
            Checkbox bind($Res<Settings>.muted)
            --
            TextInput bind($Res<Settings>.name)
            --
            // Components work too: this slider edits the panel's own `Brightness`.
            Slider BrightnessSlider bind($Ref<Brightness>(#Panel).0)
            --
            Button
            on(|_: On<Activate>| {
                let settings = &mut $ResMut<Settings>;
                settings.volume = 0.0;
                settings.muted = true;
            })
            --
            Label({ format!(
                "{} hears {:.0}%{}",
                $Res<Settings>.name,
                $Res<Settings>.volume * 100.0,
                if $Res<Settings>.muted { " (muted)" } else { "" },
            ) })
            --
            Label({ format!("brightness {:.1}", $Ref<Brightness>(#Panel).0) })
        ]
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .insert_resource(Settings {
        volume: 0.8,
        muted: false,
        name: "Player".into(),
    });
    app.world_mut().spawn_scene(settings()).unwrap();
    app.update();
    print_labels(&mut app, "spawned");

    // The user drags the volume slider, then the brightness slider.
    let volume = find::<VolumeSlider>(&mut app);
    let brightness = find::<BrightnessSlider>(&mut app);
    edit(&mut app, volume, 0.3_f32);
    edit(&mut app, brightness, 0.9_f32);
    print_labels(&mut app, "after dragging both sliders");

    // The user clicks the button.
    let button = find::<Button>(&mut app);
    app.world_mut().trigger(Activate { entity: button });
    app.update();
    print_labels(&mut app, "after clicking reset");

    // Unticking the checkbox writes `muted` back.
    let checkbox = find::<Checkbox>(&mut app);
    edit(&mut app, checkbox, false);
    print_labels(&mut app, "after unticking mute");
}

fn edit<T: Send + Sync + 'static>(app: &mut App, source: Entity, value: T) {
    app.world_mut().trigger(ValueChange {
        source,
        value,
        is_final: true,
    });
    app.update();
}

fn find<C: Component>(app: &mut App) -> Entity {
    let world = app.world_mut();
    world
        .query_filtered::<Entity, With<C>>()
        .single(world)
        .unwrap()
}

fn print_labels(app: &mut App, when: &str) {
    println!("{when}:");
    let world = app.world_mut();
    for label in world.query::<&Label>().iter(world) {
        println!("  {}", label.0);
    }
}
