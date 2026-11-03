#![cfg(all(feature = "reactive", feature = "ui"))]
//! `rsn!` with `bevy_ui` components, written the way `bsn!` UI code is.

use bevy::{asset::AssetPlugin, prelude::*, scene::ScenePlugin};
use bevy_radioactive::reactive::*;

#[derive(Resource, Clone, Copy, Default)]
struct Score(u32);

#[test]
fn text_constructors_next_to_plain_nodes() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        ScenePlugin,
        ReactivePlugin,
    ))
    .init_resource::<Score>();
    app.world_mut()
        .spawn_scene(rsn! {
            Node
            Children [
                Text::new({ format!( "Score: {}", $Res<Score>.0) })
                Node {
                    padding: px(16),
                    align_self: AlignSelf::FlexStart
                },
                Node {
                    width: px($Res<Score>.0 as f32),
                    padding: UiRect { left: px($Res<Score>.0 as f32 * 2.0), right: px(1) },
                },
                BackgroundColor(Color::srgb($Res<Score>.0 as f32 / 10.0, 0.0, 0.0)),
                TextFont::from_font_size(10.0 + $Res<Score>.0 as f32)
            ]
        })
        .unwrap();
    app.update();
    app.world_mut().resource_mut::<Score>().0 = 7;
    app.update();

    let world = app.world_mut();
    let (text, node) = world.query::<(&Text, &Node)>().single(world).unwrap();
    assert_eq!(text.0, "Score: 7");
    assert_eq!(node.padding.left, px(16));
    assert_eq!(node.align_self, AlignSelf::FlexStart);

    let node = world
        .query_filtered::<&Node, (Without<Text>, With<ChildOf>)>()
        .iter(world)
        .find(|node| node.width != Val::Auto)
        .unwrap();
    assert_eq!(node.width, px(7));
    assert_eq!(node.padding.left, px(14));
    assert_eq!(node.padding.right, px(1));

    let colors: Vec<Color> = world
        .query::<&BackgroundColor>()
        .iter(world)
        .map(|color| color.0)
        .collect();
    assert!(colors.contains(&Color::srgb(0.7, 0.0, 0.0)), "{colors:?}");
    let sizes: Vec<_> = world
        .query::<&TextFont>()
        .iter(world)
        .map(|font| font.font_size)
        .collect();
    assert!(sizes.contains(&TextFont::from_font_size(17.0).font_size));
}
