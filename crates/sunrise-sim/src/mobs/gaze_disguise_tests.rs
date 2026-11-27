use super::*;
use crate::mobs::world::{test_player, with_resource_test_world};

fn gaze_cases(replace_tag: bool) {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources");
        assert!(with_resource_test_world(
            std::path::Path::new(&root),
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let mut items = w.items.unwrap().clone();
                if replace_tag {
                    items.item_tags.insert(
                        "minecraft:gaze_disguise_equipment".into(),
                        ["minecraft:skeleton_skull".into()].into(),
                    );
                }
                for (head, disguised) in if replace_tag {
                    [
                        (Some("minecraft:skeleton_skull"), true),
                        (Some("minecraft:carved_pumpkin"), false),
                        (None, false),
                    ]
                } else {
                    [
                        (Some("minecraft:carved_pumpkin"), true),
                        (Some("minecraft:skeleton_skull"), false),
                        (None, false),
                    ]
                } {
                    for id in ["minecraft:enderman", "minecraft:creaking"] {
                        let mut m = Mob::new(
                            1,
                            std::sync::Arc::new(types.types[id].clone()),
                            Vec3::new(0.5, 64.0, 0.5),
                            0.0,
                        );
                        let mut p = test_player(Vec3::new(2.5, 64.0, 0.5));
                        p.head = head.map(Into::into);
                        let dy = m.eye_y() - p.eye[1];
                        let len = (4.0 + dy * dy).sqrt();
                        p.look = [-2.0 / len, dy / len, 0.0];
                        let mut world = w.clone();
                        world.items = Some(&items);
                        world.player = Some(p);
                        if id == "minecraft:enderman" {
                            assert_eq!(
                                crate::mobs::enderman::stared_at(&m, &world),
                                !disguised,
                                "{head:?}"
                            );
                        } else {
                            monster_brain::init(&mut m, 0);
                            monster_brain::mem(&mut m)
                                .nearest_players
                                .set_list(vec![Who::Player]);
                            m.extra.flag = true;
                            assert_eq!(
                                check_can_move(&mut m, &world, &mut Vec::new()),
                                disguised,
                                "{head:?}"
                            );
                        }
                    }
                }
            }
        ));
    });
}

#[test]
#[ignore = "daily"]
fn gaze_disguise_shipped_tag() {
    gaze_cases(false);
}

#[test]
#[ignore = "daily"]
fn gaze_disguise_replaced_tag() {
    gaze_cases(true);
}

/// A mod's `gaze_disguise` answer: it flips vanilla's, and logs the mob that asked.
struct Flip(std::sync::Mutex<Vec<u32>>);
impl crate::mobs::MobHooks for Flip {
    fn gaze_disguise(&self, observer: u32, _: &crate::mobs::PlayerView, disguised: bool) -> bool {
        self.0.lock().unwrap().push(observer);
        !disguised
    }
    fn enderman_anger_suppressed(&self, _: u32, _: &crate::mobs::PlayerView) -> bool {
        false
    }
    fn can_grief(&self, _: u32, rule: bool) -> bool {
        rule
    }
    fn grief_hooked(&self) -> bool {
        false
    }
    fn living_visibility(&self, _: &crate::mobs::PlayerView, _: u32, visibility: f64) -> f64 {
        visibility
    }
    fn living_change_target(
        &self,
        _: &mut Mob,
        _: &MobWorld<'_>,
        _: bool,
        _: Option<Who>,
    ) -> crate::mobs::TargetChange {
        crate::mobs::TargetChange::Keep
    }
}

/// `gaze_disguise` (#5948): the enderman's stare and an awake creaking's look both take the
/// mod's answer over vanilla's tag test, with the observing mob's id; a dormant creaking
/// doesn't ask.
#[test]
#[ignore = "daily"]
fn a_mods_gaze_disguise_answer_rules_the_enderman_and_the_creaking() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources");
        assert!(with_resource_test_world(
            std::path::Path::new(&root),
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let hook = Flip(Default::default());
                for (head, vanilla) in [(Some("minecraft:carved_pumpkin"), true), (None, false)] {
                    for (id, observer) in [("minecraft:enderman", 7), ("minecraft:creaking", 8)] {
                        let mut m = Mob::new(
                            observer,
                            std::sync::Arc::new(types.types[id].clone()),
                            Vec3::new(0.5, 64.0, 0.5),
                            0.0,
                        );
                        let mut p = test_player(Vec3::new(2.5, 64.0, 0.5));
                        p.head = head.map(Into::into);
                        let dy = m.eye_y() - p.eye[1];
                        let len = (4.0 + dy * dy).sqrt();
                        p.look = [-2.0 / len, dy / len, 0.0];
                        let mut world = w.clone();
                        world.player = Some(p);
                        world.mob_hooks = Some(&hook);
                        hook.0.lock().unwrap().clear();
                        if id == "minecraft:enderman" {
                            assert_eq!(
                                crate::mobs::enderman::stared_at(&m, &world),
                                vanilla,
                                "{head:?}"
                            );
                        } else {
                            monster_brain::init(&mut m, 0);
                            monster_brain::mem(&mut m)
                                .nearest_players
                                .set_list(vec![Who::Player]);
                            m.extra.flag = true;
                            assert_eq!(
                                check_can_move(&mut m, &world, &mut Vec::new()),
                                !vanilla,
                                "{head:?}"
                            );
                        }
                        assert_eq!(*hook.0.lock().unwrap(), [observer], "{id} asks once");
                    }
                }
                // A dormant creaking wakes whatever the player wears: it doesn't ask.
                let mut m = Mob::new(
                    9,
                    std::sync::Arc::new(types.types["minecraft:creaking"].clone()),
                    Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                monster_brain::init(&mut m, 0);
                monster_brain::mem(&mut m)
                    .nearest_players
                    .set_list(vec![Who::Player]);
                let mut world = w.clone();
                world.player = Some(test_player(Vec3::new(30.5, 64.0, 0.5)));
                world.mob_hooks = Some(&hook);
                hook.0.lock().unwrap().clear();
                check_can_move(&mut m, &world, &mut Vec::new());
                assert!(
                    hook.0.lock().unwrap().is_empty(),
                    "a dormant creaking doesn't ask"
                );
            }
        ));
    });
}
