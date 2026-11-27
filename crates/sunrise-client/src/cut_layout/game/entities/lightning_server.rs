#![cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mod_api::{self, ServerHost};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::collect_entities_in;
    use sunrise_gameplay_common::game::lightning_common::Bolt;
    use sunrise_sim::mobs::Mob;

    #[test]
    #[ignore = "daily"]
    fn conversion_post_reads_original_and_lightning_outcome() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{
            EntityJoinLevelEvent, EquipmentSlot, Flow, Hooks, LivingConversionPostEvent,
            LivingConversionPreEvent,
        };
        use mod_platform::__private::install;
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;

        #[derive(Debug)]
        struct SeenConversion {
            old: Option<String>,
            outcome: Option<String>,
            weapon: Option<String>,
            mushrooms: usize,
        }
        struct Observer {
            seen: Arc<Mutex<Vec<SeenConversion>>>,
            pre: Arc<AtomicUsize>,
            cancel_cow: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn living_conversion_pre(
                &self,
                _: &mut dyn ServerHost,
                _: &mut LivingConversionPreEvent<'_>,
            ) -> Flow {
                self.pre.fetch_add(1, Ordering::SeqCst);
                Flow::Continue
            }
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:cow" && self.cancel_cow.load(Ordering::SeqCst) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn living_conversion_post(
                &self,
                host: &mut dyn ServerHost,
                event: &mut LivingConversionPostEvent<'_>,
            ) {
                let old = host
                    .entity(event.entity)
                    .and_then(|entity| entity.kind(host)?.id(host))
                    .map(|name| name.as_str().to_owned());
                let outcome = host.entity(event.converted);
                let weapon = outcome
                    .and_then(|entity| host.living(entity))
                    .and_then(|living| {
                        living.with_equipment(host, EquipmentSlot::MainHand, |stack| {
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .to_string()
                        })
                    });
                let mushrooms = host
                    .level("minecraft:overworld")
                    .and_then(|level| {
                        collect_entities_in(host, level, [-10.0, 60.0, -10.0], [20.0, 70.0, 20.0])
                            .ok()
                    })
                    .unwrap()
                    .into_iter()
                    .filter_map(|entity| host.item_entity(entity))
                    .filter(|item| {
                        item.with_stack(host, |stack| {
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .ends_with("mushroom")
                        }) == Some(true)
                    })
                    .count();
                self.seen.lock().unwrap().push(SeenConversion {
                    old,
                    outcome: outcome
                        .and_then(|entity| entity.kind(host)?.id(host))
                        .map(|name| name.as_str().to_owned()),
                    weapon,
                    mushrooms,
                });
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let pre = Arc::new(AtomicUsize::new(0));
        let cancel_cow = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = Arc::from([mod_api::ModEntry::new(
            "lightning-observer",
            Box::new(Observer {
                seen: seen.clone(),
                pre: pre.clone(),
                cancel_cow: cancel_cow.clone(),
            }),
        )]);
        install(mods);
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            fixture_resident_column(&mut game.world, (6, 6));
            let piglin_type = game
                .res
                .mob_types()
                .get("minecraft:piglin")
                .unwrap()
                .clone();
            game.server.next_entity += 1;
            let mut old = Mob::new(
                game.server.next_entity,
                piglin_type,
                Vec3::new(100.0, 64.0, 100.0),
                0.0,
            );
            old.removed = true; // The tick loop has already taken this mob out.
            let converted = game
                .convert_to(
                    &old,
                    "minecraft:zombified_piglin",
                    sunrise_sim::mobs::conversion::Params::KEEP_ALL,
                    false,
                    |_, _| {},
                )
                .unwrap();
            assert!(game.entity(old.id).is_none());
            assert!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == converted)
                    .unwrap()
                    .has_effect("minecraft:nausea")
            );

            let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.server.next_entity += 1;
            let pig_id = game.server.next_entity;
            game.mobs.push(Mob::new(
                pig_id,
                pig_type.clone(),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            ));
            game.ui.difficulty = 2;
            let mut bolt = Bolt::struck_at(0, [0.0, 64.0, 0.0]);
            game.thunder_hit(&mut bolt);
            assert!(game.entity(pig_id).is_none());
            let mushroom_type = game
                .res
                .mob_types()
                .get("minecraft:mooshroom")
                .unwrap()
                .clone();
            for (x, player) in [(5, true), (10, false)] {
                game.server.next_entity += 1;
                let id = game.server.next_entity;
                game.mobs.push(Mob::new(
                    id,
                    mushroom_type.clone(),
                    Vec3::new(f64::from(x) + 0.5, 64.0, 0.5),
                    0.0,
                ));
                if player {
                    game.common.inventory[game.common.selected] =
                        Some(sunrise_items::items::ItemStack::new("minecraft:shears", 1));
                    assert!(
                        game.interact_mob(Some(sunrise_gameplay_common::game::InteractTarget {
                            id: id as i32,
                            location: [0.0; 3]
                        }))
                        .unwrap()
                    );
                } else {
                    let shears = sunrise_client::items::ItemStack::new("minecraft:shears", 1);
                    assert!(game.shear_entity((x, 64, 0), &shears).unwrap());
                }
            }
            assert_eq!(pre.load(Ordering::SeqCst), 4);
            {
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 4);
                assert_eq!(seen[2].mushrooms, 5);
                assert_eq!(seen[3].mushrooms, 10);
            }
            cancel_cow.store(true, Ordering::SeqCst);
            game.server.next_entity += 1;
            let old_id = game.server.next_entity;
            let old = Mob::new(old_id, mushroom_type, Vec3::new(15.0, 64.0, 0.0), 0.0);
            game.mobs.push(old.clone());
            game.server.next_entity += 1;
            let rider_id = game.server.next_entity;
            let mut rider = Mob::new(rider_id, pig_type, Vec3::new(15.0, 64.0, 0.0), 0.0);
            rider.extra.mount = Some(old_id);
            game.mobs.push(rider);
            assert!(
                game.convert_to(
                    &old,
                    "minecraft:cow",
                    sunrise_sim::mobs::conversion::Params::KEEP_ALL,
                    false,
                    |_, _| {}
                )
                .is_none()
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == rider_id)
                    .unwrap()
                    .extra
                    .mount,
                Some(old_id)
            );
            assert!(!game.mobs.iter().find(|m| m.id == old_id).unwrap().removed);
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 4);
            assert_eq!(seen[0].old.as_deref(), Some("minecraft:piglin"));
            assert_eq!(
                seen[0].outcome.as_deref(),
                Some("minecraft:zombified_piglin")
            );
            assert_eq!(seen[1].old.as_deref(), Some("minecraft:pig"));
            assert_eq!(
                seen[1].outcome.as_deref(),
                Some("minecraft:zombified_piglin")
            );
            assert!(matches!(
                seen[1].weapon.as_deref(),
                Some("minecraft:golden_sword" | "minecraft:golden_spear")
            ));
        });
    }
}
