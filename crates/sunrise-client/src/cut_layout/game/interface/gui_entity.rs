#![cfg(test)]
use sunrise_client::hud::HudBatch;
use sunrise_core::nbt::Tag;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use mod_api::{GuiEntity, GuiEntityError, GuiEntityLook, GuiEntityView};
    use sunrise_core::nbt::Compound;

    #[test]
    #[ignore = "daily"]
    fn a_screen_draws_a_mob_by_type_once_and_refuses_what_it_cant_draw() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let look = GuiEntityLook::At([50.0, 40.0]);
            let view = GuiEntityView::new([26.0, 8.0, 75.0, 78.0], 30.0, 0.0625, look);
            let mut batch = HudBatch::scratch([320, 240]);
            let zombie = GuiEntity::kind("minecraft:zombie");
            assert_eq!(game.native_draw_entity(&mut batch, &zombie, view), Ok(()));
            let once = batch.vertices.len();
            assert!(once > 0, "the zombie's model is drawn");
            let mut data = Compound::new();
            data.insert("IsBaby".into(), Tag::Byte(1));
            let baby = GuiEntity::with_data("minecraft:zombie", data.clone());
            assert_eq!(game.native_draw_entity(&mut batch, &baby, view), Ok(()));
            assert!(batch.vertices.len() > once);
            // Drawn again every frame, each is made once and kept.
            for _ in 0..3 {
                game.native_draw_entity(&mut batch, &zombie, view).unwrap();
                game.native_draw_entity(&mut batch, &baby, view).unwrap();
            }
            assert_eq!(
                sunrise_client_gameplay::game::client_side::gui_entity::test_cached_entity_count(),
                2
            );
            let spin = GuiEntityLook::Turned {
                yaw: 90.0,
                pitch: 0.0,
            };
            let turned = GuiEntityView::new(view.rect, 30.0, 0.0, spin);
            assert_eq!(game.native_draw_entity(&mut batch, &zombie, turned), Ok(()));
            let before = batch.vertices.len();
            let refused = |entity: &GuiEntity, view| {
                let mut batch = HudBatch::scratch([320, 240]);
                let answer = game.native_draw_entity(&mut batch, entity, view);
                assert!(batch.vertices.is_empty(), "{answer:?} drew nothing");
                answer
            };
            for kind in ["minecraft:no_such_mob", "minecraft:arrow"] {
                let answer = refused(&GuiEntity::kind(kind), view);
                assert_eq!(answer, Err(GuiEntityError::UnknownType), "{kind}");
            }
            data.insert("id".into(), Tag::String("minecraft:skeleton".into()));
            let other = GuiEntity::with_data("minecraft:zombie", data);
            assert_eq!(refused(&other, view), Err(GuiEntityError::BadData));
            let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(999), 5);
            assert_eq!(
                refused(&GuiEntity::existing(stale), view),
                Err(GuiEntityError::NoEntity)
            );
            let empty = GuiEntityView::new([26.0, 8.0, 26.0, 78.0], 30.0, 0.0, look);
            assert_eq!(refused(&zombie, empty), Err(GuiEntityError::BadView));
            // Finite values whose sums or products overflow are refused too, drawing nothing.
            let huge = [
                GuiEntityView::new([f32::MAX / 2.0, 0.0, f32::MAX, 100.0], 30.0, 0.0, look),
                GuiEntityView::new([0.0, f32::MAX / 2.0, 100.0, f32::MAX], 30.0, 0.0, look),
                GuiEntityView::new([0.0, 0.0, 100.0, 100.0], f32::MAX, 0.0, look),
                GuiEntityView::new([0.0, 0.0, 100.0, 100.0], 30.0, f32::MAX, look),
            ];
            for view in huge {
                assert!(view.is_drawable(), "{:?}", view.rect);
                assert_eq!(
                    refused(&zombie, view),
                    Err(GuiEntityError::BadView),
                    "{:?}",
                    view.rect
                );
            }
            assert_eq!(batch.vertices.len(), before);
        });
    }
}
