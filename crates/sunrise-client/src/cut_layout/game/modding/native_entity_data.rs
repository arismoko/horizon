#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::{EntityRef, MobCategory};
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::{JavaString, Tag};
    use sunrise_game_data::text::Text;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5;
    use sunrise_sim::{entities::ArmorStand, mobs::Mob};

    /// The entity refs fixture with chunk (0, 0) loaded: a handle resolves only for an
    /// entity in an accessible section (`LevelEntityGetter`), and every entity here stands in it.
    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            test(game);
        });
    }

    fn push_mob(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
        kind: &str,
    ) -> EntityRef {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        game.mobs
            .push(Mob::new(id, kind, Vec3::new(0.5, 70.0, 0.5), 0.0));
        game.entity(id).unwrap()
    }

    /// `EntityType.getCategory()` read from the type alone (#6044), matching what a live mob
    /// of the type reports.
    #[test]
    #[ignore = "daily"]
    fn an_entity_types_category_reads_without_an_entity() {
        with_game(|game| {
            let key = |id: &str| mod_api::__private::resolved_entity_type_key(id).unwrap();
            for (id, expected) in [
                ("minecraft:zombie", MobCategory::Monster),
                ("minecraft:cow", MobCategory::Creature),
                ("minecraft:bat", MobCategory::Ambient),
                ("minecraft:axolotl", MobCategory::Axolotls),
                (
                    "minecraft:glow_squid",
                    MobCategory::UndergroundWaterCreature,
                ),
                ("minecraft:dolphin", MobCategory::WaterCreature),
                ("minecraft:cod", MobCategory::WaterAmbient),
                ("minecraft:ender_dragon", MobCategory::Monster),
                ("minecraft:item", MobCategory::Misc),
                ("minecraft:player", MobCategory::Misc),
            ] {
                assert_eq!(key(id).category(&*game), Some(expected), "{id}");
            }
            let zombie = push_mob(game, 41, "minecraft:zombie");
            assert_eq!(
                zombie.classification(&*game),
                key("minecraft:zombie").category(&*game)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn live_entity_tags_lend_exact_utf16_without_changing_command_text() {
        with_game(|game| {
            let mob = push_mob(game, 11, "minecraft:zombie");
            assert!(mob.tags(game).unwrap().iter_utf16().next().is_none());

            game.put_saved(
                mob.id(),
                "Tags",
                Some(Tag::List(vec![
                    Tag::String("tag".into()),
                    Tag::String("\u{1f680}".into()),
                    Tag::Byte(7),
                    Tag::String(JavaString(vec![0xD800])),
                ])),
            );

            let tags = mob.tags(game).unwrap();
            let expected: [&[u16]; 3] = [&[0x0074, 0x0061, 0x0067], &[0xD83D, 0xDE80], &[0xD800]];
            assert_eq!(tags.iter_utf16().collect::<Vec<_>>(), expected);
            assert!(tags.contains("tag"));
            assert!(tags.contains("\u{1f680}"));
            assert!(!tags.contains("\u{fffd}"));

            // `/tag ... list` still receives the same strings, in the same order.
            let target = game.native_entity_target(mob).unwrap();
            assert_eq!(game.tags_of(target), ["tag", "\u{1f680}", "\u{fffd}"]);

            assert!(mob.discard(game));
            assert!(mob.tags(game).is_none());
        });
    }

    /// `Entity.getCustomName` (#5380): None until a name is set, then the name as set, style
    /// included, for a mob, an armor stand and a kind whose name lives in its saved data (a
    /// dropped item); None again once taken away. A spawn's plain mob name, a cushion's own
    /// name and an orb's saved name read too; the player has none.
    #[test]
    #[ignore = "daily"]
    fn custom_name_reads_the_styled_name_or_none() {
        with_game(|game| {
            let mob = push_mob(game, 15, "minecraft:zombie");
            assert_eq!(mob.custom_name(game).unwrap(), None);
            let name: mod_api::Text = Text::colored("Grumbles", 0xFF5555).try_into_api().unwrap();
            assert!(mob.set_custom_name(game, Some(name.clone())).unwrap());
            assert_eq!(mob.custom_name(game).unwrap(), Some(name.clone()));
            assert!(mob.set_custom_name(game, None).unwrap());
            assert_eq!(mob.custom_name(game).unwrap(), None);

            game.armor_stands.push(ArmorStand::new(16, Vec3::ZERO, 0.0));
            let stand = game.entity(16).unwrap();
            assert_eq!(stand.custom_name(game).unwrap(), None);
            assert!(stand.set_custom_name(game, Some(name.clone())).unwrap());
            assert_eq!(stand.custom_name(game).unwrap(), Some(name.clone()));

            let stone = sunrise_items::items::ItemStack::new("minecraft:stone", 1);
            game.spawn_item(stone, [0.5, 70.0, 0.5], [0.0; 3], 0);
            let item = game.entity(game.dropped.last().unwrap().id).unwrap();
            assert_eq!(item.custom_name(game).unwrap(), None);
            assert!(item.set_custom_name(game, Some(name.clone())).unwrap());
            assert_eq!(item.custom_name(game).unwrap(), Some(name));

            // A mob a spawn named with a plain name: no styled text, the name all the same.
            game.mobs
                .iter_mut()
                .find(|m| m.id == 15)
                .unwrap()
                .custom_name = Some("Plain".into());
            assert_eq!(
                mob.custom_name(game).unwrap(),
                Some(mod_api::Text::literal("Plain"))
            );

            // A cushion keeps its name on itself, not in the saved data.
            game.cushions.push(sunrise_sim::entities::Cushion {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 17,
                position: Vec3::new(2.5, 70.0, 2.5),
                pos: [2, 70, 2],
                yaw: 0.0,
                color: 0,
                check_interval: 0,
                custom_name: Some(Tag::String("Seat".into())),
            });
            let cushion = game.entity(17).unwrap();
            assert_eq!(
                cushion.custom_name(game).unwrap(),
                Some(mod_api::Text::literal("Seat"))
            );

            // An experience orb, a kind whose name only its saved `CustomName` holds.
            game.spawn_orbs([0.5, 70.0, 0.5], 3);
            let orb_id = game.orbs.last().unwrap().id;
            let orb = game.entity(orb_id).unwrap();
            assert_eq!(orb.custom_name(game).unwrap(), None);
            game.put_saved(orb_id, "CustomName", Some(Tag::String("Shiny".into())));
            assert_eq!(
                orb.custom_name(game).unwrap(),
                Some(mod_api::Text::literal("Shiny"))
            );

            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            assert_eq!(player.custom_name(game).unwrap(), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn custom_name_and_visibility_are_saved_and_load_again() {
        with_game(|game| {
            let mob = push_mob(game, 12, "minecraft:zombie");
            assert_eq!(mob.custom_name_visible(game), Some(false));
            let name = Text::colored("Grumbles", 0xFF5555);
            assert!(
                mob.set_custom_name(game, Some(name.try_into_api().unwrap()))
                    .unwrap()
            );
            assert!(mob.set_custom_name_visible(game, true));
            assert_eq!(mob.name(game).as_deref(), Some("Grumbles"));
            assert_eq!(mob.custom_name_visible(game), Some(true));
            let saved = game.entity_nbt(mob).unwrap();
            assert_eq!(saved.get(&"CustomNameVisible".into()), Some(&Tag::Byte(1)));

            // Taken away: neither the name nor the flag is saved.
            assert!(mob.set_custom_name(game, None).unwrap());
            assert!(mob.set_custom_name_visible(game, false));
            assert_ne!(mob.name(game).as_deref(), Some("Grumbles"));
            assert_eq!(mob.custom_name_visible(game), Some(false));
            let cleared = game.entity_nbt(mob).unwrap();
            assert!(!cleared.contains_key(&"CustomName".into()));
            assert!(!cleared.contains_key(&"CustomNameVisible".into()));

            // The saved data loads back with the styled name and the flag.
            let t = game.native_entity_target(mob).unwrap();
            game.apply_entity_nbt(t, Tag::Compound(saved)).unwrap();
            let m = game.mobs.iter().find(|m| m.id == 12).unwrap();
            assert_eq!(m.custom_name_text.as_ref(), Some(&name));
            assert_eq!(mob.name(game).as_deref(), Some("Grumbles"));
            assert_eq!(mob.custom_name_visible(game), Some(true));

            // An armor stand keeps its name in its own field.
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let stand = game.entity(13).unwrap();
            assert!(
                stand
                    .set_custom_name(game, Some(mod_api::Text::literal("Sign")))
                    .unwrap()
            );
            assert_eq!(stand.name(game).as_deref(), Some("Sign"));
            let saved = game.entity_nbt(stand).unwrap();
            let loaded = saved
                .get(&"CustomName".into())
                .map(sunrise_sim::entity_nbt::name_text);
            assert_eq!(loaded, Some(Text::literal("Sign")));
            assert!(stand.set_custom_name(game, None).unwrap());
            let saved = game.entity_nbt(stand).unwrap();
            assert!(!saved.contains_key(&"CustomName".into()));

            // The dragon keeps no name: refused, and a stale handle too.
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon.replace(dragon);
            let dragon = game.entity(14).unwrap();
            assert!(
                !dragon
                    .set_custom_name(game, Some(mod_api::Text::literal("Puff")))
                    .unwrap()
            );
            assert!(!dragon.set_custom_name_visible(game, true));
            assert!(mob.discard(game));
            assert!(
                !mob.set_custom_name(game, Some(mod_api::Text::literal("Gone")))
                    .unwrap()
            );
            assert_eq!(mob.custom_name_visible(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn classification_is_the_types_category() {
        with_game(|game| {
            for (id, kind, category) in [
                (20, "minecraft:zombie", MobCategory::Monster),
                (21, "minecraft:cow", MobCategory::Creature),
                (22, "minecraft:bat", MobCategory::Ambient),
                (23, "minecraft:axolotl", MobCategory::Axolotls),
                (
                    24,
                    "minecraft:glow_squid",
                    MobCategory::UndergroundWaterCreature,
                ),
                (25, "minecraft:dolphin", MobCategory::WaterCreature),
                (26, "minecraft:cod", MobCategory::WaterAmbient),
            ] {
                let mob = push_mob(game, id, kind);
                assert_eq!(mob.classification(game), Some(category), "{kind}");
            }
            game.armor_stands.push(ArmorStand::new(27, Vec3::ZERO, 0.0));
            let stand = game.entity(27).unwrap();
            assert_eq!(stand.classification(game), Some(MobCategory::Misc));
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 28;
            game.dragon.replace(dragon);
            let dragon = game.entity(28).unwrap();
            assert_eq!(dragon.classification(game), Some(MobCategory::Monster));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_entity_is_in_its_level_from_its_join_until_its_removal() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, Host, ServerHost};
        use std::sync::{Arc, Mutex};

        /// Whether the joining entity was in its level while the hook ran.
        struct Observer(Arc<Mutex<Vec<bool>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                let host: &dyn Host = host;
                let entity = host.entity(event.subject.id()).unwrap();
                self.0.lock().unwrap().push(entity.added_to_level(host));
                Flow::Continue
            }
        }
        let during = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "level-membership-observer",
            Box::new(Observer(during.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.native_joins_skip();
                game.server.next_entity += 1;
                let id = game.server.next_entity;
                let cow = push_mob(game, id, "minecraft:cow");
                // In the lists, but not yet let in.
                assert!(!cow.added_to_level(game));
                game.native_joins_flush(false, 0);
                assert_eq!(*during.lock().unwrap(), [false]);
                assert!(cow.added_to_level(game));
                assert!(cow.discard(game));
                assert!(!cow.added_to_level(game));
            });
        });
    }
}
