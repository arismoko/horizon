#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_sim::mobs::Species;
#[cfg(test)]
mod mod_parrot_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_gameplay::game::shoulders::shoulder_variants;
    use sunrise_sim::mobs::Mob;
    use sunrise_sim::motion::PlayerMotion;

    /// A parrot lands on its owner's shoulder by its class, saved there as
    /// itself, so a content mod's parrot type (#4031) does too, under its own
    /// id: it comes back off through the same load as any saved mob, which
    /// `a_mod_tameable_type_saves_and_loads_its_owner` covers for mod types
    /// (this game's resources have none). Only the vanilla parrot gets a
    /// shoulder variant, as `ServerPlayer.extractParrotVariant` reads only that
    /// id: a mod draws its own parrot there.
    #[test]
    #[ignore = "daily"]
    fn a_mod_parrot_type_rides_a_shoulder_as_itself() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let owner = [1, 2, 3, 4];
            game.level.player_uuid = Some(owner);
            let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap();
            player.ground.on_ground = true;
            game.player = Some(player);
            let parrot = game
                .res
                .mob_types()
                .get("minecraft:parrot")
                .cloned()
                .unwrap();
            let mut macaw = (*parrot).clone();
            macaw.id = "ruby:macaw".into();
            for (n, (kind, variant)) in [(parrot, Some(0)), (Arc::new(macaw), None)]
                .into_iter()
                .enumerate()
            {
                let id = kind.id.clone();
                let mob = 9 + n as u32;
                let mut bird = Mob::new(mob, kind, Vec3::new(0.5, 100.0, 0.5), 0.0);
                bird.extra.tame = true;
                bird.extra.owner = Some(owner);
                game.mobs.push(bird);
                game.parrot_to_shoulder(mob);
                let extra = game.level.player_extra.as_ref();
                let left = extra.and_then(|e| e.get("ShoulderEntityLeft"));
                assert_eq!(
                    left.and_then(|t| t.get("id")).cloned(),
                    Some(Tag::String(id.as_str().into())),
                    "{id} is saved on the shoulder as itself"
                );
                assert_eq!(shoulder_variants(extra), [variant, None], "{id}");
                game.world.game_time += 40;
                game.remove_entities_on_shoulder();
                if variant.is_some() {
                    let back = game
                        .mobs
                        .iter()
                        .find(|m| !m.removed && m.kind.id == id)
                        .expect("it comes back off the shoulder");
                    assert_eq!(
                        (back.species(), back.extra.owner),
                        (Species::Parrot, Some(owner)),
                        "{id}"
                    );
                }
                game.level.player_extra = None;
                game.mobs.clear();
            }
        });
    }
}
