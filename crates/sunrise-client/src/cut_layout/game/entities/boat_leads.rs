#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::server_side::boat_leads::Held;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_sim::mobs::leash::{Holder, Pending};
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn command_view_uses_live_and_pending_boat_leashes_not_raw_tags() {
        use sunrise_gameplay_common::game::commands_common::Target;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (id, uuid, actual_holder, unknown) =
                (1, [326, 1, 0, 1], [326, 2, 0, 1], [111, 222, 333, 444]);
            game.boats
                .push(Boat::new(id, "oak".into(), Vec3::new(6.5, 100.0, 4.5), 0.0));
            game.persist.uuids.insert(id, uuid);
            game.level.player_uuid = Some([326, 3, 0, 1]);
            let read = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.command_target_nbt(Target::Boat(id)).unwrap()
            };
            assert!(!game.persist.raw.contains_key(&id));
            assert_eq!(read(game).get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
            assert!(read(game).get("leash").is_none());
            game.boats
                .update(0, |boat| boat.leash = Some(Holder::Player));
            game.boats
                .update(0, |boat| boat.leash_player = Some(actual_holder));
            let expected = Tag::compound([("UUID", Tag::IntArray(actual_holder.to_vec()))]);
            assert_eq!(read(game).get("leash"), Some(&expected));
            assert_eq!(read(game).get("leash"), Some(&expected));
            let delayed = Tag::compound([("UUID", Tag::IntArray(unknown.to_vec()))]);
            game.persist
                .raw
                .insert(id, Tag::compound([("leash", delayed.clone())]));
            assert_eq!(read(game).get("leash"), Some(&expected));
            game.boats.update(0, |boat| boat.leash = None);
            game.boats.update(0, |boat| boat.leash_player = None);
            game.boats
                .update(0, |boat| boat.leash_pending = Some(Pending::Uuid(unknown)));
            assert_eq!(read(game).get("leash"), Some(&delayed));
            game.boats.update(0, |boat| boat.leash_pending = None);
            assert!(read(game).get("leash").is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn non_guest_holder_branches_keep_their_existing_velocity_sources() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        assert!(game.run_command("summon oak_boat 6.5 100 4.5 {UUID:[I;327,1,0,1]}"));
        let (id, expected) = game.boats.update(0, |boat| {
            boat.velocity = Vec3::new(0.25, -0.125, 0.5);
            (boat.id, boat.velocity)
        });
        let Held::At(state, quad) = game.boat_holder(Holder::Boat(id), None) else {
            panic!("actual boat holder must resolve");
        };
        assert_eq!(state.velocity, expected);
        assert!(!quad);
        assert!(game.run_command("summon pig 4.5 100 4.5 {UUID:[I;327,2,0,1],NoAI:1b}"));
        let mob = &mut game.mobs[0];
        mob.velocity = Vec3::new(-0.5, 0.25, 0.125);
        let (id, expected) = (mob.id, mob.velocity);
        let Held::At(state, quad) = game.boat_holder(Holder::Mob(id), None) else {
            panic!("actual mob holder must resolve");
        };
        assert_eq!(state.velocity, expected);
        assert!(!quad);
        assert!(matches!(
            game.boat_holder(Holder::Player, Some([327, 3, 0, 1])),
            Held::Gone
        ));
        assert!(matches!(
            game.boat_holder(Holder::Boat(u32::MAX), None),
            Held::Gone
        ));
        assert!(matches!(
            game.boat_holder(Holder::Knot([0, 100, 0]), None),
            Held::Gone
        ));
    }
}
