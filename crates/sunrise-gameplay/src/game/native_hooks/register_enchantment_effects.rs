/// Register value and entity effect codecs before loading enchantment content.
pub fn native_register_enchantment_effects(
    context: &mod_api::RegistrationContext,
    effects: &mut mod_api::__private::EnchantmentEffectRegistry,
) {
    crate::game::native_hooks::fire::fire_register_enchantment_effects(context, |listen| {
        effects.register_listener(listen)
    });
}

#[cfg(test)]
mod tests {
    use mod_api::{
        EnchantmentEntityEffect, EnchantmentEntityEffectCodec, EnchantmentEntityEffectContext,
        EnchantmentValueEffect, EnchantmentValueEffectCodec, Hooks, ModEntry, RandomSource,
        RegisterEnchantmentEffectsEvent, ServerHost,
    };
    use mod_platform::testing::with_mods;
    use serde_json::Value;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::tables::GameItems as _;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

    type Observations = Arc<Mutex<Vec<(u32, f32)>>>;
    type Hits = Arc<Mutex<Vec<(String, u32, u32)>>>;
    struct ValueCodec(Observations);
    struct ValueEffect {
        reads: Observations,
        factor: f32,
        add_level: bool,
    }
    impl EnchantmentValueEffectCodec for ValueCodec {
        fn decode(&self, fields: &Value) -> Result<Arc<dyn EnchantmentValueEffect>, String> {
            assert!(fields.get("type").is_none());
            let factor = fields["factor"].as_f64().ok_or("missing factor")? as f32;
            Ok(Arc::new(ValueEffect {
                reads: self.0.clone(),
                factor,
                add_level: fields["level"] == true,
            }))
        }
    }
    impl EnchantmentValueEffect for ValueEffect {
        fn process(&self, level: u32, _: &mut dyn RandomSource, value: f32) -> f32 {
            self.reads.lock().unwrap().push((level, value));
            value * self.factor + if self.add_level { level as f32 } else { 0.0 }
        }
    }
    struct HitCodec(Hits);
    struct HitEffect(Hits, String);
    impl EnchantmentEntityEffectCodec for HitCodec {
        fn decode(&self, fields: &Value) -> Result<Arc<dyn EnchantmentEntityEffect>, String> {
            Ok(Arc::new(HitEffect(
                self.0.clone(),
                fields["label"].as_str().ok_or("missing label")?.into(),
            )))
        }
    }
    impl EnchantmentEntityEffect for HitEffect {
        fn apply(&self, host: &mut dyn ServerHost, context: &EnchantmentEntityEffectContext<'_>) {
            let holder = context.holder.unwrap();
            assert_eq!(host.entity(holder.id()), Some(holder.entity()));
            match self.1.as_str() {
                "tick-main" | "tick-feet" => {
                    let slot = if self.1 == "tick-main" {
                        mod_api::EquipmentSlot::MainHand
                    } else {
                        mod_api::EquipmentSlot::Feet
                    };
                    assert_eq!(context.slot, Some(slot));
                    assert_eq!(context.victim, None);
                    assert_eq!(context.causing, None);
                    assert_eq!(context.direct, None);
                    assert_eq!(context.affected, holder.entity());
                    assert_eq!(context.affected.position(host), Some(context.position));
                }
                label => {
                    assert_eq!(context.victim, Some(context.affected));
                    if label == "equipped" {
                        assert_eq!(holder.id(), context.affected.id());
                        assert_eq!(context.slot, Some(mod_api::EquipmentSlot::Feet));
                        assert_eq!(context.item.damage(), 22);
                        assert_eq!(context.causing.map(|entity| entity.id()), Some(7));
                    } else {
                        assert_eq!(context.item.damage(), 11);
                        assert_eq!(context.causing, Some(holder.entity()));
                    }
                    if label == "sweep-before" {
                        assert!(
                            context
                                .affected
                                .fire_ticks(host)
                                .is_some_and(|ticks| ticks <= 0)
                        );
                    } else if label == "sweep-after" {
                        assert!(
                            context
                                .affected
                                .fire_ticks(host)
                                .is_some_and(|ticks| ticks > 0)
                        );
                    }
                }
            }
            self.0
                .lock()
                .unwrap()
                .push((self.1.clone(), holder.id(), context.affected.id()));
        }
    }
    struct PanicCodec(Arc<AtomicUsize>);
    impl EnchantmentEntityEffectCodec for PanicCodec {
        fn decode(&self, fields: &Value) -> Result<Arc<dyn EnchantmentEntityEffect>, String> {
            assert_ne!(fields["fail_decode"], true, "codec fixture panic");
            Ok(Arc::new(Self(self.0.clone())))
        }
    }
    impl EnchantmentEntityEffect for PanicCodec {
        fn apply(&self, _: &mut dyn ServerHost, _: &EnchantmentEntityEffectContext<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("effect fixture panic");
        }
    }
    struct TestMod {
        reads: Observations,
        hits: Hits,
        cancel: Arc<AtomicBool>,
        panics: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for TestMod {
        fn living_incoming_damage(
            &self,
            _: &mut dyn mod_api::HitHost,
            _: &mut mod_api::LivingIncomingDamageEvent<'_>,
        ) -> mod_api::Flow {
            if self.cancel.load(Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }

        fn register_enchantment_effects(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut RegisterEnchantmentEffectsEvent<'_>,
        ) {
            let id = |s| mod_api::Identifier::parse(s).unwrap();
            assert!(
                event
                    .effects
                    .register_value(
                        id("minecraft:add"),
                        Arc::new(ValueCodec(self.reads.clone()))
                    )
                    .is_err()
            );
            assert!(
                event
                    .effects
                    .register_entity(
                        id("minecraft:ignite"),
                        Arc::new(HitCodec(self.hits.clone()))
                    )
                    .is_err()
            );
            event
                .effects
                .register_value(
                    id("damage_test:value"),
                    Arc::new(ValueCodec(self.reads.clone())),
                )
                .unwrap();
            event
                .effects
                .register_entity(id("damage_test:hit"), Arc::new(HitCodec(self.hits.clone())))
                .unwrap();
            event
                .effects
                .register_entity(
                    id("damage_test:panic"),
                    Arc::new(PanicCodec(self.panics.clone())),
                )
                .unwrap();
        }
    }

    fn with_codec_game(
        definitions: &[(&str, &[&str], Value)],
        run: impl FnOnce(&mut crate::game::ServerGame<'_>, &Hits, &Observations, &AtomicBool),
    ) {
        let hits = Hits::default();
        let reads = Observations::default();
        let cancel = Arc::new(AtomicBool::new(false));
        with_mods(
            vec![ModEntry::new(
                "damage_test",
                Box::new(TestMod {
                    hits: hits.clone(),
                    reads: reads.clone(),
                    cancel: cancel.clone(),
                    panics: Arc::default(),
                }),
            )]
            .into(),
            || {
                // This fixture is called only in a fresh child: startup freezes the codec table.
                super::super::super::mods::register_enchantment_effects();
                crate::game::native_entity_refs::tests::with_game(|game| {
                    let root = tempfile::tempdir().unwrap();
                    game.server.world_root = root.path().to_owned();
                    game.enter_level(Dim::Overworld).unwrap();
                    // A mod reaches an entity only in an accessible section.
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    game.options.dedicated = Some(Vec::new());
                    let pack = root.path().join("datapacks/effects");
                    let entries = pack.join("data/damage_test/enchantment");
                    std::fs::create_dir_all(&entries).unwrap();
                    std::fs::write(
                        pack.join("pack.mcmeta"),
                        r#"{"pack":{"description":"codec fixture"}}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        root.path().join("datapacks.json"),
                        r#"{"enabled":["file/effects"],"disabled":[]}"#,
                    )
                    .unwrap();
                    for (id, slots, effects) in definitions {
                        let definition = serde_json::json!({
                            "description":"Codec fixture", "supported_items":"minecraft:stick",
                            "weight":1, "max_level":1, "anvil_cost":1,
                            "min_cost":{"base":1,"per_level_above_first":0},
                            "max_cost":{"base":1,"per_level_above_first":0},
                            "slots":slots, "effects":effects
                        });
                        std::fs::write(
                            entries.join(format!("{id}.json")),
                            serde_json::to_vec(&definition).unwrap(),
                        )
                        .unwrap();
                    }
                    let stack = game.reload_datapacks(false, &[]).unwrap();
                    let resources = game.options.resources.clone();
                    game.server.world_tables.0 = game
                        .stage_world_tables(&stack, false, &resources)
                        .unwrap()
                        .tables;
                    run(game, &hits, &reads, &cancel);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn sweep_applies_victim_effect_then_attacker_tree_only_on_a_landed_hit() {
        use crate::game::{
            mace::Spared,
            native_entity_refs::tests::{bind_local_player, run_fresh_cases},
        };
        if run_fresh_cases(
            "game::native_hooks::register_enchantment_effects::tests::sweep_applies_victim_effect_then_attacker_tree_only_on_a_landed_hit",
            "SUNRISE_ENCHANTMENT_EFFECT_CHILD",
            &["sweep"],
        ) {
            return;
        }
        with_codec_game(
            &[
                (
                    "sweep",
                    &["mainhand"],
                    serde_json::json!({
                        "minecraft:damage":[{"effect":{"type":"damage_test:value","factor":2}}],
                        "minecraft:post_attack":[{"enchanted":"attacker","affected":"victim","effect":{
                            "type":"minecraft:all_of","effects":[
                                {"type":"damage_test:hit","label":"sweep-before"},
                                {"type":"minecraft:ignite","duration":1},
                                {"type":"damage_test:hit","label":"sweep-after"}
                            ]
                        }}]
                    }),
                ),
                (
                    "swept_boots",
                    &["feet"],
                    serde_json::json!({
                        "minecraft:post_attack":[{"enchanted":"victim","affected":"victim","effect":{"type":"damage_test:hit","label":"equipped"}}]
                    }),
                ),
            ],
            |game, hits, reads, cancel| {
                let mut player = PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
                player.ground.on_ground = true;
                game.player = Some(player);
                bind_local_player(game, 7, 1);
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs
                    .push(Mob::new(21, kind.clone(), Vec3::new(1.0, 64.0, 0.0), 0.0));
                let mut victim = Mob::new(22, kind, Vec3::new(1.5, 64.0, 0.0), 0.0);
                let mut boots = ItemStack::new("minecraft:stick", 1);
                boots.damage = 22;
                boots.enchant("damage_test:swept_boots", 1);
                victim
                    .extra
                    .set_slot_stack(sunrise_items::equipment::FEET, Some(boots));
                let health = victim.health;
                game.mobs.push(victim);
                let mut weapon = ItemStack::new("minecraft:stick", 1);
                weapon.damage = 11;
                weapon.enchant("damage_test:sweep", 1);
                game.common.inventory[game.common.selected] = Some(weapon);
                let primary = game.mobs[0].bounds();

                cancel.store(true, Ordering::Relaxed);
                game.sweep_damage(primary, Spared::Mob(21), 1.0);
                assert_eq!(game.mobs[1].health, health);
                assert!(hits.lock().unwrap().is_empty());
                cancel.store(false, Ordering::Relaxed);
                game.sweep_damage(primary, Spared::Mob(21), 1.0);
                assert_eq!(health - game.mobs[1].health, 2.0);
                assert_eq!(*reads.lock().unwrap(), [(1, 1.0), (1, 1.0)]);
                assert_eq!(
                    *hits.lock().unwrap(),
                    [
                        (String::from("equipped"), 22, 22),
                        (String::from("sweep-before"), 7, 22),
                        (String::from("sweep-after"), 7, 22),
                    ]
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn borrowed_mob_tick_applies_equipment_effects_once_in_slot_order() {
        use crate::game::native_entity_refs::tests::{
            prepare_entity_ticking_neighborhood, run_fresh_cases,
        };
        if run_fresh_cases(
            "game::native_hooks::register_enchantment_effects::tests::borrowed_mob_tick_applies_equipment_effects_once_in_slot_order",
            "SUNRISE_ENCHANTMENT_EFFECT_CHILD",
            &["mob-tick"],
        ) {
            return;
        }
        let effect = |label| serde_json::json!({"minecraft:tick":[{"effect":{"type":"damage_test:hit","label":label}}]});
        with_codec_game(
            &[
                ("tick_main", &["mainhand"], effect("tick-main")),
                ("tick_feet", &["feet"], effect("tick-feet")),
            ],
            |game, hits, _, _| {
                prepare_entity_ticking_neighborhood(
                    game,
                    sunrise_core::position::ChunkPos::new(0, 0),
                );
                assert!(game.entity_ticking_area()(1, 0));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut mob = Mob::new(71, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
                mob.no_ai = true;
                mob.no_gravity = true;
                mob.ground.on_ground = true;
                for (slot, id) in [
                    (sunrise_items::equipment::FEET, "damage_test:tick_feet"),
                    (sunrise_items::equipment::HEAD, "damage_test:tick_feet"),
                    (sunrise_items::equipment::MAINHAND, "damage_test:tick_main"),
                ] {
                    let mut stack = ItemStack::new("minecraft:stick", 1);
                    stack.enchant(id, 1);
                    mob.extra.set_slot_stack(slot, Some(stack));
                }
                game.mobs.push(mob);
                for _ in 0..2 {
                    game.tick_creature_entities().unwrap();
                    assert_eq!(
                        std::mem::take(&mut *hits.lock().unwrap()),
                        [
                            (String::from("tick-main"), 71, 71),
                            (String::from("tick-feet"), 71, 71)
                        ]
                    );
                }
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn decoded_enchantment_effects_run_on_real_hits_and_rebind_on_load() {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::native_hooks::register_enchantment_effects::tests::decoded_enchantment_effects_run_on_real_hits_and_rebind_on_load",
            "SUNRISE_ENCHANTMENT_EFFECT_CHILD",
            &["combat"],
        ) {
            return;
        }
        let reads: Observations = Default::default();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let panics = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "damage_test",
                Box::new(TestMod {
                    reads: reads.clone(),
                    hits: hits.clone(),
                    cancel: cancel.clone(),
                    panics: panics.clone(),
                }),
            )]
            .into(),
            || {
                // Exercise the generated registrar dispatch, then the same publication as startup.
                super::super::super::mods::register_enchantment_effects();
                crate::game::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let root = std::env::temp_dir()
                        .join(format!("sunrise-damage-effects-{}", std::process::id()));
                    let pack = root.join("datapacks/effects");
                    let definitions = pack.join("data/damage_test/enchantment");
                    std::fs::create_dir_all(&definitions).unwrap();
                    std::fs::write(
                        pack.join("pack.mcmeta"),
                        r#"{"pack":{"description":"damage fixture"}}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        root.join("datapacks.json"),
                        r#"{"enabled":["file/effects"],"disabled":[]}"#,
                    )
                    .unwrap();
                    let definition = serde_json::json!({
                        "description":"Damage fixture", "supported_items":"minecraft:stick", "weight":1, "max_level":2,
                        "min_cost":{"base":1,"per_level_above_first":0}, "max_cost":{"base":1,"per_level_above_first":0},
                        "anvil_cost":1,"slots":["mainhand"],
                        "effects":{"minecraft:post_attack":[
                            {"enchanted":"attacker","affected":"victim","effect":{"type":"all_of","effects":[
                                {"type":"damage_test:hit","label":"first"},
                                {"type":"minecraft:ignite","duration":1},
                                {"type":"damage_test:panic"},
                                {"type":"damage_test:hit","label":"second"}
                            ]}}
                        ], "minecraft:damage":[{"effect":{"type":"minecraft:all_of","effects":[
                            {"type":"minecraft:add","value":2}, {"type":"damage_test:value","factor":2},
                            {"type":"minecraft:all_of","effects":[{"type":"damage_test:value","factor":1,"level":true}]}
                        ]}}]}
                    });
                    std::fs::write(
                        definitions.join("force.json"),
                        serde_json::to_vec(&definition).unwrap(),
                    )
                    .unwrap();
                    let mut equipment = definition.clone();
                    equipment["supported_items"] = serde_json::json!("minecraft:stick");
                    equipment["slots"] = serde_json::json!(["feet"]);
                    equipment["effects"] = serde_json::json!({"minecraft:post_attack":[{
                        "enchanted":"victim", "affected":"victim",
                        "effect":{"type":"damage_test:hit", "label":"equipped"}
                    }]});
                    std::fs::write(
                        definitions.join("equipment.json"),
                        serde_json::to_vec(&equipment).unwrap(),
                    )
                    .unwrap();
                    game.server.world_root = root.clone();
                    let stack = game.reload_datapacks(false, &[]).unwrap();
                    let resources = game.options.resources.clone();
                    game.server.world_tables.0 = game
                        .stage_world_tables(&stack, false, &resources)
                        .unwrap()
                        .tables;
                    assert!(
                        game.stations()
                            .enchantments
                            .get("damage_test:force")
                            .is_some()
                    );
                    // A mod reaches an entity only in an accessible section.
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
                    player.entity_id = 7;
                    player.ground.on_ground = true;
                    game.player = Some(player);
                    crate::game::native_entity_refs::tests::bind_local_player(game, 7, 1);
                    game.server.lan_acting_uuid = Some(1);
                    game.server.lan_acting_entity = Some(7);
                    game.server.lan_acting_name = Some("attacker".into());
                    let golem = game
                        .res
                        .mob_types()
                        .get("minecraft:iron_golem")
                        .unwrap()
                        .clone();
                    let mut damage_with = |levels: &[(&str, u32)]| {
                        game.mobs.clear();
                        game.mobs
                            .push(Mob::new(21, golem.clone(), Vec3::new(1.0, 64.0, 0.0), 0.0));
                        if levels.iter().any(|(id, _)| *id == "damage_test:force") {
                            let mut boots = ItemStack::new("minecraft:stick", 1);
                            boots.damage = 22;
                            boots.enchant("damage_test:equipment", 1);
                            game.mobs[0]
                                .extra
                                .set_slot_stack(sunrise_items::equipment::FEET, Some(boots));
                        }
                        let mut weapon = ItemStack::new("minecraft:stick", 1);
                        weapon.damage = 11;
                        weapon.data_mut().enchantments = levels
                            .iter()
                            .map(|&(id, level)| (id.into(), level))
                            .collect();
                        game.common.inventory[game.common.selected] = Some(weapon);
                        game.server.attack_ticks = 100;
                        let before = game.mobs[0].health;
                        game.attack_mob(0);
                        before - game.mobs[0].health
                    };
                    let sharpness = ("minecraft:sharpness", 3);
                    let force = ("damage_test:force", 2);
                    assert_eq!(damage_with(&[]), 1.0);
                    let sharpness_damage = damage_with(&[sharpness]);
                    assert_eq!(sharpness_damage, 3.0);
                    // Within one all_of the order is defined: add 2, double, add level 2.
                    cancel.store(true, Ordering::Relaxed);
                    assert_eq!(damage_with(&[force]), 0.0);
                    assert!(hits.lock().unwrap().is_empty());
                    assert_eq!(panics.load(Ordering::Relaxed), 0);
                    cancel.store(false, Ordering::Relaxed);
                    // Vanilla computes damage before asking whether the victim accepts the hit.
                    assert_eq!(
                        std::mem::take(&mut *reads.lock().unwrap()),
                        [(2, 3.0), (2, 6.0)]
                    );
                    let force_damage = damage_with(&[force]);
                    assert_eq!(force_damage, (1.0 + 2.0) * 2.0 + 2.0);
                    assert_eq!(
                        std::mem::take(&mut *reads.lock().unwrap()),
                        [(2, 3.0), (2, 6.0)]
                    );
                    assert_eq!(
                        std::mem::take(&mut *hits.lock().unwrap()),
                        [
                            ("equipped".into(), 21, 21),
                            ("first".into(), 7, 21),
                            ("second".into(), 7, 21)
                        ]
                    );
                    assert_eq!(panics.load(Ordering::Relaxed), 1);
                    assert_eq!(
                        mod_api::__private::take_hook_panics(),
                        [mod_api::__private::HookPanicReport {
                            mod_id: "damage_test",
                            hook: "enchantment_entity_apply",
                        }]
                    );
                    // Distinct enchantments have no defined order: both must contribute.
                    let mixed_damage = damage_with(&[sharpness, force]);
                    assert!(mixed_damage > sharpness_damage && mixed_damage > force_damage);
                    assert_eq!(std::mem::take(&mut *reads.lock().unwrap()).len(), 2);
                    assert_eq!(
                        std::mem::take(&mut *hits.lock().unwrap()),
                        [
                            ("equipped".into(), 21, 21),
                            ("first".into(), 7, 21),
                            ("second".into(), 7, 21)
                        ]
                    );
                    assert_eq!(panics.load(Ordering::Relaxed), 1);
                    assert!(mod_api::__private::take_hook_panics().is_empty());
                    let mut trident = sunrise_sim::projectiles::Projectile::new(
                        sunrise_sim::projectiles::ProjectileKind::Trident,
                        Vec3::ZERO,
                        Some(sunrise_sim::mobs::Who::Player),
                    );
                    let mut weapon = ItemStack::new("minecraft:trident", 1);
                    weapon.damage = 11;
                    weapon.data_mut().enchantments = vec![("damage_test:force".into(), 2)];
                    trident.arrow.as_mut().unwrap().pickup_item = weapon;
                    assert_eq!(
                        game.shot_damage(
                            &mut trident,
                            Some("minecraft:iron_golem"),
                            sunrise_sim::mobs::Who::Mob(21)
                        ),
                        22.0
                    );
                    let mut arrow = sunrise_sim::projectiles::Projectile::new(
                        sunrise_sim::projectiles::ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        },
                        Vec3::ZERO,
                        Some(sunrise_sim::mobs::Who::Player),
                    );
                    arrow.velocity = Vec3::new(3.0, 0.0, 0.0);
                    let mut bow = ItemStack::new("minecraft:bow", 1);
                    bow.damage = 11;
                    bow.data_mut().enchantments = vec![("damage_test:force".into(), 2)];
                    arrow.arrow.as_mut().unwrap().fired_from_weapon = Some(bow);
                    assert_eq!(
                        game.shot_damage(
                            &mut arrow,
                            Some("minecraft:iron_golem"),
                            sunrise_sim::mobs::Who::Mob(21)
                        ),
                        30.0
                    );
                    if let sunrise_sim::projectiles::ProjectileKind::Arrow { critical, .. } =
                        &mut arrow.kind
                    {
                        *critical = true;
                    }
                    arrow.random.0 = sunrise_core::random::LegacyRandom::new(42);
                    assert_eq!(
                        game.shot_damage(
                            &mut arrow,
                            Some("minecraft:iron_golem"),
                            sunrise_sim::mobs::Who::Mob(21)
                        ),
                        39.0
                    );
                    // java.util.Random(42): nextInt(17) = 9, then nextInt() = 234785527.
                    use sunrise_core::random::RandomSource;
                    assert_eq!(arrow.random.0.next_int(), 234785527);
                    assert_eq!(
                        *reads.lock().unwrap(),
                        [(2, 10.0), (2, 20.0), (2, 4.0), (2, 8.0), (2, 4.0), (2, 8.0)]
                    );
                    let mut invalid = definition.clone();
                    invalid["effects"]["minecraft:damage"][0]["effect"]["effects"][1]["factor"] =
                        serde_json::Value::Null;
                    std::fs::write(
                        definitions.join("force.json"),
                        serde_json::to_vec(&invalid).unwrap(),
                    )
                    .unwrap();
                    // The reload decodes every enchantment's effects and refuses the pack.
                    let error = game.reload_datapacks(false, &[]).err().unwrap();
                    assert!(error.to_string().contains("damage_test:force"));
                    assert!(error.to_string().contains("factor"));
                    assert_eq!(
                        game.shot_damage(
                            &mut trident,
                            Some("minecraft:iron_golem"),
                            sunrise_sim::mobs::Who::Mob(21)
                        ),
                        22.0
                    );
                    invalid["effects"]["minecraft:damage"][0]["effect"]["effects"][1]["factor"] =
                        serde_json::json!(3);
                    std::fs::write(
                        definitions.join("force.json"),
                        serde_json::to_vec(&invalid).unwrap(),
                    )
                    .unwrap();
                    let stack = game.reload_datapacks(false, &[]).unwrap();
                    game.server.world_tables.0 = game
                        .stage_world_tables(&stack, false, &resources)
                        .unwrap()
                        .tables;
                    assert_eq!(
                        game.shot_damage(
                            &mut trident,
                            Some("minecraft:iron_golem"),
                            sunrise_sim::mobs::Who::Mob(21)
                        ),
                        32.0
                    );
                    invalid["effects"]["minecraft:post_attack"][0]["effect"]["effects"][2]["fail_decode"] =
                        serde_json::json!(true);
                    std::fs::write(
                        definitions.join("force.json"),
                        serde_json::to_vec(&invalid).unwrap(),
                    )
                    .unwrap();
                    // The reload decodes every enchantment's effects and refuses the pack.
                    let error = game.reload_datapacks(false, &[]).err().unwrap();
                    assert!(error.to_string().contains("codec fixture panic"));
                    assert_eq!(
                        mod_api::__private::take_hook_panics(),
                        [mod_api::__private::HookPanicReport {
                            mod_id: "damage_test",
                            hook: "enchantment_entity_decode",
                        }]
                    );
                    assert_eq!(
                        game.shot_damage(
                            &mut trident,
                            Some("minecraft:iron_golem"),
                            sunrise_sim::mobs::Who::Mob(21)
                        ),
                        32.0
                    );
                    std::fs::remove_dir_all(root).unwrap();
                });
            },
        );
    }
}
