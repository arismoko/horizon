//! `living_model_posed` through the real client-host adapter and the living draw.
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{Hooks, LivingModelPosedEvent, PartPose};
use std::sync::{Arc, Mutex};
use sunrise_client::mobs::render;
use sunrise_client_gameplay::game::ClientGame;
use sunrise_core::geometry::Vec3;
use sunrise_items::{equipment::CHEST, items::ItemStack};
use sunrise_sim::mobs::{Mob, anim::Poses};

#[derive(Clone, Copy)]
enum Action {
    Observe,
    Write(f32),
    WriteThenReset,
    WriteNonFinite,
}

/// What one call saw of the right arm, and what its edit did.
struct Call {
    layer: String,
    before: PartPose,
    after: PartPose,
    accepted: Option<bool>,
}

struct Posed(Arc<Mutex<Vec<Call>>>, Action);
#[mod_api::__subscriptions]
impl Hooks for Posed {
    fn living_model_posed(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut LivingModelPosedEvent<'_>,
    ) {
        let Some(index) = event.model.part("right_arm").map(|part| part.index) else {
            self.0.lock().unwrap().push(Call {
                layer: event.layer.into(),
                before: PartPose::ZERO,
                after: PartPose::ZERO,
                accepted: None,
            });
            return;
        };
        let before = event.model.part_at(index).unwrap().pose();
        let mut written = before;
        let accepted = match self.1 {
            Action::Observe => None,
            Action::Write(x_rot) => {
                written.x_rot = x_rot;
                Some(event.model.set_pose(index, written))
            }
            Action::WriteThenReset => {
                written.x_rot = 1.25;
                assert!(event.model.set_pose(index, written));
                Some(event.model.reset_pose(index))
            }
            Action::WriteNonFinite => {
                written.x_rot = f32::NAN;
                Some(event.model.set_pose(index, written))
            }
        };
        let after = event.model.part_at(index).unwrap().pose();
        self.0.lock().unwrap().push(Call {
            layer: event.layer.into(),
            before,
            after,
            accepted,
        });
    }
}

/// Draws `mob` once and returns what the frame model holds for its right arm.
fn right_arm_drawn(game: &ClientGame<'_>, mob: &Mob) -> PartPose {
    let lookup = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
    let mut draw = render::FrameDraw::new(true);
    render::render_with_frame(
        mob,
        0.5,
        glam::DVec3::ZERO,
        &lookup,
        0xf0f0,
        game,
        &[],
        &mut draw,
        &mut Vec::new(),
    );
    draw.model()
        .expect("the baked body was posed")
        .part("right_arm")
        .unwrap()
        .pose()
}

fn with_posed<R>(action: Action, test: impl FnOnce(&Arc<Mutex<Vec<Call>>>) -> R) -> R {
    crate::entry::install_client_hooks();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "posed",
        Box::new(Posed(calls.clone(), action)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(&calls))
}

fn player(game: &ClientGame<'_>) -> Mob {
    let mut mob = Mob::new(
        73,
        game.client_res().player_type().clone().unwrap(),
        Vec3::ZERO,
        0.0,
    );
    // Halfway through an attack swing of the main hand.
    mob.swing_hand(false, 6);
    mob.anim.attack_anim_o = 0.4;
    mob.anim.attack_anim = 0.6;
    mob
}

#[test]
#[ignore = "daily"]
fn an_edit_after_an_attack_swing_sees_the_swing_and_is_drawn() {
    with_client_game(|game| {
        let mob = player(game);
        let swung = right_arm_drawn(game, &mob);
        let mut idle = Mob::new(74, mob.kind.clone(), Vec3::ZERO, 0.0);
        idle.anim.attack_anim = 0.0;
        assert_ne!(right_arm_drawn(game, &idle).x_rot, swung.x_rot);
        with_posed(Action::Write(1.25), |calls| {
            let drawn = right_arm_drawn(game, &mob);
            let calls = calls.lock().unwrap();
            assert_eq!(calls[0].layer, "minecraft:player#main");
            // The swing was already applied when the listener read the arm.
            assert_eq!(calls[0].before, swung);
            assert_eq!(calls[0].accepted, Some(true));
            assert_eq!(drawn.x_rot, 1.25);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_worn_chestplate_model_gets_a_call_of_its_own() {
    with_client_game(|game| {
        let mut mob = Mob::new(
            75,
            game.res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone(),
            Vec3::ZERO,
            0.0,
        );
        mob.extra
            .set_slot_stack(CHEST, Some(ItemStack::new("minecraft:iron_chestplate", 1)));
        with_posed(Action::Observe, |calls| {
            right_arm_drawn(game, &mob);
            let layers: Vec<_> = calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.layer.clone())
                .collect();
            assert_eq!(layers[0], "minecraft:zombie#main");
            assert!(
                layers[1..].contains(&"minecraft:zombie#chestplate".to_string()),
                "{layers:?}"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn reset_pose_returns_the_bind_pose() {
    with_client_game(|game| {
        let mob = player(game);
        let bind = {
            let model = game
                .client_res()
                .client_mob_models()
                .model(&mob.kind.id)
                .unwrap();
            let mut poses = Poses::new(model.as_ref());
            *poses.get("right_arm")
        };
        with_posed(Action::WriteThenReset, |calls| {
            let drawn = right_arm_drawn(game, &mob);
            let calls = calls.lock().unwrap();
            assert_eq!(calls[0].accepted, Some(true));
            let after = [
                calls[0].after.x,
                calls[0].after.y,
                calls[0].after.z,
                calls[0].after.x_rot,
                calls[0].after.y_rot,
                calls[0].after.z_rot,
                calls[0].after.x_scale,
                calls[0].after.y_scale,
                calls[0].after.z_scale,
            ];
            assert_eq!(after, bind);
            assert_eq!(drawn.x_rot, bind[3]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_non_finite_write_is_refused_and_the_part_keeps_its_pose() {
    with_client_game(|game| {
        let mob = player(game);
        let swung = right_arm_drawn(game, &mob);
        with_posed(Action::WriteNonFinite, |calls| {
            let drawn = right_arm_drawn(game, &mob);
            let calls = calls.lock().unwrap();
            assert_eq!(calls[0].accepted, Some(false));
            assert_eq!(calls[0].after, calls[0].before);
            assert_eq!(drawn, swung);
        });
    });
}

/// A mod that listens to another hook leaves the draw without a posed observer: no event is
/// built and nothing is called.
#[test]
#[ignore = "daily"]
fn no_listener_means_no_call() {
    struct Other;
    #[mod_api::__subscriptions]
    impl Hooks for Other {
        fn humanoid_setup_anim(
            &self,
            _: &dyn mod_api::ClientHost,
            _: &mut mod_api::HumanoidSetupAnimEvent<'_>,
        ) {
        }
    }
    with_client_game(|game| {
        assert!(render::RenderDataSource::model_posed(&*game).is_none());
        let mods = vec![mod_api::ModEntry::new("other", Box::new(Other))].into();
        mod_platform::testing::with_mods(mods, || {
            assert!(render::RenderDataSource::model_posed(&*game).is_none());
        });
    });
}
