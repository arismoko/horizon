#[cfg(test)]
#[path = "tests/fireworks_events_tests.rs"]
pub(crate) mod events_tests;

#[cfg(test)]
pub(crate) mod tests {

    /// `Projectile.shoot` turns a shot rocket to its motion at once.
    #[test]
    fn a_shot_rocket_starts_turned_to_its_motion() {
        use sunrise_core::geometry::Vec3;
        let mut rocket = sunrise_gameplay_common::game::fireworks::Rocket::remote(
            1,
            Vec3::ZERO,
            Vec3::new(0.0, 0.5, 0.0),
        );
        rocket.turn_to_shot();
        assert!((rocket.rotation[1] - 90.0).abs() < 1e-3);
        let mut rocket = sunrise_gameplay_common::game::fireworks::Rocket::remote(
            1,
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 1.0),
        );
        rocket.turn_to_shot();
        assert!((rocket.rotation[0] - 45.0).abs() < 1e-4);
        assert!(rocket.rotation[1].abs() < 1e-4);
    }

    /// A rocket draws its UUID's two longs first, then the motion's two triangles
    /// (four doubles), then `nextInt(6)` and `nextInt(7)` for the lifetime, all from
    /// the one random. The expected values come from `java.util.Random` replayed
    /// outside the game: a rocket whose UUID draw were skipped would read 30, 22
    /// and 26 here.
    #[test]
    fn a_rockets_lifetime_follows_its_uuid_and_motion_draws() {
        use sunrise_core::random::LegacyRandom;
        for (seed, flight, lifetime) in [
            (1, 1, 26),
            (12_345, 1, 26),
            (987_654_321, 1, 25),
            (1, 3, 46),
        ] {
            let mut own = LegacyRandom::new(seed);
            sunrise_sim::save::insecure_uuid(&mut own);
            assert_eq!(
                sunrise_gameplay_common::game::fireworks::rocket_draws(&mut own, flight).1,
                lifetime,
                "seed {seed}"
            );
        }
    }

    #[test]
    fn use_on_passes_only_while_gliding() {
        assert!(sunrise_gameplay_common::game::fireworks::use_on_passes(
            true
        ));
        assert!(!sunrise_gameplay_common::game::fireworks::use_on_passes(
            false
        ));
    }

    /// `FireworkRocketItem.use` on the server, while fall-flying: a lead on a
    /// mob within the 32-block box drops as an item and one
    /// `minecraft:item.lead.break` follows the player on NEUTRAL, heard by the
    /// acting player too. A lead outside that box, or held for another player,
    /// stays. Standing still does neither.
    #[test]
    #[ignore = "daily"]
    fn a_gliding_firework_drops_leads_and_plays_lead_break() {
        use crate::game::{Settings, load_resources};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{Mob, leash::Holder};
        use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let res = loaded.resources;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &res,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(10.5, 64.0, -3.25))
                .unwrap(),
        );
        let feet = {
            let p = game.player.as_ref().expect("a player").position;
            [p.x, p.y, p.z]
        };
        let kind = game
            .res
            .mob_types()
            .get("minecraft:cow")
            .expect("cow")
            .clone();
        // A server action keeps the guest identity while its hub is swapped out.
        game.server.lan_acting = Some(11);
        game.server.lan_acting_entity = Some(37);
        let me = game.acting_uuid();
        let someone_else = [me[0].wrapping_add(1), me[1], me[2], me[3]];
        let mut leashed = |id, at: Vec3, owner: [i32; 4]| {
            let mut mob = Mob::new(id, kind.clone(), at, 0.0);
            mob.extra.leash = Some(Holder::Player);
            mob.extra.leash_player = Some(owner);
            game.mobs.push(mob);
        };
        let here = Vec3::new(feet[0] + 1.0, feet[1], feet[2]);
        leashed(1, here, me);
        leashed(2, Vec3::new(feet[0] + 40.0, feet[1], feet[2]), me);
        leashed(3, here, someone_else);
        game.common.inventory[game.common.selected] = Some(sunrise_items::items::ItemStack::new(
            "minecraft:firework_rocket",
            1,
        ));
        let stack = game.inventory[game.selected].clone().unwrap();
        game.start_audio_capture();

        assert!(!game.boost_firework(&stack));
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Player));
        assert!(game.dropped.is_empty());
        assert!(game.server.pending_sounds.is_empty());

        game.player.as_mut().unwrap().fall_flying = true;
        assert!(game.boost_firework(&stack));
        assert!(
            game.mobs[0].extra.leash.is_none(),
            "the mob on the lead comes free"
        );
        assert_eq!(game.mobs[1].extra.leash, Some(Holder::Player));
        assert_eq!(game.mobs[2].extra.leash, Some(Holder::Player));
        let leads: Vec<_> = game
            .dropped
            .iter()
            .filter(|d| &*d.stack.item == "minecraft:lead")
            .collect();
        assert_eq!(leads.len(), 1);
        let heard = std::mem::take(&mut game.server.pending_sounds);
        let snaps: Vec<_> = heard
            .iter()
            .filter(|s| s.event == "minecraft:item.lead.break")
            .collect();
        assert_eq!(snaps.len(), 1);
        let snap = snaps[0];
        assert_eq!((snap.position, snap.volume, snap.pitch), (feet, 1.0, 1.0));
        assert_eq!(
            snap.entity,
            Some(37),
            "the snap must follow the acting guest"
        );
        assert_eq!(snap.source, 6, "SoundSource.NEUTRAL");
        assert_eq!(snap.except_entity, None, "vanilla excludes no player");
        let (packet, range) = sunrise_sim::lan::containers::captured_sound_packet(snap, None, 42);
        let sunrise_protocol::packets::play::Clientbound::SoundEntity(packet) = packet else {
            panic!("a firework lead snap must use the entity-bound packet");
        };
        assert_eq!((packet.id, packet.source, range), (37, 6, 16.0));
        assert_eq!(packet.sound.1.unwrap().0, "minecraft:item.lead.break");
    }
}
#[path = "fireworks/impact.rs"]
pub(crate) mod impact;
