//! The living-entity render adapter uses a cat's registry variant to choose its texture.
use super::{HiddenLayersData, StockLayerMask};
use crate::game::native_entity_refs::tests::with_client_game;
use sunrise_client::mobs::render;
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Mob;

#[test]
#[ignore = "daily"]
fn the_world_draw_paints_a_cat_with_its_variant_coat() {
    with_client_game(|game| {
        let kind = game.res.mob_types()["minecraft:cat"].clone();
        let mut cat = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        cat.extra.cat_variant = game.world_variants().cat.lookup("minecraft:ragdoll");
        assert!(
            cat.extra.cat_variant.is_some(),
            "vanilla has the ragdoll coat"
        );
        let data = HiddenLayersData {
            game,
            hidden_layers: StockLayerMask::EMPTY,
        };
        assert_eq!(
            render::drawn_textures(&cat, &data)
                .first()
                .map(String::as_str),
            Some("entity/cat/cat_ragdoll")
        );
    });
}
