#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_items::items::EntityTypeTags;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::Who;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::server_side::mounts::tests::sparkle_draws;

    use sunrise_gameplay::game::server_side::mounts::tests::equip;

    use sunrise_gameplay::game::server_side::mounts::tests::feed;

    use sunrise_gameplay::game::server_side::mounts::tests::mount;

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn steering_stick_break_converts_to_fishing_rod() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            game.riding = Some(21);
            let mut stick = ItemStack::new("minecraft:carrot_on_a_stick", 1);
            let max = game.items().get(&stick.item).unwrap().max_damage;
            stick.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Old Nag".into()));
            // One point short of breaking: the shared durability path adds wear.
            stick.damage = max - 8;
            let (updated, broken) = game.damage_stack(&stick, 7, true);
            assert!(broken.is_none());
            assert_eq!(updated.unwrap().damage, max - 1);
            stick.damage = max - 7;
            assert!(game.damage_stack(&stick, 0, true).1.is_none());
            let mut undamageable = stick.clone();
            undamageable.data_mut().set_other(
                "!minecraft:max_damage",
                Some(sunrise_core::nbt::Tag::Byte(1)),
            );
            assert!(
                game.damage_stack(&undamageable, max as i32, true)
                    .1
                    .is_none()
            );
            let selected = game.selected;
            game.inventory[selected] = Some(stick);
            assert!(game.use_steering_stick("minecraft:carrot_on_a_stick"));
            let rod = game.inventory[game.selected].as_ref().unwrap();
            assert_eq!(&*rod.item, "minecraft:fishing_rod");
            assert_eq!(rod.count, 1);
            assert_eq!(rod.damage, 0, "the rod starts whole");
            assert_eq!(rod.custom_name().as_deref(), Some("Old Nag"));
        });
    }
    /// A server-side game with nothing in it but an empty chunk, or None when
    /// the prepared resources are missing (CI has no vanilla data).
    pub(crate) fn server_game(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> Option<Box<sunrise_gameplay::game::ServerGame<'_>>> {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return None;
        }
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
        .ok()?;
        let mut options = Options::parse(std::iter::empty()).ok()??;
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
        .ok()?;
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.creative = false;
        game.options.dedicated = Some(Vec::new());
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        game.server.streaming.forced.insert((0, 0));
        Some(Box::new(game))
    }
    /// `AbstractChestedHorse.equipChest`: a tamed donkey or mule given a chest
    /// plays its chest sound at volume 1.0 (`AbstractChestedHorse`
    /// `playChestEquipsSound` uses `DONKEY_CHEST`, `Mule` overrides it with
    /// `MULE_CHEST`) and posts no mob equip event, which only the saddle and
    /// the armour do.
    #[test]
    #[ignore = "daily"]
    fn a_chest_on_a_donkey_or_mule_plays_its_own_sound_at_full_volume() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        for (kind, sound) in [
            ("minecraft:donkey", "minecraft:entity.donkey.chest"),
            ("minecraft:mule", "minecraft:entity.mule.chest"),
        ] {
            mount(&mut game, kind, 5, [2.5, 64.0, 2.5], 7);
            let selected = game.selected;
            game.inventory[selected] = Some(ItemStack::new("minecraft:chest", 1));
            game.start_audio_capture();
            sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
                .with(|events| *events.borrow_mut() = Some(Vec::new()));
            assert!(game.interact_mount(0, "minecraft:chest"), "{kind}");
            let sounds: Vec<_> = std::mem::take(&mut game.server.pending_sounds)
                .into_iter()
                .map(|s| (s.event, s.position, s.volume, s.pitch))
                .collect();
            game.stop_audio_capture();
            let equip_events = sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
                .with(|events| events.borrow_mut().take().unwrap_or_default());
            sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
                .with(|events| *events.borrow_mut() = None);
            assert!(game.mobs[0].extra.chested, "{kind} wears the chest");
            assert_eq!(sounds.len(), 1, "{kind}: {sounds:?}");
            assert_eq!(sounds[0].0, sound, "{kind} chest sound");
            assert_eq!(sounds[0].2, 1.0, "{kind} chest sound volume");
            assert!(
                (0.8..=1.2).contains(&sounds[0].3),
                "{kind} chest pitch {} outside vanilla's 0.8..1.2",
                sounds[0].3
            );
            assert!(
                equip_events.is_empty(),
                "{kind} posted a mob equip event: {equip_events:?}"
            );
        }
    }
    /// `AbstractHorse.handleEating`: a mouthful that ages a baby up draws the
    /// happy villager's position from the mount's own random (`getRandomX`,
    /// `getRandomY` and `getRandomZ` take a double each) before it grows; a
    /// mount that is not silent then eats with `1.0F + (nextFloat() -
    /// nextFloat()) * 0.2F` of its own random at its feet, and every used
    /// mouthful is a `minecraft:eat` game event. A silent mount eats
    /// noiselessly, drawing no pitch, and a locked baby grows not at all.
    #[test]
    #[ignore = "daily"]
    fn feeding_a_mount_draws_from_its_own_random_and_posts_eat() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        // A silent baby horse fed wheat: the particle's draws, the growth, the
        // event, and no eating sound (so no pitch draws).
        mount(&mut game, "minecraft:horse", 1, [1.5, 64.0, -2.5], 7);
        game.mobs[0].extra.age = -24000;
        game.mobs[0].silent = true;
        let level_random = game.world.random.clone().next_long();
        let (used, shown) = feed(&mut game, "minecraft:wheat");
        assert!(used, "wheat is horse food");
        let mut r = sunrise_core::random::LegacyRandom::new(7);
        sparkle_draws(&mut r);
        let at = [1.5, 64.0, -2.5];
        assert!(shown.sounds.is_empty(), "Silent: {shown:?}");
        assert_eq!(shown.events, vec![("minecraft:eat", at, Some(Who::Mob(1)))]);
        assert_eq!(game.mobs[0].extra.age, -23600, "wheat ages a horse by 20 s");
        assert_eq!(
            game.mobs[0].random.clone().next_double(),
            r.next_double(),
            "three doubles for the particle, none for a silent pitch"
        );
        assert_eq!(
            game.world.random.clone().next_long(),
            level_random,
            "the level's own random drew nothing"
        );

        // A hurt adult draws no particle and eats at its feet, its pitch from
        // its own random, after healing.
        mount(&mut game, "minecraft:horse", 2, [0.5, 64.0, 0.5], 11);
        game.mobs[0].health = game.mobs[0].attrs.max_health - 5.0;
        let max = game.mobs[0].attrs.max_health;
        let (used, shown) = feed(&mut game, "minecraft:wheat");
        assert!(used);
        let mut r = sunrise_core::random::LegacyRandom::new(11);
        let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
        let at = [0.5, 64.0, 0.5];
        assert_eq!(
            shown.sounds,
            vec![("minecraft:entity.horse.eat".to_string(), at, 1.0, pitch)],
            "{shown:?}"
        );
        assert_eq!(game.mobs[0].health, max - 3.0, "wheat heals 2");
        assert_eq!(shown.events, vec![("minecraft:eat", at, Some(Who::Mob(2)))]);

        // An age-locked baby that is tamed and whole eats nothing at all.
        mount(&mut game, "minecraft:horse", 3, [0.5, 64.0, 2.5], 13);
        game.mobs[0].extra.age = -24000;
        game.mobs[0].extra.age_locked = true;
        let (used, shown) = feed(&mut game, "minecraft:wheat");
        assert!(!used, "a locked baby leaves the wheat alone: {shown:?}");
        assert!(
            shown.sounds.is_empty() && shown.events.is_empty(),
            "{shown:?}"
        );
        assert_eq!(game.mobs[0].extra.age, -24000);
        assert_eq!(
            game.mobs[0].random.clone().next_double(),
            sunrise_core::random::LegacyRandom::new(13).next_double(),
            "nothing drawn"
        );

        // A baby camel fed cactus: `Camel.handleEating` draws the same particle,
        // then eats and posts EAT, the three doubles before the two floats.
        mount(&mut game, "minecraft:camel", 4, [0.5, 64.0, 4.5], 21);
        game.mobs[0].extra.age = -24000;
        let (used, shown) = feed(&mut game, "minecraft:cactus");
        assert!(used, "cactus is camel food");
        let mut r = sunrise_core::random::LegacyRandom::new(21);
        sparkle_draws(&mut r);
        let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
        let at = [0.5, 64.0, 4.5];
        assert_eq!(
            shown.sounds,
            vec![("minecraft:entity.camel.eat".to_string(), at, 1.0, pitch)],
            "{shown:?}"
        );
        assert_eq!(shown.events, vec![("minecraft:eat", at, Some(Who::Mob(4)))]);
        assert_eq!(game.mobs[0].extra.age, -23800, "camel cactus ages 10 s");
    }
    /// `LivingEntity.onEquipItem`: a saddle or horse armour plays its equip sound
    /// at pitch `1.0F`, at the mount's feet, seeded with the mount's own random's
    /// `nextLong`. A chest's sound is `AbstractChestedHorse.playChestEquipsSound`'s,
    /// pitched from two floats of that random at volume 1.0. A silent mount plays
    /// nothing (and a silent saddle or armour draws no seed either), and the
    /// level's random draws nothing in any of it.
    #[test]
    #[ignore = "daily"]
    fn saddling_and_armouring_play_at_pitch_one_from_the_mounts_own_random() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        let at = [1.5, 64.0, -2.5];
        mount(&mut game, "minecraft:horse", 1, at, 7);
        let level_random = game.world.random.clone().next_long();
        let (used, shown) = equip(&mut game, "minecraft:saddle");
        assert!(used, "a tamed horse takes the saddle: {shown:?}");
        assert_eq!(
            shown.sounds,
            vec![("minecraft:entity.horse.saddle".to_string(), at, 1.0, 1.0)],
            "{shown:?}"
        );
        assert_eq!(
            shown.events,
            vec![("minecraft:equip", at, Some(Who::Mob(1)))]
        );
        let mut r = sunrise_core::random::LegacyRandom::new(7);
        r.next_long();
        assert_eq!(
            game.mobs[0].random.clone().next_double(),
            r.next_double(),
            "one nextLong for the equip sound's seed, and nothing else"
        );
        assert_eq!(
            game.world.random.clone().next_long(),
            level_random,
            "the level's own random drew nothing"
        );

        // Horse armour takes the same path and seeds from the same random.
        let (used, shown) = equip(&mut game, "minecraft:iron_horse_armor");
        assert!(used, "a tamed horse takes armour: {shown:?}");
        assert_eq!(
            shown.sounds,
            vec![("minecraft:entity.horse.armor".to_string(), at, 1.0, 1.0)],
            "{shown:?}"
        );
        let mut r = sunrise_core::random::LegacyRandom::new(7);
        r.next_long();
        r.next_long();
        assert_eq!(
            game.mobs[0].random.clone().next_double(),
            r.next_double(),
            "a nextLong for each equip sound"
        );

        // A donkey's chest is pitched from its own random's two floats, at
        // volume 1.0 and still at its feet.
        let feet = [0.5, 64.0, 0.5];
        mount(&mut game, "minecraft:donkey", 2, feet, 11);
        let level_random = game.world.random.clone().next_long();
        let (used, shown) = equip(&mut game, "minecraft:chest");
        assert!(used, "a tamed donkey takes a chest: {shown:?}");
        let mut r = sunrise_core::random::LegacyRandom::new(11);
        let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
        assert_eq!(
            shown.sounds,
            vec![(
                "minecraft:entity.donkey.chest".to_string(),
                feet,
                1.0,
                pitch
            )],
            "{shown:?}"
        );
        assert_eq!(
            game.world.random.clone().next_long(),
            level_random,
            "the level's own random drew nothing"
        );

        // A silent mount plays no equip sound and draws no seed; the equip
        // event still goes out.
        let feet = [0.5, 64.0, 2.5];
        mount(&mut game, "minecraft:horse", 3, feet, 13);
        game.mobs[0].silent = true;
        let (used, shown) = equip(&mut game, "minecraft:saddle");
        assert!(used, "a silent horse still takes the saddle");
        assert!(shown.sounds.is_empty(), "{shown:?}");
        assert_eq!(
            shown.events,
            vec![("minecraft:equip", feet, Some(Who::Mob(3)))]
        );
        assert_eq!(
            game.mobs[0].random.clone().next_double(),
            sunrise_core::random::LegacyRandom::new(13).next_double(),
            "a silent mount's equip draws nothing"
        );
    }
    /// `CamelHusk.interact` sets `PersistenceRequired` before it hands the click
    /// on, so a husk a player saddled is kept past the monster category's 128
    /// blocks (`Mob.checkDespawn` reads `isPersistenceRequired`).
    #[test]
    #[ignore = "daily"]
    fn a_camel_husk_keeps_itself_once_clicked() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        mount(&mut game, "minecraft:camel_husk", 7, [1.5, 64.0, -2.5], 7);
        assert!(!game.mobs[0].persistent);
        assert!(
            !sunrise_sim::mobs::ai::requires_custom_persistence(&game.mobs[0]),
            "nothing else keeps it: the click must"
        );
        game.inventory[0] = Some(ItemStack::new("minecraft:saddle", 1));
        let target = Some(sunrise_gameplay_common::game::InteractTarget {
            id: 7,
            location: [0.0; 3],
        });
        assert!(game.interact_mob(target).unwrap(), "the saddle is used");
        assert!(game.mobs[0].extra.saddled(), "the saddle went on");
        assert!(game.mobs[0].persistent, "a clicked camel husk stays");
    }
}

