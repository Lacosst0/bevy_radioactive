//! Counter in plain Bevy, the counterpart of `subscription_radioactive`.
//!
//! There is no shared state: the label owns its count. Without `subscribe_to`,
//! each button's own observer has to find the label by a marker component, and
//! the label's `CountChanged` observer has to find the Reset button the same way.

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

/// The count, owned by the label.
#[derive(Component, Clone, Default)]
struct LabelCount(i32);

/// Marks the label that owns and shows the count.
#[derive(Component, Clone, Default)]
struct CountLabel;

/// Marks the Reset button, shown while the count is not zero.
#[derive(Component, Clone, Default)]
struct ResetButton;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FeathersPlugins))
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Startup, scene.spawn())
        .run();
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
                on(|_: On<Activate>, labels: CountLabels, commands: Commands| {
                    update_count(labels, commands, |n| n - 1);
                })
                Children [ @caption("-1") ]
                --
                CountLabel
                LabelCount(0)
                Text("0")
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                // The label announces its count; Reset shows itself while it is not zero.
                on(|changed: On<CountChanged>, mut resets: Query<&mut Node, With<ResetButton>>| {
                    for mut node in &mut resets {
                        node.display = if changed.count != 0 { Display::Flex } else { Display::None };
                    }
                })
                --
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(|_: On<Activate>, labels: CountLabels, commands: Commands| {
                    update_count(labels, commands, |n| n + 1);
                })
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
            on(|_: On<Activate>, labels: CountLabels, commands: Commands| {
                update_count(labels, commands, |_| 0);
            })
            Children [ @caption("Reset") ]
        ]
    }
}

/// Emitted by the label whenever its count changes.
#[derive(EntityEvent, Clone)]
struct CountChanged {
    #[event_target]
    label: Entity,
    count: i32,
}

type CountLabels<'w, 's> =
    Query<'w, 's, (Entity, &'static mut LabelCount, &'static mut Text), With<CountLabel>>;

fn update_count(mut labels: CountLabels, mut commands: Commands, f: impl Fn(i32) -> i32) {
    for (label, mut count, mut text) in &mut labels {
        count.0 = f(count.0);
        **text = count.0.to_string();
        commands.trigger(CountChanged {
            label,
            count: count.0,
        });
    }
}
