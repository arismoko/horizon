#[cfg(test)]
pub(in crate::game) mod rope_hold_tests {
    use crate::game::client_side::leads_client::rope_hold_angles;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::leash_knot::{
        held_by_acting_player, resolve_pending_leash, write_mob_leash_tag,
    };
    use sunrise_protocol::crypto::offline_uuid;
    use sunrise_sim::lan::playerdata::uuid_ints;
    use sunrise_sim::mobs::leash::{self, Holder, Pending};

    #[test]
    fn rope_hold_angles_lerp_by_half_the_partial_tick() {
        // Mth.lerp(pt * 0.5, yRot, yRotO): at pt 1 the view is halfway to the old one.
        let (y, x) = rope_hold_angles((90.0, 20.0), (50.0, 0.0), 1.0);
        assert!((y - 70.0_f64.to_radians()).abs() < 1e-6);
        assert!((x - 10.0_f64.to_radians()).abs() < 1e-6);
        let (y, _) = rope_hold_angles((90.0, 20.0), (50.0, 0.0), 0.0);
        assert!((y - 90.0_f64.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn player_leash_survives_save_and_reopen_for_owner_and_guest() {
        // Integrated servers initialize `level.player_uuid` to `offline_uuid("Player")`,
        // while the single-player owner joins `hub.guests` under their actual profile UUID.
        let level_player_uuid = uuid_ints(offline_uuid("Player"));
        let owner_uuid = uuid_ints(offline_uuid("Player46606"));
        let guest_uuid = uuid_ints(offline_uuid("Alex"));

        for acting in [owner_uuid, guest_uuid] {
            let mut tag = Tag::Compound(Default::default());
            write_mob_leash_tag(
                &mut tag,
                Some(Holder::Player),
                None,
                Some(acting),
                Some(level_player_uuid),
                |_| [0; 4],
            );
            let pending = leash::pending_from_nbt(tag.get("leash")).expect("saved leash tag");
            assert_eq!(pending, Pending::Uuid(acting));

            let (holder, restored_player) = resolve_pending_leash(
                pending,
                level_player_uuid,
                Some(owner_uuid),
                |u| u == owner_uuid || u == guest_uuid,
                |_| None,
                |_| None,
                |_| None,
            )
            .expect("restored player leash");
            assert_eq!(holder, Holder::Player);
            assert_eq!(restored_player, Some(acting));
            assert!(held_by_acting_player(Some(holder), restored_player, acting));
            let other = if acting == owner_uuid {
                guest_uuid
            } else {
                owner_uuid
            };
            assert!(!held_by_acting_player(Some(holder), restored_player, other));
        }
    }

    #[test]
    fn legacy_singleplayer_uuid_leash_restores_to_integrated_owner() {
        let level_player_uuid = uuid_ints(offline_uuid("Player"));
        let owner_uuid = uuid_ints(offline_uuid("Player46606"));

        let (holder, restored_player) = resolve_pending_leash(
            Pending::Uuid(level_player_uuid),
            level_player_uuid,
            Some(owner_uuid),
            |_| false,
            |_| None,
            |_| None,
            |_| None,
        )
        .expect("restored legacy singleplayer leash");
        assert_eq!(holder, Holder::Player);
        assert_eq!(restored_player, Some(owner_uuid));
        assert!(held_by_acting_player(
            Some(holder),
            restored_player,
            owner_uuid
        ));
    }

    #[test]
    fn an_absent_players_uuid_is_not_reassigned_to_the_owner() {
        let owner = uuid_ints(offline_uuid("LeashOwner"));
        let absent = uuid_ints(offline_uuid("Alex"));
        // Even an imported world's player UUID is not evidence of ownership.
        for level in [uuid_ints(offline_uuid("Player")), absent] {
            assert!(
                resolve_pending_leash(
                    Pending::Uuid(absent),
                    level,
                    Some(owner),
                    |_| false,
                    |_| None,
                    |_| None,
                    |_| None,
                )
                .is_none()
            );
        }
        // An entity with that UUID still resolves as an entity, not the owner.
        assert_eq!(
            resolve_pending_leash(
                Pending::Uuid(absent),
                absent,
                Some(owner),
                |_| false,
                |u| (u == absent).then_some(17),
                |_| None,
                |_| None,
            ),
            Some((Holder::Mob(17), None))
        );
    }

    /// #13113: a leash knot's UUID resolves to the knot's fence post, as
    /// `ServerLevel.getEntity` finds the knot like any entity.
    #[test]
    fn a_leash_knots_uuid_resolves_to_its_post() {
        let knot = [7193, 0, 0, 1];
        let post = [4, 64, 6];
        let player = uuid_ints(offline_uuid("Player"));
        assert_eq!(
            resolve_pending_leash(
                Pending::Uuid(knot),
                player,
                None,
                |_| false,
                |_| None,
                |_| None,
                |u| (u == knot).then_some(post),
            ),
            Some((Holder::Knot(post), None))
        );
        // An entity that holds the UUID wins, as nothing else can share it.
        assert_eq!(
            resolve_pending_leash(
                Pending::Uuid(knot),
                player,
                None,
                |_| false,
                |u| (u == knot).then_some(17),
                |_| None,
                |_| Some(post),
            ),
            Some((Holder::Mob(17), None))
        );
    }

    #[test]
    fn legacy_player_uuid_without_an_integrated_owner_stays_pending() {
        let legacy = uuid_ints(offline_uuid("Player"));
        assert!(
            resolve_pending_leash(
                Pending::Uuid(legacy),
                legacy,
                None,
                |_| false,
                |_| None,
                |_| None,
                |_| None,
            )
            .is_none()
        );
    }
}
