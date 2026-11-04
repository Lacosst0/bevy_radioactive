//! Game state and rules. Nothing here knows about the UI: systems and
//! observers write components and resources, and the `rsn!` scenes in
//! `ui.rs` show whatever they currently hold.

use bevy::prelude::*;

/// Which screen the game is on. A Bevy state, so `OnEnter` runs the setup.
#[derive(States, Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Menu,
    InGame,
    GameOver,
}

#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub fn next(self) -> Self {
        match self {
            Self::Easy => Self::Normal,
            Self::Normal => Self::Hard,
            Self::Hard => Self::Easy,
        }
    }

    pub fn hit(self) -> u32 {
        match self {
            Self::Easy => 10,
            Self::Normal => 20,
            Self::Hard => 35,
        }
    }
}

/// Whether the game is paused. Not a state, so the HUD stays as it is
/// under the pause menu.
#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Paused(pub bool);

#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct InventoryOpen(pub bool);

/// The inventory slot the player clicked last.
#[derive(Resource, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Selected(pub Option<Entity>);

#[derive(Component, Clone, Copy, Default)]
pub struct Player;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Health {
    pub current: u32,
    pub max: u32,
}

impl Health {
    #[allow(clippy::cast_precision_loss)]
    pub fn fraction(self) -> f32 {
        self.current as f32 / self.max as f32
    }

    pub fn is_low(self) -> bool {
        self.current * 4 <= self.max
    }
}

#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Gold(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Potion,
    Weapon,
    Junk,
}

/// An item in the bag. Each item is an entity, so the inventory is a query.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: &'static str,
    pub kind: ItemKind,
    pub count: u32,
    /// Position in the bag, so slots keep their order.
    pub slot: u32,
}

impl Item {
    pub fn description(&self) -> &'static str {
        match self.kind {
            ItemKind::Potion => "Restores 40 health.",
            ItemKind::Weapon => "Equip it to deal more damage.",
            ItemKind::Junk => "Worth a little gold.",
        }
    }
}

/// On the weapon in hand.
#[derive(Component, Clone, Copy, Default)]
pub struct Equipped;

/// Something hits the player.
#[derive(Event, Clone, Copy)]
pub struct Hit;

/// The player opens a chest.
#[derive(Event, Clone, Copy)]
pub struct Loot;

/// The player uses (drinks, equips, sells) an item from the bag.
#[derive(Event, Clone, Copy)]
pub struct UseItem(pub Entity);

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Screen>()
            .init_resource::<Difficulty>()
            .init_resource::<Paused>()
            .init_resource::<InventoryOpen>()
            .init_resource::<Selected>()
            .add_systems(OnEnter(Screen::InGame), start_run)
            .add_systems(
                Update,
                (
                    keyboard,
                    regenerate.run_if(resource_equals(Paused(false))),
                    die.run_if(in_state(Screen::InGame)),
                ),
            )
            .add_observer(hit)
            .add_observer(loot)
            .add_observer(use_item);
    }
}

/// A fresh player with a starting bag, replacing the last run's.
fn start_run(
    mut commands: Commands,
    old: Query<Entity, Or<(With<Player>, With<Item>)>>,
    mut paused: ResMut<Paused>,
    mut open: ResMut<InventoryOpen>,
    mut selected: ResMut<Selected>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    *paused = Paused(false);
    *open = InventoryOpen(false);
    *selected = Selected(None);
    commands.spawn((
        Player,
        Health {
            current: 100,
            max: 100,
        },
        Gold(10),
    ));
    commands.spawn(Item {
        name: "Potion",
        kind: ItemKind::Potion,
        count: 2,
        slot: 0,
    });
    commands.spawn(Item {
        name: "Rusty sword",
        kind: ItemKind::Weapon,
        count: 1,
        slot: 1,
    });
}

#[allow(clippy::needless_pass_by_value)]
fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    screen: Res<State<Screen>>,
    mut paused: ResMut<Paused>,
    mut open: ResMut<InventoryOpen>,
) {
    if *screen.get() != Screen::InGame {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        paused.0 = !paused.0;
    }
    if keys.just_pressed(KeyCode::KeyI) && !paused.0 {
        open.0 = !open.0;
    }
}

/// One health per second, so the HUD changes on its own.
#[allow(clippy::needless_pass_by_value)]
fn regenerate(
    time: Res<Time>,
    mut timer: Local<f32>,
    mut players: Query<&mut Health, With<Player>>,
) {
    *timer += time.delta_secs();
    if *timer < 1.0 {
        return;
    }
    *timer -= 1.0;
    for mut health in &mut players {
        if health.current > 0 && health.current < health.max {
            health.current += 1;
        }
    }
}

fn die(players: Query<&Health, Changed<Health>>, mut next: ResMut<NextState<Screen>>) {
    if players.iter().any(|health| health.current == 0) {
        next.set(Screen::GameOver);
    }
}

#[allow(clippy::needless_pass_by_value)]
fn hit(_: On<Hit>, difficulty: Res<Difficulty>, mut players: Query<&mut Health, With<Player>>) {
    for mut health in &mut players {
        health.current = health.current.saturating_sub(difficulty.hit());
    }
}

fn loot(
    _: On<Loot>,
    mut commands: Commands,
    mut players: Query<&mut Gold, With<Player>>,
    mut items: Query<&mut Item>,
) {
    let mut found = 0;
    for mut gold in &mut players {
        gold.0 += 5;
        found = gold.0;
    }
    // Something different each time: a potion, then junk, then a sword.
    let (name, kind) = match found / 5 % 3 {
        0 => ("Potion", ItemKind::Potion),
        1 => ("Old boot", ItemKind::Junk),
        _ => ("Short sword", ItemKind::Weapon),
    };
    if kind != ItemKind::Weapon
        && let Some(mut stack) = items.iter_mut().find(|item| item.name == name)
    {
        stack.count += 1;
        return;
    }
    let slot = items.iter().map(|item| item.slot + 1).max().unwrap_or(0);
    commands.spawn(Item {
        name,
        kind,
        count: 1,
        slot,
    });
}

#[allow(clippy::needless_pass_by_value)]
fn use_item(
    event: On<UseItem>,
    mut commands: Commands,
    mut items: Query<(&mut Item, Has<Equipped>)>,
    equipped: Query<Entity, With<Equipped>>,
    mut players: Query<(&mut Health, &mut Gold), With<Player>>,
    mut selected: ResMut<Selected>,
) {
    let Ok((mut item, is_equipped)) = items.get_mut(event.0) else {
        return;
    };
    let Ok((mut health, mut gold)) = players.single_mut() else {
        return;
    };
    match item.kind {
        ItemKind::Potion => {
            health.current = (health.current + 40).min(health.max);
            item.count -= 1;
        }
        ItemKind::Junk => {
            gold.0 += 2;
            item.count -= 1;
        }
        ItemKind::Weapon if is_equipped => {
            commands.entity(event.0).remove::<Equipped>();
        }
        ItemKind::Weapon => {
            for weapon in &equipped {
                commands.entity(weapon).remove::<Equipped>();
            }
            commands.entity(event.0).insert(Equipped);
        }
    }
    if item.count == 0 {
        commands.entity(event.0).despawn();
        selected.0 = None;
    }
}
