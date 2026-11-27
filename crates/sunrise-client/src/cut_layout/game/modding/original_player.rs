#![cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
use sunrise_items::items::ItemStack;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Attachments;
    use mod_api::{
        AttachmentDescriptor, AttachmentKey, Holder, Hooks, HostTransaction, PlayerCloneEvent,
        PlayerContainerCloseEvent, ServerHost,
    };
    use sunrise_core::nbt::{Compound, JavaString, Tag};
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_mod_runtime::attachments::AttachmentType;

    const VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "unsaved",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };
    const DEFAULTED: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "defaulted",
        default: || 41,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };
    const RETAINED: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "retained",
        default: || 0,
        codec: Some(mod_api::AttachmentCodec {
            encode: |value| Ok([("value".into(), Tag::Int(*value))].into()),
            decode: |value| match value.get(&JavaString::from("value")) {
                Some(Tag::Int(value)) => Ok(*value),
                _ => Err(mod_api::AttachmentError::Decode),
            },
        }),
        copy_on_death: true,
        sync_codec: None,
    };
    const SAVED: AttachmentType<i32> = AttachmentType {
        name: "saved",
        copy_on_death: false,
        ..RETAINED
    };

    const MEMORY_NOW: AttachmentType<i32> = AttachmentType {
        name: "memory_now",
        ..VALUE
    };

    fn issued(raw: &AttachmentType<i32>) -> AttachmentKey<i32> {
        mod_api::testing::attachment_key("clone", raw.name)
    }

    fn describe(id: &'static str, raw: &AttachmentType<i32>) -> AttachmentDescriptor<i32> {
        let mut descriptor = AttachmentDescriptor::new(id, raw.default);
        if let Some(codec) = raw.codec {
            descriptor = descriptor.serialize(codec);
        }
        if raw.copy_on_death {
            descriptor = descriptor.copy_on_death();
        }
        descriptor
    }

    struct Listener {
        death: bool,
        keep: bool,
        heard: Arc<Mutex<usize>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_attachment_type(describe("clone:unsaved", &VALUE))
                .unwrap();
            r.register_attachment_type(describe("clone:defaulted", &DEFAULTED))
                .unwrap();
            r.register_attachment_type(describe("clone:retained", &RETAINED))
                .unwrap();
            r.register_attachment_type(describe("clone:saved", &SAVED))
                .unwrap();
            r.register_attachment_type(describe("clone:memory_now", &MEMORY_NOW))
                .unwrap();
        }

        fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
            assert_eq!(event.was_death, self.death);
            assert_eq!(event.original.uuid(), 12);
            assert_eq!(event.original.position(), [7.0, 64.0, -9.0]);
            let dimension = if self.death { Dim::Nether } else { Dim::End };
            assert_eq!(
                event.original.dimension().unwrap().to_string(),
                dimension.id()
            );
            let original_level = event.original.level().unwrap();
            assert_eq!(original_level.dimension_id(), dimension.id());
            assert_eq!(event.original.health(), if self.death { 0.0 } else { 13.0 });
            assert_eq!(event.original.experience().level, 7);
            let effects = event.original.effects();
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].duration, 200);
            assert_eq!(effects[0].amplifier, 1);

            event.original.with_inventory(|inv| {
                if self.death && !self.keep {
                    assert!(inv.get(2).is_none());
                    assert!(inv.get(36).is_none());
                    assert!(inv.get(40).is_none());
                } else {
                    assert_eq!(inv.get(2).unwrap().count(), 4);
                    assert_eq!(inv.get(36).unwrap().count(), 1);
                    assert_eq!(inv.get(40).unwrap().count(), 2);
                }
                assert!(inv.get(99).is_none());
            });

            let outside = JavaString::from("outside");
            let persisted = JavaString::from("PlayerPersisted");
            let persisted_value = Tag::compound([("inside", Tag::Int(2))]);
            let original_data = event.original.persistent_data().unwrap();
            assert_eq!(original_data.get(&outside), Some(&Tag::Int(1)));
            assert_eq!(original_data.get(&persisted), Some(&persisted_value));
            let replacement = host.player(event.player).unwrap();
            // Cross-dimension clone: replacement is already in its destination level while
            // the retained original preserves its source level.
            let replacement_level = replacement.level(host).unwrap();
            assert_eq!(replacement_level.dimension_id(), Dim::Overworld.id());
            assert_eq!(
                replacement.dimension(host).unwrap().to_string(),
                Dim::Overworld.id()
            );
            assert_ne!(original_level, replacement_level);
            assert_eq!(replacement.entity().position(host), Some([0.0; 3]));
            let _ = replacement_level
                .visit_online_players(host, &mut |found| {
                    assert_ne!(
                        found, replacement,
                        "replacement is not resident during clone"
                    );
                    core::ops::ControlFlow::Continue(())
                })
                .unwrap();
            let replacement_data = replacement.persistent_data(host).unwrap();
            assert_eq!(replacement_data.get(&outside), None);
            assert_eq!(replacement_data.get(&persisted), Some(&persisted_value));
            let mut replacement_count = 0;
            assert_eq!(
                mod_api::LivingRef::new(replacement.entity()).visit_effects(host, &mut |_| {
                    replacement_count += 1;
                    std::ops::ControlFlow::Continue(())
                }),
                Ok(std::ops::ControlFlow::Continue(()))
            );
            assert_eq!(replacement_count, usize::from(!self.death));

            let (_, stale) = mod_api::testing::registered_attachment_fixture(
                "clone",
                AttachmentDescriptor::new("clone:unsaved", || 0i32),
            );
            assert_eq!(
                event.original.attachment(&stale, Clone::clone),
                Err(mod_api::AttachmentError::StaleKey)
            );

            // A declared but absent key yields its default without writing either holder.
            assert_eq!(
                event.original.attachment(&issued(&DEFAULTED), Clone::clone),
                Ok(41)
            );
            assert_eq!(
                event.original.has_attachment(&issued(&DEFAULTED)),
                Ok(false)
            );
            assert_eq!(
                event
                    .original
                    .existing_attachment(&issued(&DEFAULTED), |value| value.cloned()),
                Ok(None)
            );
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&issued(&DEFAULTED), |value| value.copied()),
                Ok(None)
            );
            assert_eq!(
                issued(&DEFAULTED).has(host, Holder::Player(event.player)),
                Ok(false)
            );

            // Saved codec-backed entry decodes once on existing_attachment and caches it.
            assert_eq!(event.original.has_attachment(&issued(&SAVED)), Ok(true));
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&issued(&SAVED), |value| value.copied()),
                Ok(None)
            );
            assert_eq!(
                event
                    .original
                    .existing_attachment(&issued(&SAVED), |value| value.cloned()),
                Ok(Some(31))
            );
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&issued(&SAVED), |value| value.copied()),
                Ok(Some(31))
            );
            assert_eq!(
                issued(&SAVED).has(host, Holder::Player(event.player)),
                Ok(!self.death)
            );
            // Copied serializable entries start undecoded (independent of original's decoded cache).
            assert_eq!(
                issued(&SAVED)
                    .with_decoded(host, Holder::Player(event.player), |value| value.copied()),
                Ok(None)
            );
            assert_eq!(
                issued(&SAVED).get(host, Holder::Player(event.player), Clone::clone),
                Ok(if self.death { 0 } else { 31 })
            );

            // Its old serialized envelope still exists on the retained original, but
            // the installed descriptor no longer allows either respawn route to copy it.
            let memory_now = issued(&MEMORY_NOW);
            assert_eq!(event.original.has_attachment(&memory_now), Ok(true));
            assert_eq!(
                memory_now.has(host, Holder::Player(event.player)),
                Ok(false)
            );
            assert_eq!(
                memory_now.get_existing(host, Holder::Player(event.player), |value| value.copied()),
                Ok(None)
            );

            // Codec-less attachment exists only on original until explicitly written to replacement,
            // on both death and End credits return (NeoForge AttachmentInternals.copyEntityAttachments).
            assert_eq!(
                event.original.attachment(&issued(&VALUE), Clone::clone),
                Ok(73)
            );
            assert_eq!(
                issued(&VALUE).has(host, Holder::Player(event.player)),
                Ok(false)
            );
            assert_eq!(
                event.original.attachment(&issued(&RETAINED), Clone::clone),
                Ok(17)
            );
            assert_eq!(
                issued(&RETAINED)
                    .with_decoded(host, Holder::Player(event.player), |value| value.copied()),
                Ok(None)
            );
            assert_eq!(
                issued(&RETAINED).get(host, Holder::Player(event.player), Clone::clone),
                Ok(17)
            );

            // Writes and removals on original affect only original, leaving replacement untouched.
            assert_eq!(event.original.set_attachment(&issued(&VALUE), 100), Ok(()));
            assert_eq!(
                event
                    .original
                    .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                Ok(Some(100))
            );
            assert_eq!(
                issued(&VALUE).has(host, Holder::Player(event.player)),
                Ok(false)
            );
            assert_eq!(
                event.original.remove_attachment(&issued(&RETAINED)),
                Ok(Some(17))
            );
            assert_eq!(event.original.has_attachment(&issued(&RETAINED)), Ok(false));
            assert_eq!(
                issued(&RETAINED).get(host, Holder::Player(event.player), Clone::clone),
                Ok(17)
            );

            // Mutating the replacement leaves original untouched.
            issued(&VALUE)
                .set(host, Holder::Player(event.player), 99)
                .unwrap();
            assert_eq!(
                event
                    .original
                    .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                Ok(Some(100))
            );
            *self.heard.lock().unwrap() += 1;
        }
    }

    struct Observer {
        heard: Arc<Mutex<usize>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
            // The first listener read the default without storing it. Its explicit mutations
            // are visible to the second listener only on the holder it wrote to.
            assert_eq!(
                event.original.has_attachment(&issued(&DEFAULTED)),
                Ok(false)
            );
            assert_eq!(
                issued(&DEFAULTED).has(host, Holder::Player(event.player)),
                Ok(false)
            );
            assert_eq!(
                event
                    .original
                    .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                Ok(Some(100))
            );
            assert_eq!(
                issued(&VALUE).get(host, Holder::Player(event.player), Clone::clone),
                Ok(99)
            );
            assert_eq!(event.original.has_attachment(&issued(&RETAINED)), Ok(false));
            assert_eq!(
                issued(&RETAINED).get(host, Holder::Player(event.player), Clone::clone),
                Ok(17)
            );
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&issued(&SAVED), |value| value.copied()),
                Ok(Some(31))
            );
            *self.heard.lock().unwrap() += 1;
        }
    }

    fn clone_case(death: bool, keep: bool) {
        let heard = Arc::new(Mutex::new(0));
        let observer_heard = Arc::new(Mutex::new(0));
        let mods = vec![
            mod_api::ModEntry::new(
                "clone",
                Box::new(Listener {
                    death,
                    keep,
                    heard: heard.clone(),
                }),
            ),
            mod_api::ModEntry::new(
                "observer",
                Box::new(Observer {
                    heard: observer_heard.clone(),
                }),
            ),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let dimension = if death { Dim::Nether } else { Dim::End };
                game.enter_level(dimension).unwrap();
                game.ui.rules.set(
                    "keep_inventory",
                    sunrise_game_data::gamerules::Value::Bool(keep),
                );
                let host = game.server.lan.as_mut().unwrap();
                host.hub.guests.update(0, |guest| {
                    guest.dimension = dimension.id().to_owned();
                    guest.position = [7.0, 64.0, -9.0];
                    guest.spawn = Some([3.5, 64.0, 5.5]);
                    guest.dead = death;
                    guest.health = if death { 0.0 } else { 13.0 };
                    guest.xp_level = 7;
                    guest
                        .effects
                        .list
                        .push(sunrise_level::effects::EffectInstance::from_def(
                            sunrise_game_data::potions::effect_by_index(
                                mod_api::__private::effect_index(
                                    mod_platform::__private::part_tests::effect_key(
                                        "minecraft:speed",
                                    ),
                                )
                                .unwrap(),
                            )
                            .unwrap(),
                            200,
                            1,
                        ));
                    // Respawn starts after death drops; keep-inventory leaves these intact.
                    if !death || keep {
                        guest.inventory[2] = Some(ItemStack::new("minecraft:diamond", 4));
                        guest.armor[3] = Some(ItemStack::new("minecraft:diamond_boots", 1));
                        guest.offhand = Some(ItemStack::new("minecraft:apple", 2));
                    }
                    let mut neoforge_data = Compound::new();
                    neoforge_data.insert("outside".into(), Tag::Int(1));
                    neoforge_data.insert(
                        "PlayerPersisted".into(),
                        Tag::compound([("inside", Tag::Int(2))]),
                    );
                    guest.saved = Some(Tag::compound([(
                        "NeoForgeData",
                        Tag::Compound(neoforge_data),
                    )]));
                });
                let attachments = host.hub.guests.attachments_mut(12).unwrap();
                let envelope = |value, copied| {
                    let mut fields: Compound =
                        [("value".into(), Tag::compound([("value", Tag::Int(value))]))].into();
                    if copied {
                        fields.insert("copy_on_death".into(), Tag::Byte(1));
                    }
                    Tag::Compound(fields)
                };
                let opaque = envelope(59, true);
                let saved = Attachments::from_nbt(
                    [
                        (
                            "clone".into(),
                            Tag::compound([
                                ("retained", envelope(17, false)),
                                ("saved", envelope(31, true)),
                                ("memory_now", envelope(44, true)),
                            ]),
                        ),
                        (
                            "removed_package".into(),
                            Tag::compound([("opaque", opaque.clone())]),
                        ),
                    ]
                    .into(),
                );
                attachments.replace_contents(saved);
                VALUE.set(attachments, Holder::Level, 73).unwrap();
                if death {
                    game.guest_respawn(12);
                } else {
                    host.hub.update_guest(12, |guest| guest.won_game = true);
                    game.guest_respawn(12);
                }
                let saved = Tag::Compound(
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .attachments_mut(12)
                        .unwrap()
                        .try_to_nbt()
                        .unwrap(),
                );
                assert_eq!(
                    saved.get("removed_package").unwrap().get("opaque"),
                    Some(&opaque)
                );
                assert_eq!(*heard.lock().unwrap(), 1);
                assert_eq!(*observer_heard.lock().unwrap(), 1);
                assert!(game.player_state.cloning_players.is_empty());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn death_clone_retains_original_and_expires_after_dispatch() {
        use sunrise_gameplay::game::native_entity_refs::tests::{
            run_fresh_cases, run_with_game_stack,
        };
        const FLAG: &str = "SUNRISE_DEATH_CLONE_CASE";
        if run_fresh_cases(
            "cut_layout::game::server_side::original_player::tests::death_clone_retains_original_and_expires_after_dispatch",
            FLAG,
            &["drop", "keep"],
        ) {
            return;
        }
        let keep = std::env::var(FLAG).unwrap() == "keep";
        run_with_game_stack(move || clone_case(true, keep));
    }

    #[test]
    #[ignore = "daily"]
    fn end_return_clone_retains_end_state_and_expires_after_dispatch() {
        use sunrise_gameplay::game::native_entity_refs::tests::{
            run_fresh_cases, run_with_game_stack,
        };
        if run_fresh_cases(
            "cut_layout::game::server_side::original_player::tests::end_return_clone_retains_end_state_and_expires_after_dispatch",
            "SUNRISE_END_CLONE_CASE",
            &["end"],
        ) {
            return;
        }
        run_with_game_stack(|| clone_case(false, true));
    }

    #[test]
    #[ignore = "daily"]
    fn unsubscribed_clone_does_not_capture() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_guests(|game| {
                assert!(
                    game.retain_original_player(12, Attachments::default())
                        .is_none()
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn clone_removes_menus_and_retires_old_attachment_owner_before_copy() {
        use mod_api::{AttachmentHolder, AttachmentRowError, AttachmentWriteToken};
        use sunrise_mod_runtime::attachments::index::AttachmentIndex;
        type OldIndex = Arc<Mutex<AttachmentIndex<AttachmentHolder>>>;

        struct CloseAndClone {
            death: bool,
            closed: Arc<Mutex<usize>>,
            cloned: Arc<Mutex<usize>>,
            source: Arc<Mutex<Option<(OldIndex, AttachmentWriteToken)>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for CloseAndClone {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type(describe("clone:unsaved", &VALUE))
                    .unwrap();
                r.register_attachment_type(describe("clone:saved", &SAVED))
                    .unwrap();
                r.register_attachment_type(describe("clone:retained", &RETAINED))
                    .unwrap();
            }

            fn player_container_close(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                issued(&VALUE)
                    .set(host, Holder::Player(event.player.uuid()), 55)
                    .unwrap();
                issued(&SAVED)
                    .set(host, Holder::Player(event.player.uuid()), 66)
                    .unwrap();
                issued(&RETAINED)
                    .set(host, Holder::Player(event.player.uuid()), 77)
                    .unwrap();
                *self.closed.lock().unwrap() += 1;
            }

            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                assert_eq!(event.was_death, self.death);
                assert_eq!(*self.closed.lock().unwrap(), 1);
                let expected = if self.death { 0 } else { 10 };
                let count = event
                    .original
                    .with_inventory(|inv| inv.iter().map(|(_, stack)| stack.count()).sum::<u32>());
                assert_eq!(count, expected);
                let replacement = host.player(event.player).unwrap();
                assert_eq!(
                    replacement.with_inventory(host, |inv| inv
                        .iter()
                        .map(|(_, s)| s.count())
                        .sum::<u32>()),
                    Some(expected)
                );

                assert_eq!(
                    event
                        .original
                        .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                    Ok(Some(55))
                );
                assert_eq!(
                    event
                        .original
                        .existing_attachment(&issued(&SAVED), |value| value.cloned()),
                    Ok(Some(66))
                );
                assert_eq!(
                    event
                        .original
                        .existing_attachment(&issued(&RETAINED), |value| value.cloned()),
                    Ok(Some(77))
                );
                assert_eq!(
                    issued(&VALUE).has(host, Holder::Player(event.player)),
                    Ok(false)
                );
                assert_eq!(
                    issued(&SAVED).has(host, Holder::Player(event.player)),
                    Ok(!self.death)
                );
                assert_eq!(
                    issued(&RETAINED).get(host, Holder::Player(event.player), Clone::clone),
                    Ok(77)
                );
                issued(&VALUE)
                    .set(host, Holder::Player(event.player), 999)
                    .unwrap();
                let source = self.source.lock().unwrap();
                let (source_index, old_token) = source.as_ref().unwrap();
                assert_eq!(
                    host.attachment_validate_write(old_token.clone()),
                    Err(AttachmentRowError::StaleHolder)
                );
                assert!(
                    source_index
                        .lock()
                        .unwrap()
                        .snapshot_named("clone", "retained")
                        .is_empty()
                );
                let key = issued(&VALUE);
                let batch = mod_api::AttachmentQueries::capture(host, &key).unwrap();
                assert!(
                    batch
                        .query(&key)
                        .unwrap()
                        .iter()
                        .try_next()
                        .unwrap()
                        .is_none(),
                    "clone writes are unpublished until placement"
                );
                *self.cloned.lock().unwrap() += 1;
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
            for death in [true, false] {
                let closed = Arc::new(Mutex::new(0));
                let cloned = Arc::new(Mutex::new(0));
                let source = Arc::new(Mutex::new(None));
                let mods = vec![mod_api::ModEntry::new(
                    "clone",
                    Box::new(CloseAndClone {
                        death,
                        closed: closed.clone(),
                        cloned: cloned.clone(),
                        source: source.clone(),
                    }),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_guests(|game| {
                        game.options.dedicated = Some(Vec::new());
                        let dim = if death { Dim::Overworld } else { Dim::End };
                        game.enter_level(dim).unwrap();
                        game.server.streaming.insert_remote(
                            &mut game.common.world,
                            sunrise_level::chunk::Chunk::empty(0, 0),
                        );
                        if !death {
                            assert!(
                                game.set_state_named([0, 64, 0], "minecraft:end_portal", &[])
                                    .unwrap()
                            );
                        }
                        let host = game.server.lan.as_mut().unwrap();
                        host.hub.guests.update(0, |guest| {
                            guest.dimension = dim.id().to_owned();
                            guest.position = [0.5, 64.0, 0.5];
                            guest.spawn = Some([3.5, 64.0, 5.5]);
                            guest.craft_grid[0] = Some(ItemStack::new("minecraft:oak_planks", 5));
                            guest.dead = death;
                            guest.health = if death { 0.0 } else { 20.0 };
                        });
                        let attachments = host.hub.guests.attachments_mut(12).unwrap();
                        VALUE.set(attachments, Holder::Level, 19).unwrap();
                        RETAINED.set(attachments, Holder::Level, 17).unwrap();
                        game.bind_player_attachments(12, false);
                        let index = game.attachment_indexes[&dim].clone();
                        let rows = index.lock().unwrap().snapshot_named("clone", "retained");
                        let batch: mod_api::AttachmentBatch =
                            sunrise_mod_runtime::attachments::AttachmentBatch::new(rows).into();
                        let key = issued(&RETAINED);
                        let query = batch.query(&key).unwrap();
                        let old = query.iter().try_next().unwrap().unwrap();
                        let token = old.write_token();
                        assert_eq!(game.attachment_validate_write(token.clone()), Ok(()));
                        *source.lock().unwrap() = Some((index, token));
                        game.lan_open_crafting(12, [0, 64, 0]);
                        let host = game.server.lan.as_mut().unwrap();
                        host.menus
                            .get_mut(&12)
                            .unwrap()
                            .crafting
                            .table
                            .as_mut()
                            .unwrap()
                            .2
                            .load_slot(1, Some(ItemStack::new("minecraft:stick", 3)));
                        host.hub.guests.update(0, |guest| {
                            guest.carried = Some(ItemStack::new("minecraft:apple", 2));
                        });
                        if death {
                            game.guest_respawn(12);
                            let mut drops: Vec<_> = game
                                .dropped
                                .iter()
                                .map(|item| (item.stack.item.to_string(), item.stack.count))
                                .collect();
                            drops.sort();
                            assert_eq!(
                                drops,
                                [
                                    ("minecraft:apple".to_owned(), 2),
                                    ("minecraft:oak_planks".to_owned(), 5),
                                    ("minecraft:stick".to_owned(), 3)
                                ]
                            );
                            assert_eq!(
                                issued(&VALUE).get(game, Holder::Player(12), Clone::clone),
                                Ok(999),
                                "the old UUID checkpoint must not overwrite the clone listener's write"
                            );
                            assert_eq!(
                                issued(&RETAINED).get(game, Holder::Player(12), Clone::clone),
                                Ok(77)
                            );
                        } else {
                            game.handle_server_portal(12);
                            assert!(game.command_guest(12).unwrap().won_game);
                            assert_eq!(*closed.lock().unwrap(), 1, "menu closes at credits entry");
                            assert_eq!(*cloned.lock().unwrap(), 0);
                            let guest = game.command_guest(12).unwrap();
                            assert!(!guest.placed && guest.respawning);
                            let before = guest.motion.as_ref().map(|m| m.health.tick_count);
                            game.tick_guests();
                            let guest = game.command_guest(12).unwrap();
                            assert_eq!(guest.motion.as_ref().map(|m| m.health.tick_count), before);
                            assert!(!game.server.lan.as_ref().unwrap().hub.here(guest));
                            game.guest_respawn(12);
                        }
                        let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                        assert_eq!(
                            guest
                                .inventory
                                .iter()
                                .flatten()
                                .map(|stack| stack.count)
                                .sum::<u32>(),
                            if death { 0 } else { 10 }
                        );
                        assert_eq!(*closed.lock().unwrap(), 1);
                        assert_eq!(*cloned.lock().unwrap(), 1);
                    });
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn direct_respawn_clones_in_the_resolved_level_not_the_callers_level() {
        use sunrise_core::dimension::Dimension;
        use sunrise_level::chunk::Chunk;

        struct Destination {
            missing: bool,
            heard: Arc<Mutex<Vec<String>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for Destination {
            fn player_respawn_position(
                &self,
                _: &mut dyn ServerHost,
                event: &mut mod_api::PlayerRespawnPositionEvent<'_>,
            ) {
                assert_eq!(event.missing_respawn_block, self.missing);
                assert_eq!(event.original_level, Dimension::Overworld);
                if self.missing {
                    *event.level = Dimension::End;
                    *event.position = [2.5, 90.0, 4.5];
                }
            }

            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                assert_eq!(
                    event.original.level().unwrap().dimension_id(),
                    Dim::Overworld.id()
                );
                self.heard.lock().unwrap().push(
                    host.player(event.player)
                        .unwrap()
                        .level(host)
                        .unwrap()
                        .dimension_id()
                        .to_owned(),
                );
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
            for missing in [false, true] {
                let heard = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "clone",
                    Box::new(Destination {
                        missing,
                        heard: heard.clone(),
                    }),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_guests(|game| {
                        game.options.dedicated = Some(Vec::new());
                        game.level.world_spawn = Some([0, 100, 0]);
                        assert!(game.ui.rules.set_string("respawn_radius", "0"));
                        if missing {
                            for x in -2..=2 {
                                for z in -2..=2 {
                                    game.server
                                        .streaming
                                        .insert_remote(&mut game.common.world, Chunk::empty(x, z));
                                }
                            }
                        }
                        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                            g.dead = true;
                            g.health = 0.0;
                            g.spawn = (!missing).then_some([7.5, 64.0, 5.5]);
                            g.respawn_elsewhere = missing.then(|| {
                                (Dim::Nether.id().to_owned(), [0, 100, 0], 0.0, 0.0, false)
                            });
                        });
                        game.enter_level(Dim::End).unwrap();
                        let player = game.player(12).unwrap();
                        player.respawn(game).unwrap();
                        assert_eq!(game.dim, Dim::End, "the caller's tick level is restored");
                        if missing {
                            assert!(
                                heard.lock().unwrap().is_empty(),
                                "do not clone before checking the Nether point"
                            );
                            game.enter_level(Dim::Nether).unwrap();
                            for x in -2..=2 {
                                for z in -2..=2 {
                                    game.server
                                        .streaming
                                        .insert_remote(&mut game.common.world, Chunk::empty(x, z));
                                }
                            }
                            game.tick_server_travel();
                        }
                        let wanted = if missing { Dim::End } else { Dim::Overworld };
                        assert_eq!(*heard.lock().unwrap(), [wanted.id().to_owned()]);
                        let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                        assert_eq!(guest.dimension, wanted.id());
                        assert_eq!(
                            guest.position,
                            if missing {
                                [2.5, 90.0, 4.5]
                            } else {
                                [7.5, 64.0, 5.5]
                            }
                        );
                        assert!(!guest.dead);
                    })
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn unsubscribed_end_return_copies_only_serializable_attachments() {
        struct DeclareOnly;

        #[mod_api::__subscriptions]
        impl Hooks for DeclareOnly {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type(describe("clone:unsaved", &VALUE))
                    .unwrap();
                r.register_attachment_type(describe("clone:saved", &SAVED))
                    .unwrap();
                r.register_attachment_type(describe("clone:retained", &RETAINED))
                    .unwrap();
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
            let mods = vec![mod_api::ModEntry::new("clone", Box::new(DeclareOnly))].into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.enter_level(Dim::End).unwrap();
                    let host = game.server.lan.as_mut().unwrap();
                    host.hub.guests.update(0, |guest| {
                        guest.dimension = Dim::End.id().to_owned();
                        guest.spawn = Some([3.5, 64.0, 5.5]);
                    });
                    let attachments = host.hub.guests.attachments_mut(12).unwrap();
                    VALUE.set(attachments, Holder::Level, 73).unwrap();
                    SAVED.set(attachments, Holder::Level, 31).unwrap();
                    RETAINED.set(attachments, Holder::Level, 17).unwrap();
                    host.hub.update_guest(12, |guest| guest.won_game = true);
                    game.guest_respawn(12);

                    let host: &mut dyn ServerHost = game;
                    assert_eq!(issued(&VALUE).has(host, Holder::Player(12)), Ok(false));
                    assert_eq!(
                        issued(&SAVED).with_decoded(host, Holder::Player(12), |v| v.copied()),
                        Ok(None)
                    );
                    assert_eq!(
                        issued(&SAVED).get(host, Holder::Player(12), Clone::clone),
                        Ok(31)
                    );
                    assert_eq!(
                        issued(&RETAINED).with_decoded(host, Holder::Player(12), |v| v.copied()),
                        Ok(None)
                    );
                    assert_eq!(
                        issued(&RETAINED).get(host, Holder::Player(12), Clone::clone),
                        Ok(17)
                    );
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn respawn_copies_only_committed_attachment_state() {
        struct Declare;
        #[mod_api::__subscriptions]
        impl Hooks for Declare {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type(describe("clone:retained", &RETAINED))
                    .unwrap();
            }
        }
        struct Observe(Arc<Mutex<Vec<i32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observe {
            fn player_clone(&self, _: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                self.0.lock().unwrap().push(
                    event
                        .original
                        .existing_attachment(&issued(&RETAINED), |value| value.cloned())
                        .unwrap()
                        .unwrap(),
                );
            }
        }
        sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
            for subscribe in [false, true] {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let mut mods = vec![mod_api::ModEntry::new("clone", Box::new(Declare))];
                if subscribe {
                    mods.push(mod_api::ModEntry::new(
                        "observe",
                        Box::new(Observe(seen.clone())),
                    ));
                }
                mod_platform::testing::with_mods(mods.into(), || {
                    with_guests(|game| {
                        game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                            g.spawn = Some([3.5, 64.0, 5.5]);
                            g.dead = true;
                            g.health = 0.0;
                        });
                        let holder = Holder::Player(12);
                        issued(&RETAINED).set(game, holder, 17).unwrap();
                        game.storage_scope(|game, _| {
                            issued(&RETAINED).set(game, holder, 23).unwrap();
                            game.guest_respawn(12);
                            assert!(game.command_guest(12).unwrap().dead);
                            ((), false)
                        });
                        assert!(game.command_guest(12).unwrap().dead);
                        assert_eq!(issued(&RETAINED).get(game, holder, Clone::clone), Ok(17));
                        assert!(seen.lock().unwrap().is_empty());
                        game.storage_scope(|game, _| {
                            issued(&RETAINED).set(game, holder, 29).unwrap();
                            game.guest_respawn(12);
                            ((), true)
                        });
                        assert!(!game.command_guest(12).unwrap().dead);
                        assert_eq!(issued(&RETAINED).get(game, holder, Clone::clone), Ok(29));
                        assert_eq!(
                            &*seen.lock().unwrap(),
                            if subscribe { &[29][..] } else { &[] }
                        );
                    });
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn failed_transaction_rolls_back_both_original_and_replacement_attachments() {
        struct AbortWriter {
            heard: Arc<Mutex<usize>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for AbortWriter {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type(describe("clone:unsaved", &VALUE))
                    .unwrap();
                r.register_attachment_type(describe("clone:defaulted", &DEFAULTED))
                    .unwrap();
                r.register_attachment_type(describe("clone:saved", &SAVED))
                    .unwrap();
                r.register_attachment_type(describe("clone:retained", &RETAINED))
                    .unwrap();
            }

            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                let aborted: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, _| {
                        issued(&VALUE)
                            .set(host, Holder::Player(event.player), 555)
                            .unwrap();
                        event.original.set_attachment(&issued(&VALUE), 444).unwrap();
                        assert_eq!(
                            event.original.remove_attachment(&issued(&RETAINED)),
                            Ok(Some(17))
                        );
                        assert_eq!(
                            event.original.attachment(&issued(&DEFAULTED), Clone::clone),
                            Ok(41)
                        );
                        assert_eq!(
                            event
                                .original
                                .existing_attachment(&issued(&SAVED), |value| value.cloned()),
                            Ok(Some(31))
                        );
                        // Inner committed transaction must still roll back when outer aborts.
                        host.transaction(|_, _| {
                            event.original.set_attachment(&issued(&VALUE), 445).unwrap();
                            Ok::<(), ()>(())
                        })
                        .unwrap();
                        assert_eq!(
                            event
                                .original
                                .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                            Ok(Some(445))
                        );
                        assert_eq!(
                            issued(&VALUE).get(host, Holder::Player(event.player), Clone::clone),
                            Ok(555)
                        );
                        Err(())
                    });
                assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                assert_eq!(
                    issued(&VALUE).has(host, Holder::Player(event.player)),
                    Ok(false)
                );
                assert_eq!(
                    event
                        .original
                        .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                    Ok(Some(73))
                );
                assert_eq!(event.original.has_attachment(&issued(&RETAINED)), Ok(true));
                assert_eq!(
                    event.original.has_attachment(&issued(&DEFAULTED)),
                    Ok(false)
                );
                assert_eq!(
                    event
                        .original
                        .with_decoded_attachment(&issued(&SAVED), |value| value.copied()),
                    Ok(None)
                );
                *self.heard.lock().unwrap() += 1;
            }
        }

        struct CleanObserver {
            heard: Arc<Mutex<usize>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for CleanObserver {
            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                assert_eq!(
                    issued(&VALUE).has(host, Holder::Player(event.player)),
                    Ok(false)
                );
                assert_eq!(
                    issued(&RETAINED).get(host, Holder::Player(event.player), Clone::clone),
                    Ok(17)
                );
                assert_eq!(
                    event
                        .original
                        .existing_attachment(&issued(&VALUE), |value| value.cloned()),
                    Ok(Some(73))
                );
                assert_eq!(event.original.has_attachment(&issued(&RETAINED)), Ok(true));
                assert_eq!(
                    event.original.has_attachment(&issued(&DEFAULTED)),
                    Ok(false)
                );
                assert_eq!(
                    event
                        .original
                        .with_decoded_attachment(&issued(&SAVED), |value| value.copied()),
                    Ok(None)
                );
                *self.heard.lock().unwrap() += 1;
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
            let writer_heard = Arc::new(Mutex::new(0));
            let observer_heard = Arc::new(Mutex::new(0));
            let mods = vec![
                mod_api::ModEntry::new(
                    "clone",
                    Box::new(AbortWriter {
                        heard: writer_heard.clone(),
                    }),
                ),
                mod_api::ModEntry::new(
                    "observer",
                    Box::new(CleanObserver {
                        heard: observer_heard.clone(),
                    }),
                ),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let host = game.server.lan.as_mut().unwrap();
                    host.hub.guests.update(0, |guest| {
                        guest.dead = true;
                        guest.health = 0.0;
                        guest.spawn = Some([3.5, 64.0, 5.5]);
                    });
                    let attachments = host.hub.guests.attachments_mut(12).unwrap();
                    RETAINED.set(attachments, Holder::Level, 17).unwrap();
                    SAVED.set(attachments, Holder::Level, 31).unwrap();
                    let saved = Attachments::from_nbt(attachments.try_to_nbt().unwrap());
                    attachments.replace_contents(saved);
                    VALUE.set(attachments, Holder::Level, 73).unwrap();
                    game.guest_respawn(12);
                    assert_eq!(*writer_heard.lock().unwrap(), 1);
                    assert_eq!(*observer_heard.lock().unwrap(), 1);
                });
            });
        });
    }
}
