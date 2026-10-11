//! Counter using event subscriptions.
//!
//! There is no shared state: the label owns its count and subscribes to the
//! `Activate` events of the three buttons. It announces each new count with a
//! `CountChanged` event, which the Reset button subscribes to.

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
use bevy_radioactive::prelude::*;

#[derive(Component, Clone, Default)]
struct Count(i32);

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
                #Minus @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                Children [ @caption("-1") ]
                --
                #Label
                Text("0")
                TextFont { font_size: FontSize::VMin(20.0) }
                ThemeTextColor(tokens::TEXT_MAIN)
                Count(0)
                subscribe_to(#Minus, |click: Subscription<Activate>, q: Labels, commands: Commands| {
                    apply(click.target(), q, commands, |n| n - 1);
                })
                subscribe_to(#Plus, |click: Subscription<Activate>, q: Labels, commands: Commands| {
                    apply(click.target(), q, commands, |n| n + 1);
                })
                subscribe_to(#Reset, |click: Subscription<Activate>, q: Labels, commands: Commands| {
                    apply(click.target(), q, commands, |_| 0);
                })
                --
                #Plus @FeathersButton
                Node { height: Val::Auto, min_width: vmin(18), padding: UiRect::axes(vmin(3), vmin(1.5)) }
                InheritableFont { font_size: FontSize::VMin(6.0) }
                Children [ @caption("+1") ]
            ]
            --
            #Reset @FeathersButton
            Node {
                display: Display::None,
                height: Val::Auto,
                min_width: vmin(18),
                padding: UiRect::axes(vmin(3), vmin(1.5)),
            }
            InheritableFont { font_size: FontSize::VMin(6.0) }
            // The label announces its count; Reset shows itself while it is not zero.
            subscribe_to(#Label, |changed: Subscription<CountChanged>, mut nodes: Query<&mut Node>| {
                if let Ok(mut node) = nodes.get_mut(changed.target()) {
                    node.display = if changed.count != 0 { Display::Flex } else { Display::None };
                }
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

type Labels<'w, 's> = Query<'w, 's, (&'static mut Count, &'static mut Text)>;

fn apply(label: Entity, mut labels: Labels, mut commands: Commands, f: impl Fn(i32) -> i32) {
    if let Ok((mut count, mut text)) = labels.get_mut(label) {
        count.0 = f(count.0);
        **text = count.0.to_string();
        commands.trigger(CountChanged {
            label,
            count: count.0,
        });
    }
}