#[cfg(test)]
pub(crate) mod off_level_tests {
    use super::*;
    use sunrise_gameplay::game::server_side::mounts::dismounts_underwater;

    #[test]
    #[ignore = "daily"]
    fn underwater_dismount_uses_the_levels_current_mount_tag() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::mounts::off_level_tests::underwater_dismount_uses_the_levels_current_mount_tag",
            || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    let tags = &game.items().entity_type_tags;
                    // Default membership from the 26.3 dismounts_underwater tag.
                    for (kind, expected) in [
                        ("minecraft:camel", true),
                        ("minecraft:chicken", true),
                        ("minecraft:donkey", true),
                        ("minecraft:happy_ghast", true),
                        ("minecraft:horse", true),
                        ("minecraft:llama", true),
                        ("minecraft:mule", true),
                        ("minecraft:pig", true),
                        ("minecraft:ravager", true),
                        ("minecraft:spider", true),
                        ("minecraft:strider", true),
                        ("minecraft:trader_llama", true),
                        ("minecraft:zombie_horse", true),
                        ("minecraft:skeleton_horse", false),
                        ("minecraft:camel_husk", false),
                        ("minecraft:cow", false),
                    ] {
                        let mob_type = game.res.mob_types().get(kind).unwrap().clone();
                        let mob = Mob::new(0, mob_type, Vec3::ZERO, 0.0);
                        assert_eq!(dismounts_underwater(tags, &mob), expected, "{kind}");
                    }

