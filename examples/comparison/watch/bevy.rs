//! Counter in plain Bevy, the counterpart of `watch_radioactive`.
//!
//! The count is a resource. Buttons mutate it; instead of `bind`, global
//! systems gated on `resource_changed` find the label and the Reset button by
//! marker components and write to them.

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

#[derive(Resource, Clone, Default)]
struct Count(i32);

/// Marks the text that shows the count.
#[derive(Component, Clone, Default)]
struct CountLabel;

/// Marks the Reset button, shown while the count is not zero.
#[derive(Component, Clone, Default)]
struct ResetButton;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FeathersPlugins))
        .insert_resource(UiTheme(create_dark_theme()))
        .init_resource::<Count>()
        .add_systems(Startup, scene.spawn())
        .add_systems(
            Update,
            (sync_count_label, sync_reset_button).run_if(resource_changed::<Count>),
        )
        .run();
}

#[allow(clippy::needless_pass_by_value)]
fn sync_count_label(count: Res<Count>, mut labels: Query<&mut Text, With<CountLabel>>) {
    for mut text in &mut labels {
        **text = count.0.to_string();
    }
}

#[allow(clippy::needless_pass_by_value)]
fn sync_reset_button(count: Res<Count>, mut buttons: Query<&mut Node, With<ResetButton>>) {
    for mut node in &mut buttons {
        node.display = if count.0 != 0 {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn scene() -> impl SceneList {
    bsn_list! { Camera2d -- @counter() }
}

fn counter() -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: vmin(4),
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        Children [
            Node { align_items: AlignItems::Center, column_gap: vmin(6) }
            Children [
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(|_: On<Activate>, mut count: ResMut<Count>| count.0 -= 1)
                Children [ @caption("-1") ]
                --
                CountLabel
                Text("0")
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                --
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(|_: On<Activate>, mut count: ResMut<Count>| count.0 += 1)
                Children [ @caption("+1") ]
            ]
            --
            ResetButton
            @FeathersButton
            Node {
                display: Display::None,
                height: Val::Auto,
                min_width: vmin(18),
                padding: UiRect::axes(vmin(3), vmin(1.5)),
            }
            InheritableFont { font_size: FontSize::VMin(6.0) }
            on(|_: On<Activate>, mut count: ResMut<Count>| count.0 = 0)
            Children [ @caption("Reset") ]
        ]
    }
}
