//! Counter in plain Bevy, the counterpart of `hooks_radioactive`.
//!
//! The count is state on the counter's root entity. Like a React re-render,
//! a system rebuilds the whole subtree from that state whenever it changes;
//! without `hooks` it has to run for every counter and do the respawn itself.

use bevy::{
    feathers::{
        FeathersPlugins,
        controls::FeathersButton,
        dark_theme::create_dark_theme,
        display::caption,
        font_styles::InheritableFont,
        theme::{ThemeBackgroundColor, ThemeTextColor, UiTheme},
        tokens,
    },
    prelude::*,
    ui_widgets::Activate,
};

/// The counter's state, stored on its root entity.
#[derive(Component, Clone, Default)]
struct CounterState(i32);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FeathersPlugins))
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Startup, scene.spawn())
        .add_systems(Update, render_counters)
        .run();
}

fn scene() -> impl SceneList {
    bsn_list! {
        Camera2d
        --
        Node { width: percent(100), height: percent(100) }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        Children [ CounterState(0) ]
    }
}

/// Re-renders every counter whose state was added or changed.
fn render_counters(
    counters: Query<(Entity, &CounterState), Changed<CounterState>>,
    mut commands: Commands,
) {
    for (root, state) in &counters {
        commands
            .entity(root)
            .despawn_children()
            .apply_scene(counter(root, state.0));
    }
}

/// Builds the counter's UI for `count`; the buttons write back to `root`.
fn counter(root: Entity, count: i32) -> impl Scene {
    let set_count = move |states: &mut Query<&mut CounterState>, value: i32| {
        if let Ok(mut state) = states.get_mut(root) {
            state.0 = value;
        }
    };

    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: vmin(4),
        }
        Children [
            Node { align_items: AlignItems::Center, column_gap: vmin(6) }
            Children [
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(move |_: On<Activate>, mut states: Query<&mut CounterState>| set_count(&mut states, count - 1))
                Children [ @caption("-1") ]
                --
                Text({count.to_string()})
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                --
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(move |_: On<Activate>, mut states: Query<&mut CounterState>| set_count(&mut states, count + 1))
                Children [ @caption("+1") ]
            ]
            --
            {(count != 0).then(|| bsn_list! {
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(move |_: On<Activate>, mut states: Query<&mut CounterState>| set_count(&mut states, 0))
                Children [ @caption("Reset") ]
            })}
        ]
    }
}
