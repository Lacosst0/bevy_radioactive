//! Counter as a React-style `webby` component.
//!
//! The count is component state; every change re-renders the component.

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
use bevy_radioactive::{
    RadioactivePlugin,
    webby::{Cx, component},
};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FeathersPlugins, RadioactivePlugin))
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Startup, scene.spawn())
        .run();
}

fn scene() -> impl SceneList {
    bsn_list! {
        Camera2d
        --
        Node { width: percent(100), height: percent(100) }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        @counter()
    }
}

#[component]
fn counter(cx: &Cx) -> impl Scene {
    let (count, set_count) = cx.use_state(|| 0);
    let (minus, plus, reset) = (set_count.clone(), set_count.clone(), set_count);

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
                on(move |_: On<Activate>| minus(count - 1))
                Children [ @caption("-1") ]
                --
                Text({count.to_string()})
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                --
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(move |_: On<Activate>| plus(count + 1))
                Children [ @caption("+1") ]
            ]
            --
            // Like `{count !== 0 && <Reset />}` in JSX.
            {(count != 0).then(|| bsn_list! {
                @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                on(move |_: On<Activate>| reset(0))
                Children [ @caption("Reset") ]
            })}
        ]
    }
}
