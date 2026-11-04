//! The whole UI, written with `rsn!` only: no marker components, no sync
//! systems. Each scene says what it shows; `$` marks what it reads.

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui_widgets::{Activate, Button},
};
use bevy_radioactive::reactive::*;

use crate::game::*;

const TEXT: Color = Color::srgb(0.92, 0.92, 0.88);
const MUTED: Color = Color::srgb(0.6, 0.6, 0.58);
const PANEL: Color = Color::srgba(0.1, 0.1, 0.12, 0.92);
const BUTTON: Color = Color::srgb(0.22, 0.22, 0.26);
const BUTTON_HOVER: Color = Color::srgb(0.32, 0.32, 0.38);
const GOLD: Color = Color::srgb(0.95, 0.78, 0.25);
const HEALTHY: Color = Color::srgb(0.3, 0.75, 0.35);
const HURT: Color = Color::srgb(0.85, 0.25, 0.2);

/// The root: one screen at a time.
pub fn root() -> impl Scene {
    rsn! {
        Node {
            width: percent(100),
            height: percent(100),
        }
        Children [
            match *$Res<State<Screen>>.get() {
                Screen::Menu => @main_menu(),
                Screen::InGame => @in_game(),
                Screen::GameOver => @game_over(),
            }
        ]
    }
}

/// A button that lights up under the pointer. Add `on(|_: On<Activate>| ..)`
/// where it is used: `@button("Play") on(..)`.
pub fn button(label: &'static str) -> impl Scene {
    rsn! {
        #Button
        Button
        Hovered
        Node {
            padding: UiRect::axes(px(18), px(8)),
            justify_content: JustifyContent::Center,
            min_width: px(140),
            border_radius: BorderRadius::all(px(6)),
        }
        BackgroundColor({ if $Ref<Hovered>(#Button).0 { BUTTON_HOVER } else { BUTTON } })
        Children [ Text({label}) TextColor(TEXT) ]
    }
}

fn centered_column() -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(14),
        }
    }
}

fn main_menu() -> impl Scene {
    rsn! {
        @centered_column()
        Children [
            Text("Radioactive Quest")
            TextFont { font_size: FontSize::Px(44.0) }
            TextColor(GOLD)
            --
            @button("Play")
            on(|_: On<Activate>| $ResMut<NextState<Screen>>.set(Screen::InGame))
            --
            Node { column_gap: px(12), align_items: AlignItems::Center }
            Children [
                @button("Difficulty")
                on(|_: On<Activate>| {
                    let next = $Res<Difficulty>.next();
                    *$ResMut<Difficulty> = next;
                })
                --
                Text({ format!("{:?}", *$Res<Difficulty>) })
                TextColor(MUTED)
            ]
        ]
    }
}

fn game_over() -> impl Scene {
    rsn! {
        @centered_column()
        BackgroundColor(Color::srgb(0.2, 0.04, 0.04))
        Children [
            Text("You died")
            TextFont { font_size: FontSize::Px(44.0) }
            TextColor(HURT)
            --
            Text({ format!("You had {} gold", $Single<&Gold, With<Player>>.0) })
            TextColor(TEXT)
            --
            @button("Try again")
            on(|_: On<Activate>| $ResMut<NextState<Screen>>.set(Screen::InGame))
            --
            @button("Main menu")
            on(|_: On<Activate>| $ResMut<NextState<Screen>>.set(Screen::Menu))
        ]
    }
}

fn in_game() -> impl Scene {
    rsn! {
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SpaceBetween,
            padding: UiRect::all(px(16)),
        }
        Children [
            @hud()
            --
            if $Res<InventoryOpen>.0 { @inventory() }
            --
            @actions()
            --
            // Over everything else; the HUD stays as it is underneath.
            if $Res<Paused>.0 { @pause_menu() }
        ]
    }
}

fn hud() -> impl Scene {
    rsn! {
        Node { column_gap: px(20), align_items: AlignItems::Center }
        Children [
            @health_bar()
            --
            Text({ format!("Gold {}", $Single<&Gold, With<Player>>.0) })
            TextColor(GOLD)
            --
            Text({
                match $Query<&Item, With<Equipped>>.iter().next() {
                    Some(weapon) => format!("Wielding {}", weapon.name),
                    None => "Bare hands".to_string(),
                }
            })
            TextColor(MUTED)
            --
            if $Single<&Health, With<Player>>.is_low() {
                Text("Low health! Drink a potion.")
                TextColor(HURT)
            }
        ]
    }
}

fn health_bar() -> impl Scene {
    rsn! {
        Node {
            width: px(220),
            height: px(22),
            border: UiRect::all(px(2)),
        }
        BorderColor::all(MUTED)
        BackgroundColor(PANEL)
        Children [
            Node {
                width: percent($Single<&Health, With<Player>>.fraction() * 100.0),
                height: percent(100),
            }
            BackgroundColor({ if $Single<&Health, With<Player>>.is_low() { HURT } else { HEALTHY } })
            --
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
            }
            Children [
                Text({
                    let health = $Single<&Health, With<Player>>;
                    format!("{} / {}", health.current, health.max)
                })
                TextFont { font_size: FontSize::Px(14.0) }
                TextColor(TEXT)
            ]
        ]
    }
}

