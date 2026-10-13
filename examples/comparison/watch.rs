//! Counter using `watch`: a resource plus `bind`.
//!
//! Plain Bevy version: `comparison_bevy`.
//!
//! The count is a resource. Buttons mutate it; the label binds to it. `bind`
//! tracks `Count` automatically, so no `app.track::<Count>()` is needed.

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
                bind(Singleton, |count: &Count, text: &mut Text| **text = count.0.to_string())
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
            bind(Singleton, |count: &Count, node: &mut Node| {
                node.display = if count.0 != 0 { Display::Flex } else { Display::None };
            })
            Children [ @caption("Reset") ]
        ]
    }
}
