//! Counter using `rfn`: the count is an argument, `$count` reads it.
//!
//! Plain Bevy version: `comparison_bevy`.
//!
//! `Count` lives on the counter's root node, so every `@counter()` has its
//! own. The `#[from("Counter")] count: Ref<Count>` argument reads it: buttons
//! write `$count` in `on(..)`, the label re-renders when it changes, and the
//! Reset button exists only while the count is not zero.

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
use bevy_radioactive::rfn::*;

#[derive(Component, Clone, Default)]
struct Count(i32);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FeathersPlugins, ReactivePlugin))
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Startup, scene.spawn())
        .run();
}

fn scene() -> impl SceneList {
    bsn_list! { Camera2d -- @counter() }
}

#[rfn]
fn counter(#[from("Counter")] count: Ref<Count>) -> impl Scene {
    let label = count.0.to_string();
    rsn! {
        #Counter
        Count(0)
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
                on(|_: On<Activate>| $count.0 -= 1)
                Children [ @caption("-1") ]
                --
                Text({ $label })
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                --
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(|_: On<Activate>| $count.0 += 1)
                Children [ @caption("+1") ]
            ]
            --
            if $count.0 != 0 {
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(|_: On<Activate>| $count.0 = 0)
                Children [ @caption("Reset") ]
            }
        ]
    }
}
