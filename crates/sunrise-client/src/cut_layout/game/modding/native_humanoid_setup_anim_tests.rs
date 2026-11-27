//! The real client-host adapter through CPU, skinned and layer-only setup routes.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{Hooks, HumanoidPart, HumanoidSetupAnimEvent};
use std::sync::{Arc, Mutex};
use sunrise_client::mobs::{render, skin};
use sunrise_sim::mobs::Mob;

struct PoseMod(Arc<Mutex<Vec<(u32, String)>>>);
#[mod_api::__subscriptions]
impl Hooks for PoseMod {
    fn humanoid_setup_anim(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut HumanoidSetupAnimEvent<'_>,
    ) {
        assert_eq!(event.partial_tick, 0.5);
        assert!(!event.humanoid.is_using_item);
        self.0
            .lock()
            .unwrap()
            .push((event.entity_id, event.entity_type.into()));
        let [_, mut rotation] = event.parts.get(HumanoidPart::Body).unwrap();
        rotation[1] = 0.19;
        assert!(event.parts.set_rotation(HumanoidPart::Body, rotation));
    }
}

#[test]
#[ignore = "daily"]
fn client_humanoid_setup_reaches_body_skinned_and_layer_routes() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "pose",
        Box::new(PoseMod(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let lookup = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
            for kind in [
                "minecraft:player",
                "minecraft:zombie",
                "minecraft:giant",
                "minecraft:enderman",
            ] {
                let ty = if kind == "minecraft:player" {
                    game.client_res().player_type().clone().unwrap()
                } else {
                    game.res.mob_types().get(kind).unwrap().clone()
                };
                let mob = Mob::new(73, ty, sunrise_core::geometry::Vec3::ZERO, 0.0);
                let mut models = skin::SkinModels::default();
                let mut frame = sunrise_client::gfx::entity_skin::SkinFrame::default();
                let mut skin = skin::Skin::new(&mut models, &mut frame);
                for route in 0..3 {
                    heard.lock().unwrap().clear();
                    let mut draw = render::FrameDraw::new(true);
                    let mut out = Vec::new();
                    match route {
                        0 => render::render_with_frame(
                            &mob,
                            0.5,
                            glam::DVec3::ZERO,
                            &lookup,
                            0xf0f0,
                            game,
                            &[],
                            &mut draw,
                            &mut out,
                        ),
                        1 => render::render_skinned(
                            &mob,
                            0.5,
                            glam::DVec3::ZERO,
                            &lookup,
                            0xf0f0,
                            game,
                            &mut draw,
                            &mut out,
                            &mut skin,
                        ),
                        _ => render::render_layers(
                            &mob,
                            0.5,
                            glam::DVec3::ZERO,
                            &lookup,
                            0xf0f0,
                            game,
                            &[],
                            &mut draw,
                            &mut out,
                        ),
                    }
                    let events = heard.lock().unwrap();
                    assert!(!events.is_empty(), "{kind} route {route}");
                    assert!(events.iter().all(|(id, ty)| *id == 73 && ty == kind));
                    drop(events);
                    let model = draw.model().expect("the baked body was posed");
                    let body = model.part("body").unwrap();
                    assert_eq!(body.pose().y_rot, 0.19, "{kind} route {route}");
                    if kind == "minecraft:enderman" && route == 0 {
                        let mut baseline = Vec::new();
                        render::render_with_frame(
                            &mob,
                            0.5,
                            glam::DVec3::ZERO,
                            &lookup,
                            0xf0f0,
                            &sunrise_client::mobs_render::ClientMobRender {
                                models: game.client_res().client_mob_models(),
                                data: game.client_res().client_mob_data(),
                            },
                            &[],
                            &mut render::FrameDraw::default(),
                            &mut baseline,
                        );
                        assert_eq!(out.len(), baseline.len());
                        assert!(
                            out.iter().zip(&baseline).any(|(a, b)| a.pos != b.pos),
                            "enderman body edit must change drawn geometry"
                        );
                    }
                }
            }
        });
    });
}