/// Buttons that stand in for gameplay, so there is something to show.
fn actions() -> impl Scene {
    rsn! {
        Node { column_gap: px(12) }
        Children [
            @button("Take a hit")
            on(|_: On<Activate>| $Commands.trigger(Hit))
            --
            @button("Open chest")
            on(|_: On<Activate>| $Commands.trigger(Loot))
            --
            @button("Bag [I]")
            on(|_: On<Activate>| $ResMut<InventoryOpen>.0 ^= true)
            --
            @button("Pause [Esc]")
            on(|_: On<Activate>| $ResMut<Paused>.0 = true)
        ]
    }
}

fn inventory() -> impl Scene {
    rsn! {
        Node {
            flex_direction: FlexDirection::Column,
            align_self: AlignSelf::Center,
            row_gap: px(12),
            padding: UiRect::all(px(16)),
            width: px(480),
            border_radius: BorderRadius::all(px(8)),
        }
        BackgroundColor(PANEL)
        Children [
            Text({ format!("Bag ({} items)", $Query<&Item>.iter().map(|item| item.count).sum::<u32>()) })
            TextColor(TEXT)
            --
            if $Query<(), With<Item>>.is_empty() {
                Text("Your bag is empty. Open a chest!")
                TextColor(MUTED)
            }
            --
            Node { flex_wrap: FlexWrap::Wrap, column_gap: px(8), row_gap: px(8) }
            Children [
                // In bag order: keyed by entity, sorted by slot.
                #[key(*item)]
                for item in {
                    let mut items: Vec<(u32, Entity)> = $Query<(Entity, &Item)>
                        .iter()
                        .map(|(entity, item)| (item.slot, entity))
                        .collect();
                    items.sort();
                    items.into_iter().map(|(_, entity)| entity).collect::<Vec<_>>()
                } {
                    @slot(item)
                }
            ]
            --
            match $Res<Selected>.0 {
                Some(item) => @details(item),
                None => Text("Click an item to inspect it.") TextColor(MUTED),
            }
        ]
    }
}

fn slot(item: Entity) -> impl Scene {
    rsn! {
        #Slot
        Button
        Hovered
        Node {
            width: px(88),
            height: px(88),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(6)),
        }
        BorderColor::all({ if $Res<Selected>.0 == Some(item) { GOLD } else { Color::NONE } })
        BackgroundColor({ if $Ref<Hovered>(#Slot).0 { BUTTON_HOVER } else { BUTTON } })
        on(|_: On<Activate>| $ResMut<Selected>.0 = Some(item))
        Children [
            Text({ $Ref<Item>(item).name })
            TextFont { font_size: FontSize::Px(13.0) }
            TextLayout { justify: Justify::Center }
            TextColor(TEXT)
            --
            if $Ref<Item>(item).count > 1 {
                Text({ format!("x{}", $Ref<Item>(item).count) })
                TextFont { font_size: FontSize::Px(12.0) }
                TextColor(MUTED)
            }
            --
            if $Query<(), With<Equipped>>.contains(item) {
                Text("equipped")
                TextFont { font_size: FontSize::Px(11.0) }
                TextColor(GOLD)
            }
        ]
    }
}

fn details(item: Entity) -> impl Scene {
    rsn! {
        Node { column_gap: px(12), align_items: AlignItems::Center }
        Children [
            Node { flex_direction: FlexDirection::Column, flex_grow: 1.0 }
            Children [
                Text({ $Ref<Item>(item).name }) TextColor(TEXT)
                --
                Text({ $Ref<Item>(item).description() }) TextColor(MUTED)
            ]
            --
            match $Ref<Item>(item).kind {
                ItemKind::Potion => @button("Drink") on(|_: On<Activate>| $Commands.trigger(UseItem(item))),
                ItemKind::Junk => @button("Sell") on(|_: On<Activate>| $Commands.trigger(UseItem(item))),
                ItemKind::Weapon => {
                    if $Query<(), With<Equipped>>.contains(item) {
                        @button("Unequip") on(|_: On<Activate>| $Commands.trigger(UseItem(item)))
                    } else {
                        @button("Equip") on(|_: On<Activate>| $Commands.trigger(UseItem(item)))
                    }
                }
            }
        ]
    }
}

fn pause_menu() -> impl Scene {
    rsn! {
        @centered_column()
        // Covers the screen, padding included.
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            width: Val::Auto,
            height: Val::Auto,
        }
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6))
        Children [
            Text("Paused")
            TextFont { font_size: FontSize::Px(36.0) }
            TextColor(TEXT)
            --
            @button("Resume")
            on(|_: On<Activate>| $ResMut<Paused>.0 = false)
            --
            @button("Quit to menu")
            on(|_: On<Activate>| $ResMut<NextState<Screen>>.set(Screen::Menu))
        ]
    }
}