                    for (id, kind, mount) in [
                        (71, "minecraft:pig", None),
                        (72, "minecraft:zombie", Some(71)),
                        (73, "minecraft:cow", None),
                        (74, "minecraft:zombie", Some(73)),
                    ] {
                        let kind = game.res.mob_types().get(kind).unwrap().clone();
                        let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                        mob.extra.mount = mount;
                        mob.fluids.eye_in_water = true;
                        game.mobs.push(mob);
                    }
                    let mut items = game.items().clone();
                    items.set_entity_type_tags(EntityTypeTags::from_ordered(
                        [(
                            "minecraft:dismounts_underwater".into(),
                            vec!["minecraft:cow".into()],
                        )]
                        .into(),
                        items.entity_type_tags.ids().clone(),
                    ));
                    game.server_items = Some(std::sync::Arc::new(items));

                    game.dismount_underwater_mob_riders();
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert_eq!(game.mob_object(74).unwrap().extra.mount, None);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn underwater_dismount_skips_deep_frozen_descendants() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for (id, mount) in [(73, Some(72)), (74, Some(73)), (75, None), (76, Some(75))] {
                let mut m = sunrise_sim::mobs::Mob::new(id, pig.clone(), Vec3::ZERO, 0.0);
                m.extra.mount = mount;
                m.fluids.eye_in_water = true;
                game.mobs.push(m);
            }
            let ghost = sunrise_sim::mobs::Mob::new(72, pig, Vec3::ZERO, 0.0);
            game.persist.off_level.insert(
                72,
                sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(ghost),
            );
            game.dismount_underwater_mob_riders();
            assert_eq!(game.mob_object(74).unwrap().extra.mount, Some(73));
            assert_eq!(game.mob_object(76).unwrap().extra.mount, None);
        });
    }
}

#[cfg(test)]
#[path = "mounts/camel_jump_tests.rs"]
pub(crate) mod camel_jump_tests;

#[cfg(test)]
#[path = "mounts/sneak_dismount_tests.rs"]
pub(crate) mod sneak_dismount_tests;
#[cfg(test)]
#[path = "tests/strider_lava_boarding_tests.rs"]
pub(crate) mod strider_lava_boarding_tests;

#[cfg(test)]
#[path = "tests/steerable_boarding_tests.rs"]
pub(crate) mod steerable_boarding_tests;

#[cfg(test)]
#[path = "tests/ride_dismount_tests.rs"]
pub(crate) mod ride_dismount_tests;

#[cfg(test)]
#[path = "tests/ride_seat_tests.rs"]
pub(crate) mod ride_seat_tests;
use sunrise_gameplay_common::game::tables::GameItems as _;
