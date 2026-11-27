use crate::game::ServerGame;
use crate::game::native_entity_refs::tests::{
    bind_local_player, run_fresh_cases, with_resident_game,
};
use mod_api::{
    DeathLootError, EntityRef, Identifier, LivingRef, LootDamageSource,
    prelude::{EntitiesHost as _, PlayersHost as _},
};
use std::sync::Arc;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource as _},
};
use sunrise_items::items::ItemStack;
use sunrise_sim::{mobs::Mob, motion::PlayerMotion};
use sunrise_world::loot::{LootData, PackOverlay};

#[test]
#[ignore = "daily"]
fn rolls_live_death_context_without_dropping() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::rolls_live_death_context_without_dropping"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_ROLL_DEATH_LOOT_CHILD",
        &["entity_context"],
    ) {
        return;
    }
    with_resident_game(|game| {
        let files = [
            ("loot_table/test:death_preview".into(), vec![br#"{
                "type":"minecraft:entity","pools":[
                  {"rolls":1,"condition":{"type":"minecraft:entity_properties","entity":"this",
                      "predicate":{"minecraft:entity_type":"minecraft:pig"}},
                   "entries":[{"type":"minecraft:item","name":"minecraft:leather"}]},
                  {"rolls":1,"condition":{"type":"minecraft:killed_by_player"},
                   "entries":[{"type":"minecraft:item","name":"minecraft:emerald"}]},
                  {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot",
                      "modifier":{"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting","count":1}}]},
                  {"rolls":1,"condition":{"type":"minecraft:damage_source_properties","predicate":{
                      "tags":[{"id":"minecraft:mob_attack","expected":true}],"is_direct":false,
                      "source_entity":{"minecraft:entity_type":"minecraft:zombie"},
                      "direct_entity":{"minecraft:entity_type":"minecraft:zombie"}}},
                   "entries":[{"type":"minecraft:item","name":"minecraft:paper"}]},
                  {"rolls":0,"bonus_rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}]}
                ]}"#.to_vec()]),
            ("loot_table/test:seeded".into(), vec![br#"{
                "type":"minecraft:entity","random_sequence":"test:seeded","pools":[
                  {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond",
                      "modifier":{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":4}}}]}
                ]}"#.to_vec()]),
            ("loot_table/test:ridden".into(), vec![br#"{
                "type":"minecraft:entity","pools":[
                  {"rolls":1,"condition":{"type":"minecraft:entity_properties","entity":"this",
                      "predicate":{"minecraft:vehicle":{"minecraft:entity_type":"minecraft:zombie"}}},
                   "entries":[{"type":"minecraft:item","name":"minecraft:saddle"}]}
                ]}"#.to_vec()]),
        ];
        let loot = LootData::load(
            &game.options.resources,
            &PackOverlay::new(&files),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server.server_loot.0.set(Some(Arc::new(loot)));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        bind_local_player(game, 11, 123);
        let credited = game.player(123).unwrap();
        assert!(game.living_set_attribute_base(
            LivingRef::new(credited.entity()),
            "minecraft:luck",
            2.0
        ));

        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut victim = Mob::new(12, pig, Vec3::new(8.0, 64.0, 8.0), 0.0);
        victim.extra.death_loot_table = Some("test:death_preview".into());
        victim.extra.death_table_seed = 1;
        let health = victim.health;
        let mut killer = Mob::new(13, zombie.clone(), Vec3::ZERO, 0.0);
        let mut sword = ItemStack::new("minecraft:iron_sword", 1);
        sword.enchant("minecraft:looting", 3);
        killer.set_equipment_stack(0, Some(sword));
        game.mobs.push(victim);
        game.mobs.push(killer);
        game.mobs.push(Mob::new(14, zombie, Vec3::ZERO, 0.0));
        game.server.next_entity = game.server.next_entity.max(14);
        let victim = game.entity(12).unwrap();
        let killer = game.entity(13).unwrap();
        let direct = game.entity(14).unwrap();
        let mob_attack = Identifier::parse("minecraft:mob_attack").unwrap();
        let source = LootDamageSource::new(&mob_attack, Some(killer), Some(direct));
        let seeded = Identifier::parse("test:seeded").unwrap();
        let mut roll = |table, source, credited| {
            let stacks = victim
                .roll_death_loot(game, table, source, credited)
                .unwrap();
            stacks
                .iter()
                .map(|stack| {
                    (
                        mod_api::__private::item_key_id(stack.item()).unwrap(),
                        stack.count(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            roll(None, source, None),
            [
                ("minecraft:leather", 1),
                ("minecraft:iron_ingot", 4),
                ("minecraft:paper", 1),
            ]
        );
        assert_eq!(
            roll(None, source, Some(credited)),
            [
                ("minecraft:leather", 1),
                ("minecraft:emerald", 1),
                ("minecraft:iron_ingot", 4),
                ("minecraft:paper", 1),
                ("minecraft:gold_ingot", 1),
                ("minecraft:gold_ingot", 1),
            ]
        );
        assert_eq!(
            roll(None, LootDamageSource::new(&mob_attack, None, None), None),
            [("minecraft:leather", 1), ("minecraft:iron_ingot", 1),]
        );
        assert_eq!(
            roll(Some(&seeded), source, None),
            [("minecraft:diamond", 3)]
        );
        assert_eq!(
            roll(Some(&seeded), source, None),
            [("minecraft:diamond", 3)]
        );
        // The victim's vehicle is read as at its death.
        let ridden = Identifier::parse("test:ridden").unwrap();
        let saddles = |game: &mut ServerGame<'_>| {
            victim
                .roll_death_loot(game, Some(&ridden), source, None)
                .unwrap()
                .iter()
                .map(|stack| mod_api::__private::item_key_id(stack.item()).unwrap())
                .collect::<Vec<_>>()
        };
        assert!(saddles(game).is_empty());
        game.mobs[0].extra.mount = Some(14);
        assert_eq!(saddles(game), ["minecraft:saddle"]);
        game.mobs[0].extra.mount = None;
        assert!(game.dropped.is_empty());
        assert!(game.orbs.is_empty());
        assert_eq!(game.mobs[0].health, health);

        game.spawn_item(
            ItemStack::new("minecraft:stick", 1),
            [8.0, 64.0, 8.0],
            [0.0; 3],
            0,
        );
        let item = game.entity(game.dropped[0].id).unwrap();
        game.mobs[0].extra.death_table_seed = 0;
        *game.world.random = LegacyRandom::new(741);
        let expected_next = game.world.random.clone().next_long();
        let unknown_type = Identifier::parse("test:no_damage_type").unwrap();
        let unknown_table = Identifier::parse("test:no_table").unwrap();
        let stale = EntityRef::new(victim.epoch(), 9999);
        assert_eq!(
            stale.roll_death_loot(game, None, source, None),
            Err(DeathLootError::StaleEntity)
        );
        assert_eq!(
            item.roll_death_loot(game, None, source, None),
            Err(DeathLootError::NotLiving)
        );
        assert_eq!(
            victim.roll_death_loot(
                game,
                None,
                LootDamageSource::new(&unknown_type, None, None),
                None
            ),
            Err(DeathLootError::UnknownDamageType)
        );
        assert_eq!(
            victim.roll_death_loot(game, Some(&unknown_table), source, None),
            Err(DeathLootError::UnknownTable)
        );
        assert_eq!(game.world.random.next_long(), expected_next);

        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        assert_eq!(
            victim.roll_death_loot(game, None, source, None),
            Err(DeathLootError::Unavailable)
        );
    });
}
