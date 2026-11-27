#![cfg(test)]
#[cfg(test)]
pub(crate) mod attach_tests {
    use sunrise_core::geometry::Vec3;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    /// A game with resources but no level: a test pushes the mobs it needs
    /// and drives the lead code alone.
    fn game(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_gameplay::game::ServerGame<'_> {
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
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_gameplay::game::ServerGame::new(
            res,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.creative = false;
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap(),
        );
        game
    }

    /// The centre of a mob's box: `Leashable.leashDistanceTo`'s own end.
    #[cfg(feature = "client")]
    fn centre(m: &sunrise_sim::mobs::Mob) -> glam::DVec3 {
        let b = m.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        glam::DVec3::new(
            (lo.x + hi.x) / 2.0,
            (lo.y + hi.y) / 2.0,
            (lo.z + hi.z) / 2.0,
        )
    }

    /// `Leashable.canHaveALeashAttachedTo` measures with the leashee's own
    /// `leashSnapDistance`: a happy ghast's 16, a cow's default 12.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_lead_attaches_within_the_species_own_snap_distance() {
        use sunrise_sim::mobs::{Mob, Species};

        let mut resources = None;
        let mut game = game(&mut resources);
        for (id, kind, species) in [
            (1, "minecraft:happy_ghast", Species::HappyGhast),
            (2, "minecraft:cow", Species::Cow),
        ] {
            let kind = game.res.mob_types().get(kind).expect(kind).clone();
            game.mobs
                .push(Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0));
            assert_eq!(game.mobs.last().unwrap().species(), species);
        }
        let at = |index: usize, blocks: f64| {
            let c = centre(&game.mobs[index]);
            glam::DVec3::new(c.x + blocks, c.y, c.z)
        };
        assert!(
            game.can_attach_lead(0, at(0, 15.0)),
            "a happy ghast takes a lead 15 blocks out, as vanilla ties it"
        );
        assert!(
            !game.can_attach_lead(0, at(0, 17.0)),
            "past its 16-block snap distance it cannot be tied"
        );
        assert!(
            game.can_attach_lead(1, at(1, 11.0)),
            "a cow takes a lead inside the default distance"
        );
        assert!(
            !game.can_attach_lead(1, at(1, 13.0)),
            "a cow still snaps its lead past 12 blocks"
        );
    }

    /// `Entity.interact` with a lead: `setLeashedTo` → `LeashData.setLeashHolder`
    /// forgets the holder a save named. A mob whose saved holder never loaded
    /// keeps the player's lead past the 100-tick deadline, and no second lead
    /// falls from the forgotten one.
    #[test]
    #[ignore = "daily"]
    fn a_player_lead_replaces_a_saved_holder_that_never_loaded() {
        use sunrise_sim::mobs::{
            Mob,
            leash::{Holder, Pending},
        };

        let mut resources = None;
        let mut game = game(&mut resources);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:cow")
            .expect("cow")
            .clone();
        let mut cow = Mob::new(1, kind, Vec3::new(1.5, 64.0, 0.5), 0.0);
        cow.extra.leash_pending = Some(Pending::Uuid([9, 9, 9, 9]));
        game.mobs.push(cow);
        game.common.inventory[game.common.selected] =
            Some(sunrise_items::items::ItemStack::new("minecraft:lead", 1));

        assert!(game.interact_leash(0, "minecraft:lead"));
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Player));
        assert!(
            game.common.inventory[game.common.selected].is_none(),
            "the lead is spent"
        );
        game.mobs[0].age = 101;
        game.tick_leads();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Player));
        assert!(
            !game
                .dropped
                .iter()
                .any(|d| &*d.stack.item == "minecraft:lead"),
            "the forgotten holder drops no second lead"
        );
    }
}

#[cfg(test)]
#[path = "tests/leash_knot_uuid_tests.rs"]
pub(crate) mod leash_knot_uuid_tests;
