#![cfg(test)]
#[cfg(test)]
mod tests {
    use mod_api::{
        ArmorHurtEvent, DamageItemEvent, Flow, HitHost, Hooks, ItemPart, LivingIncomingDamageEvent,
        Registrar, RegistrationContext, ServerHost,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_core::{equipment_slot::EquipmentSlot, geometry::Vec3};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::motion::PlayerMotion;

    const ANVIL: &str = "minecraft:falling_anvil";

    /// What the probe answers and what it heard of a hit.
    #[derive(Default)]
    struct Log {
        refuse_hit: bool,
        cancel_wear: bool,
        new_damage: Option<f32>,
        incoming: u32,
        /// `armor_hurt`: each call's slots and vanilla damage.
        armor: Vec<Vec<(EquipmentSlot, f32)>>,
        /// `damage_item` of the iron helmet: the points, and whether its host names the acting player.
        worn: Vec<(i32, bool)>,
    }
    #[derive(Clone)]
    struct Probe(Arc<Mutex<Log>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.clone());
        }
        fn living_incoming_damage(
            &self,
            _: &mut dyn HitHost,
            _: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            let mut log = self.0.lock().unwrap();
            log.incoming += 1;
            if log.refuse_hit {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn armor_hurt(&self, _: &mut dyn ServerHost, event: &mut ArmorHurtEvent<'_>) -> Flow {
            let mut log = self.0.lock().unwrap();
            let call = event.entries.iter().map(|e| (e.slot, e.damage)).collect();
            log.armor.push(call);
            if let Some(new_damage) = log.new_damage {
                for entry in event.entries.iter_mut() {
                    entry.new_damage = new_damage;
                }
            }
            if log.cancel_wear {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    #[mod_api::sunrise_part]
    impl ItemPart for Probe {
        fn damage_item(&self, host: &mut dyn ServerHost, event: &mut DamageItemEvent<'_>) -> Flow {
            let acting = host.players().acting_player().is_some();
            self.0.lock().unwrap().worn.push((event.amount, acting));
            Flow::Continue
        }
    }

    /// Runs `test` as a helmeted server player, with the probe's log.
    fn with_player(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Mutex<Log>)) {
        let log = Arc::new(Mutex::new(Log::default()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(log.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                game.server.lan_acting_uuid = Some(123);
                game.server.lan_acting_entity = Some(20);
                game.server.lan_acting_name = Some("Steve".into());
                game.server.lan_acting_permission = Some(0);
                test(game, &log);
            })
        });
    }
    /// A new player in `helmet`, and a log with no answers: a hit of 8 wears 2 points.
    fn start(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        log: &Mutex<Log>,
        helmet: Option<ItemStack>,
    ) {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.armor[0] = helmet;
        game.apply_equipment();
        game.refresh_armor_protection();
        *log.lock().unwrap() = Log::default();
    }
    fn hit(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        game.falling_hurt_player(
            241,
            Vec3::ZERO,
            sunrise_sim::falling_block::bounds(Vec3::ZERO),
            8.0,
            ANVIL,
        );
    }
    fn health(game: &sunrise_gameplay::game::ServerGame<'_>) -> f32 {
        game.player.as_ref().unwrap().health.health()
    }
    fn iron_helmet() -> ItemStack {
        ItemStack::new("minecraft:iron_helmet", 1)
    }

    #[test]
    #[ignore = "daily"]
    fn a_canceled_wear_leaves_the_helmet_and_its_share_of_the_hit() {
        with_player(|game, log| {
            start(game, log, Some(iron_helmet()));
            log.lock().unwrap().cancel_wear = true;
            hit(game);
            assert_eq!(game.armor[0], Some(iron_helmet()));
            {
                let log = log.lock().unwrap();
                assert_eq!(
                    log.armor,
                    [
                        vec![(EquipmentSlot::Head, 2.0)],
                        vec![(EquipmentSlot::Head, 1.5)]
                    ]
                );
                assert!(log.worn.is_empty());
                assert_eq!(log.incoming, 1);
            }
            // 8 less the helmet's quarter is 6; 2 armor points leave 98.4% of it.
            assert!((health(game) - 14.096).abs() < 0.001, "{}", health(game));

            // Both equipment operations already ran; draining events cannot repeat them.
            let events = std::mem::take(&mut game.player.as_mut().unwrap().events);
            game.wear_armor(&events);
            assert_eq!(log.lock().unwrap().armor.len(), 2);
            assert_eq!(game.armor[0], Some(iron_helmet()));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_answered_wear_runs_on_the_real_host_before_the_cooldown_refuses() {
        with_player(|game, log| {
            start(game, log, Some(iron_helmet()));
            log.lock().unwrap().new_damage = Some(7.0);
            hit(game);
            assert_eq!(game.armor[0].as_ref().unwrap().damage, 14);
            assert_eq!(log.lock().unwrap().worn, [(7, true), (7, true)]);
            let after_first = health(game);

            // The cooldown takes the same hit whole, after `hurtHelmet` has worn the piece.
            log.lock().unwrap().new_damage = None;
            hit(game);
            assert_eq!(game.armor[0].as_ref().unwrap().damage, 16);
            assert_eq!(health(game), after_first);
            assert_eq!(log.lock().unwrap().incoming, 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_helmet_the_hit_breaks_stops_protecting_in_that_hit() {
        with_player(|game, log| {
            let mut helmet = iron_helmet();
            helmet.damage = game.items().get(&helmet.item).unwrap().max_damage - 1;
            helmet.enchant("minecraft:protection", 4);
            start(game, log, Some(helmet));
            let status = &game.player.as_ref().unwrap().status;
            assert_eq!(status.protection(ANVIL), 4.0);
            hit(game);
            assert_eq!(game.armor[0], None);
            // The quarter stays: `hurtServer` takes it whatever the helmet became.
            assert_eq!(health(game), 14.0);
            assert_eq!(log.lock().unwrap().incoming, 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_helmet_or_a_refused_hit_fires_no_helmet_wear() {
        with_player(|game, log| {
            start(game, log, None);
            hit(game);
            assert!(log.lock().unwrap().armor.is_empty());
            assert_eq!(health(game), 12.0);

            start(game, log, Some(iron_helmet()));
            log.lock().unwrap().refuse_hit = true;
            hit(game);
            assert!(log.lock().unwrap().armor.is_empty());
            assert_eq!(game.armor[0], Some(iron_helmet()));
            assert_eq!(health(game), 20.0);
        });
    }
}

#[cfg(test)]
#[path = "falling_damage/helmet_tests.rs"]
pub(crate) mod helmet_tests;
