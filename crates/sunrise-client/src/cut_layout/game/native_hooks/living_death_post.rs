#![cfg(test)]
use sunrise_sim::mobs::Damage;
use sunrise_sim::mobs::Who;
#[cfg(test)]
pub(crate) mod tests {
    use crate::game::native_hooks::living_death_post::*;
    use mod_api::{
        Flow, Hooks, LivingDeathEvent, LivingDeathPostEvent, LivingDropsEvent,
        LivingExperienceDropEvent, ModEntry, ServerHost,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_sim::mobs::DamageKind;

    #[derive(Default)]
    pub(crate) struct Post {
        pub(in crate::game) entity: Option<u32>,
        pub(in crate::game) damage_type: Option<String>,
        pub(in crate::game) attacker: Option<u32>,
        pub(in crate::game) kind: Option<String>,
        pub(in crate::game) player: Option<u128>,
        pub(in crate::game) credited_player: Option<u128>,
        pub(in crate::game) message_key: Option<String>,
        pub(in crate::game) drops_before: bool,
        pub(in crate::game) experience_before: bool,
    }

    pub(crate) struct Observer {
        pub heard: Arc<Mutex<Vec<&'static str>>>,
        pub(crate) cancel: Arc<AtomicBool>,
        pub(crate) drops: Arc<AtomicUsize>,
        pub(crate) experience: Arc<AtomicBool>,
        pub(crate) post: Arc<Mutex<Post>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn living_death(&self, _: &mut dyn ServerHost, _: &mut LivingDeathEvent<'_>) -> Flow {
            self.heard.lock().unwrap().push("death");
            if self.cancel.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn living_drops(&self, _: &mut dyn ServerHost, event: &mut LivingDropsEvent<'_>) -> Flow {
            self.heard.lock().unwrap().push("loot");
            self.drops.store(event.drops.len(), Ordering::SeqCst);
            Flow::Continue
        }
        fn living_experience_drop(
            &self,
            _: &mut dyn ServerHost,
            _: &mut LivingExperienceDropEvent,
        ) -> Flow {
            self.heard.lock().unwrap().push("xp");
            self.experience.store(true, Ordering::SeqCst);
            Flow::Continue
        }
        fn living_death_post(&self, _: &mut dyn ServerHost, event: &mut LivingDeathPostEvent<'_>) {
            self.heard.lock().unwrap().push("post");
            let mut post = self.post.lock().unwrap();
            post.entity = Some(event.entity);
            post.damage_type = Some(event.damage_type.to_owned());
            post.attacker = Some(event.attacker);
            post.kind = Some(event.kind.to_owned());
            post.player = event.player;
            post.credited_player = event.credited_player;
            post.message_key = event.message_key.map(str::to_owned);
            post.drops_before = self.drops.load(Ordering::SeqCst) > 0;
            post.experience_before = self.experience.load(Ordering::SeqCst);
        }
    }

    #[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::native_hooks_server::living_death_post::tests::sheep;

    /// One notice per death, at the end: after `living_death` (unless it
    /// cancelled), the drop list and the experience, and before the tick is
    /// done. A player's blow and fall damage carry their damage types; the
    /// hook names the same last hurt `living_death` heard. A cancelled death
    /// is not told.
    #[test]
    #[ignore = "daily"]
    fn living_death_post_fires_once_after_each_finished_death() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let drops = Arc::new(AtomicUsize::new(0));
        let experience = Arc::new(AtomicBool::new(false));
        let post = Arc::new(Mutex::new(Post::default()));
        let mods = vec![ModEntry::new(
            "deaths",
            Box::new(Observer {
                heard: heard.clone(),
                cancel: cancel.clone(),
                drops: drops.clone(),
                experience: experience.clone(),
                post: post.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();

                let player_blow = Damage {
                    kind: DamageKind::PlayerAttack,
                    attacker: Some(Who::Player),
                    from: Some([0.0; 3]),
                    direct: None,
                    attacker_species: None,
                };
                let mut mob = sheep(game, player_blow);
                // The last hurt by a player, so the death rolls experience.
                mob.ai.last_hurt_by_player = 100;
                game.mobs.last_mut().unwrap().ai.last_hurt_by_player = 100;
                game.mob_die(&mob);
                assert_eq!(
                    *heard.lock().unwrap(),
                    ["death", "xp", "loot", "post"],
                    "NeoForge's dropAllDeathLoot: the experience, then the drop list"
                );
                {
                    let post = post.lock().unwrap();
                    assert_eq!(post.entity, Some(mob.id));
                    assert_eq!(post.damage_type.as_deref(), Some("minecraft:player_attack"));
                    assert_eq!(post.attacker, Some(0), "no player entity is credited");
                    assert_eq!(post.kind.as_deref(), Some("minecraft:sheep"));
                    assert_eq!(post.player, None);
                    assert_eq!(post.credited_player, None);
                    assert!(post.drops_before, "the drop list was built before it");
                    assert!(post.experience_before, "the experience rolled before it");
                }
                assert_eq!(
                    game.dropped.len(),
                    drops.load(Ordering::SeqCst),
                    "every item drop it heard of is in the world"
                );
                assert!(!game.dropped.is_empty());

                heard.lock().unwrap().clear();
                *post.lock().unwrap() = Post::default();
                let fall = Damage {
                    kind: DamageKind::Fall,
                    attacker: None,
                    from: None,
                    direct: None,
                    attacker_species: None,
                };
                let mob = sheep(game, fall);
                game.mob_die(&mob);
                assert_eq!(*heard.lock().unwrap(), ["death", "loot", "post"]);
                assert_eq!(
                    post.lock().unwrap().damage_type.as_deref(),
                    Some("minecraft:fall")
                );

                heard.lock().unwrap().clear();
                *post.lock().unwrap() = Post::default();
                let mut mob = sheep(
                    game,
                    Damage {
                        kind: DamageKind::Generic,
                        attacker: None,
                        from: None,
                        direct: None,
                        attacker_species: None,
                    },
                );
                mob.health = 20.0;
                game.mobs.last_mut().unwrap().health = 20.0;
                // `/kill` replaces the earlier damage source with generic_kill.
                game.kill_target(sunrise_gameplay_common::game::commands_common::Target::Mob(
                    mob.id,
                ));
                assert_eq!(*heard.lock().unwrap(), ["death", "loot", "post"]);
                assert_eq!(
                    post.lock().unwrap().damage_type.as_deref(),
                    Some("minecraft:generic_kill")
                );

                // A death `living_death` kept: no drop, no notice.
                heard.lock().unwrap().clear();
                cancel.store(true, Ordering::SeqCst);
                let mob = sheep(
                    game,
                    Damage {
                        kind: DamageKind::PlayerAttack,
                        attacker: Some(Who::Player),
                        from: Some([0.0; 3]),
                        direct: None,
                        attacker_species: None,
                    },
                );
                game.mob_die(&mob);
                assert_eq!(*heard.lock().unwrap(), ["death"]);
                assert_eq!(
                    game.mobs.last().unwrap().health,
                    0.0,
                    "cancellation alone does not heal the body"
                );
            });
        });
    }

    /// A server player's death is told at `ServerPlayer.die`'s tail too: after
    /// its death message, the inventory drops and the experience.
    #[test]
    #[ignore = "daily"]
    fn living_death_post_hears_a_player_death_after_the_message_and_loot() {
        use sunrise_gameplay::game::native_player_death::tests::add_guest;
        let heard = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let experience = Arc::new(AtomicBool::new(false));
        let post = Arc::new(Mutex::new(Post::default()));
        let mods = vec![ModEntry::new(
            "deaths",
            Box::new(Observer {
                heard: heard.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                drops: drops.clone(),
                experience: experience.clone(),
                post: post.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                add_guest(game);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.health = 0.0;
                        guest.inventory[0] =
                            Some(sunrise_items::items::ItemStack::new("minecraft:stone", 1));
                    });
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                let (id, uuid) = (guest.entity_id as u32, guest.uuid);

                game.guest_die(0, 1);

                assert_eq!(
                    *heard.lock().unwrap(),
                    ["death", "xp", "loot", "post"],
                    "the message, the experience and then the inventory drop ran before it"
                );
                let post = post.lock().unwrap();
                assert_eq!(post.entity, Some(id));
                assert_eq!(post.kind.as_deref(), Some("minecraft:player"));
                assert_eq!(post.player, Some(uuid));
                assert_eq!(post.message_key, None);
                assert_eq!(post.damage_type.as_deref(), Some("minecraft:generic"));
                assert_eq!(post.attacker, Some(0), "no attacker was recorded");
                assert!(post.experience_before, "the experience ran before it");
                assert!(
                    game.dropped
                        .iter()
                        .any(|d| &*d.stack.item == "minecraft:stone"),
                    "the inventory was dropped before it"
                );
            });
        });
    }
}
