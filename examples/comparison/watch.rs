//! Counter using `watch`: a resource plus watch systems.
//!
//! Plain Bevy version: `comparison_bevy`.
//!
//! The count is a resource. Buttons mutate it; the label and the Reset button
//! each run a `watch` system that reads `Res<Count>`, so they re-run whenever
//! it changes. Nothing is registered up front.

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
use bevy_radioactive::watch::*;

#[derive(Resource, Clone, Default)]
struct Count(i32);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FeathersPlugins, WatchPlugin))
        .insert_resource(UiTheme(create_dark_theme()))
        .init_resource::<Count>()
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
                on(|_: On<Activate>, mut count: ResMut<Count>| count.0 -= 1)
                Children [ @caption("-1") ]
                --
                Text("0")
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                watch(|count: Res<Count>, mut text: This<&mut Text>| {
                    if let Some(mut text) = text.get_mut() {
                        **text = count.0.to_string();
                    }
                })
                --
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(|_: On<Activate>, mut count: ResMut<Count>| count.0 += 1)
                Children [ @caption("+1") ]
            ]
            --
            @FeathersButton
            Node {
                display: Display::None,
                height: Val::Auto,
                min_width: vmin(18),
                padding: UiRect::axes(vmin(3), vmin(1.5)),
            }
            InheritableFont { font_size: FontSize::VMin(6.0) }
            on(|_: On<Activate>, mut count: ResMut<Count>| count.0 = 0)
            watch(|count: Res<Count>, mut node: This<&mut Node>| {
                if let Some(mut node) = node.get_mut() {
                    node.display = if count.0 != 0 { Display::Flex } else { Display::None };
                }
            })
            Children [ @caption("Reset") ]
        ]
    }
}
