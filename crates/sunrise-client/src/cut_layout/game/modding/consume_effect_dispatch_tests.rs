//! Explicit living consume effects reach real native listeners, including nested kinds.
#![cfg(test)]
use mod_api::{
    __private::{HookPanicReport, game::CallbackOwner, take_hook_panics},
    ConsumeEffectEvent, Hooks, HostTransaction, ModEntry, ObjectiveRenderType, ServerHost, Text,
};
use std::sync::{Arc, Mutex};
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_items::items::{ConsumeEffect, ConsumeSound, ItemStack, parse_consumable};
use sunrise_sim::mobs::Mob;
use {
    sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game},
    sunrise_gameplay::game::server_side::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::{Dim, api_values::TryIntoApi},
};

struct First(Arc<Mutex<Vec<&'static str>>>);
#[mod_api::__subscriptions]
impl Hooks for First {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        // Both listeners subscribe to all custom consume entries. The engine,
        // not a guard in the listener, must keep other kinds out of this callback.
        assert_eq!(event.kind, "first:effect");
        self.0.lock().unwrap().push("first");
        let source = host.entity(21).unwrap();
        assert_eq!(event.user, source);
        assert_eq!(
            event.stack.item(),
            host.item_key(&mod_api::Identifier::parse("minecraft:pufferfish").unwrap())
                .unwrap()
        );
        if event.stack.count() == 1 {
            return;
        }
        assert_eq!(event.stack.count(), 2);
        let living = host.living(source).unwrap();
        let same = ItemStack::new("minecraft:pufferfish", 1)
            .try_into_api()
            .unwrap();
        assert_eq!(living.apply_consume_effect(host, &same, 0), Ok(Some(true)));
        let other = ItemStack::new("minecraft:honey_bottle", 1)
            .try_into_api()
            .unwrap();
        assert_eq!(living.apply_consume_effect(host, &other, 0), Ok(Some(true)));
        self.0.lock().unwrap().push("first_after");
    }
}
struct Second(Arc<Mutex<Vec<&'static str>>>);
#[mod_api::__subscriptions]
impl Hooks for Second {
    fn consume_effect(&self, _: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        assert_eq!(event.kind, "second:effect");
        self.0.lock().unwrap().push("second");
    }
}

#[test]
#[ignore = "daily"]
fn nested_consume_effects_follow_call_order_even_for_the_same_kind() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new("first", Box::new(First(trace.clone()))),
        ModEntry::new("second", Box::new(Second(trace.clone()))),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let mut items = game.items().clone();
            for (item, kind) in [
                ("minecraft:pufferfish", "first:effect"),
                ("minecraft:honey_bottle", "second:effect"),
                ("minecraft:apple", "unclaimed:effect"),
            ] {
                let component = parse_consumable(&serde_json::json!({
                    "on_consume_effects": [{"type": kind}]
                }))
                .unwrap();
                items.items.get_mut(item).unwrap().consumable = Some(Arc::new(component));
            }
            game.server_items = Some(Arc::new(items));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            fixture_resident_column(&mut game.world, (0, 0));
            game.mobs
                .push(Mob::new(21, kind, Vec3::new(0.0, 64.0, 0.0), 0.0));
            let living = game.living(game.entity(21).unwrap()).unwrap();
            let stack = ItemStack::new("minecraft:pufferfish", 2)
                .try_into_api()
                .unwrap();
            assert_eq!(living.apply_consume_effect(game, &stack, 0), Ok(Some(true)));
            let unclaimed = ItemStack::new("minecraft:apple", 1).try_into_api().unwrap();
            assert_eq!(
                living.apply_consume_effect(game, &unclaimed, 0),
                Ok(Some(false))
            );
            assert_eq!(
                *trace.lock().unwrap(),
                ["first", "first", "second", "first_after"]
            );
        });
    });
}

struct PlayerConsumption(Arc<Mutex<Vec<(mod_api::EntityRef, u32)>>>);
#[mod_api::__subscriptions]
impl Hooks for PlayerConsumption {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        if event.kind == "player_meal:effect" {
            assert_eq!(host.entity(event.user.id()), Some(event.user));
            self.0
                .lock()
                .unwrap()
                .push((event.user, event.stack.count()));
        }
    }
}

#[test]
#[ignore = "daily"]
fn consuming_player_is_supplied_outside_a_guest_action() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "player_meal",
            Box::new(PlayerConsumption(heard.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let mut body =
                    sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 65.0, 0.0))
                        .unwrap();
                body.entity_id = 7;
                game.player = Some(body);
                game.bind_local_player_residency();
                assert!(game.native_acting_player().is_none());
                let user = game.entity(7).unwrap();
                let stack = ItemStack::new("minecraft:pufferfish", 4);
                let info = game.items().get(&stack.item).unwrap().clone();
                let consumable = parse_consumable(&serde_json::json!({
                    "on_consume_effects": [{"type": "player_meal:effect"}]
                }))
                .unwrap();
                game.creative = false;
                game.common.inventory[game.common.selected] = Some(stack.clone());
                game.finish_consuming(&info, &consumable, &stack);
                assert_eq!(*heard.lock().unwrap(), [(user, 4)]);
                assert_eq!(game.held().unwrap().count, 3);
            })
        },
    );
}

struct ReplaceDrink;
#[mod_api::__subscriptions]
impl Hooks for ReplaceDrink {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        let expected =
            host.item_key(&mod_api::Identifier::parse("minecraft:honey_bottle").unwrap());
        assert_eq!(Some(event.stack.item()), expected);
        let living = host.living(event.user).unwrap();
        let replacement = ItemStack::new("minecraft:stick", 1).try_into_api().unwrap();
        assert_eq!(
            living.set_hand_item(host, mod_api::Hand::Main, Some(replacement)),
            Ok(true)
        );
    }
}

#[test]
#[ignore = "daily"]
fn replacing_the_consumed_hand_keeps_the_new_item_and_emits_no_remainder() {
    let mods = vec![ModEntry::new("replace_drink", Box::new(ReplaceDrink))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mut body =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 65.0, 0.0))
                    .unwrap();
            body.entity_id = 7;
            game.player = Some(body);
            game.bind_local_player_residency();
            game.creative = false;
            game.inventory.fill(None);
            let mut stack = ItemStack::new("minecraft:honey_bottle", 1);
            stack.data_mut().set_other(
                "minecraft:consumable",
                Some(Tag::compound([(
                    "on_consume_effects",
                    Tag::List(vec![Tag::compound([(
                        "type",
                        Tag::String("replace_drink:drink".into()),
                    )])]),
                )])),
            );
            let info = game.items().get(&stack.item).unwrap().clone();
            let consumable = game.consumable(&stack).unwrap();
            game.common.inventory[game.common.selected] = Some(stack.clone());
            game.finish_consuming(&info, &consumable, &stack);
            assert_eq!(game.held(), Some(&ItemStack::new("minecraft:stick", 1)));
            assert_eq!(game.inventory.iter().flatten().count(), 1);
            assert!(game.dropped.is_empty());
        });
    });
}

struct ObservePotion(Arc<Mutex<Vec<(String, bool)>>>);
#[mod_api::__subscriptions]
impl Hooks for ObservePotion {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        let living = host.living(event.user).unwrap();
        let breathing = host.effect_key("minecraft:water_breathing").unwrap();
        use std::ops::ControlFlow;
        let has_effect = living
            .visit_effects(host, &mut |effect| {
                if effect.id == breathing {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap()
            .is_break();
        assert_eq!(event.stack.count(), 1);
        self.0
            .lock()
            .unwrap()
            .push((event.kind.to_owned(), has_effect));
    }
}

#[test]
#[ignore = "daily"]
fn potion_finishes_with_its_patched_effects_after_potion_contents() {
    use sunrise_core::random::RandomSource;

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "potion_order",
        Box::new(ObservePotion(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mut body =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 65.0, 0.0))
                    .unwrap();
            body.entity_id = 7;
            game.player = Some(body);
            game.bind_local_player_residency();
            game.creative = false;
            game.player.as_mut().unwrap().random.set_seed(41);
            let mut potion =
                ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water_breathing");
            potion.data_mut().set_other(
                "minecraft:consumable",
                Some(Tag::compound([(
                    "on_consume_effects",
                    Tag::List(
                        [
                            "potion_order:before_clear",
                            "minecraft:clear_all_effects",
                            "potion_order:after_clear",
                        ]
                        .map(|id| Tag::compound([("type", Tag::String(id.into()))]))
                        .into(),
                    ),
                )])),
            );
            let info = game.items().get(&potion.item).unwrap().clone();
            let slot = game.selected;
            game.inventory[slot] = Some(potion.clone());
            game.complete_using_item(slot, &potion, &info);
            // LegacyRandomSource after the emission's boolean, three floats and 16 crumbs.
            assert_eq!(
                game.player.as_ref().unwrap().random.clone().next_long(),
                -4_022_577_910_208_863_423,
            );
            assert_eq!(
                *seen.lock().unwrap(),
                [
                    ("potion_order:before_clear".into(), true),
                    ("potion_order:after_clear".into(), false)
                ]
            );
            assert_eq!(
                game.held(),
                Some(&ItemStack::new("minecraft:glass_bottle", 1))
            );
        });
    });
}

struct PanicsDuringConsumption(Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::__subscriptions]
impl Hooks for PanicsDuringConsumption {
    fn consume_effect(&self, _: &mut dyn ServerHost, _: &mut ConsumeEffectEvent<'_>) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        panic!("consume effect fixture");
    }
}

#[test]
#[ignore = "daily"]
fn a_consume_callback_panic_leaves_later_effects_and_other_callbacks_running() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new(
            "panics_during_meal",
            Box::new(PanicsDuringConsumption(calls.clone())),
        ),
        ModEntry::new("observes_meal", Box::new(ObservePotion(seen.clone()))),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let mut body =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 65.0, 0.0))
                    .unwrap();
            body.entity_id = 7;
            game.player = Some(body);
            game.bind_local_player_residency();
            game.creative = false;
            let stack = ItemStack::new("minecraft:apple", 1);
            let info = game.items().get(&stack.item).unwrap().clone();
            let consumable = parse_consumable(&serde_json::json!({
                "on_consume_effects": [
                    {"type": "panics_during_meal:first"},
                    {"type": "minecraft:apply_effects", "effects": [
                        {"id": "minecraft:water_breathing", "duration": 120}
                    ]},
                    {"type": "observes_meal:after_builtin"},
                    {"type": "panics_during_meal:again"}
                ]
            }))
            .unwrap();
            for _ in 0..2 {
                game.common.inventory[game.common.selected] = Some(stack.clone());
                game.finish_consuming(&info, &consumable, &stack);
                assert!(game.held().is_none());
            }
            assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(
                *seen.lock().unwrap(),
                [
                    ("observes_meal:after_builtin".into(), true),
                    ("observes_meal:after_builtin".into(), true)
                ]
            );
        });
    });
}

struct DelegatingMeal;
#[mod_api::__subscriptions]
impl Hooks for DelegatingMeal {
    fn consume_effect(&self, host: &mut dyn ServerHost, _: &mut ConsumeEffectEvent<'_>) {
        let guest = host.living(host.entity(13).unwrap()).unwrap();
        let speed = host.effect_key("minecraft:speed").unwrap();
        let effect = mod_api::ActiveEffect::new(speed, 120, 0, false, true, true);
        assert!(guest.add_effect(host, &effect));
    }
}

struct PanickingEffectListener(std::sync::atomic::AtomicBool);
#[mod_api::__subscriptions]
impl Hooks for PanickingEffectListener {
    fn mob_effect_applicable(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::MobEffectApplicableEvent,
    ) -> Option<mod_api::EffectApplicable> {
        if event.entity.entity().id() == 13
            && self.0.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            panic!("foreign effect listener");
        }
        None
    }
}

#[test]
#[ignore = "daily"]
fn a_foreign_listener_is_quarantined_before_returning_to_the_outer_guest() {
    let mods = vec![
        ModEntry::new("delegating_meal", Box::new(DelegatingMeal)),
        ModEntry::new(
            "effect_listener",
            Box::new(PanickingEffectListener(true.into())),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_guests(|game| {
            let world = tempfile::tempdir().unwrap();
            game.server.world_root = world.path().to_path_buf();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.dimension = Dim::Nether.id().into())
                .unwrap();
            let outer_dim = game.dim;
            game.as_guest(12, |game| {
                use sunrise_game_data::text::Text;
                game.server.chat_capture = Some(vec![Text::literal("outer guest")]);
                let stack = ItemStack::new("minecraft:apple", 1);
                let info = game.items().get(&stack.item).unwrap().clone();
                let consumable = parse_consumable(&serde_json::json!({
                    "on_consume_effects": [
                        {"type": "delegating_meal:effect"},
                        {"type": "minecraft:apply_effects", "effects": [
                            {"id": "minecraft:water_breathing", "duration": 120}
                        ]}
                    ]
                }))
                .unwrap();
                game.common.inventory[game.common.selected] = Some(stack.clone());
                game.finish_consuming(&info, &consumable, &stack);
                assert_eq!(game.server.lan_acting, Some(12));
                assert_eq!(game.dim, outer_dim);
                assert_eq!(
                    game.server.chat_capture.as_ref().unwrap()[0].plain(),
                    "outer guest"
                );
                assert!(game.held().is_none());
                assert!(game.potions.effects.get("water_breathing").is_some());
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "effect_listener",
                        hook: "mob_effect_applicable",
                    }]
                );
                let other = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
                assert!(other.effects.get("speed").is_some());
                assert!(other.effects.get("water_breathing").is_none());
                assert_eq!(other.dimension, Dim::Nether.id());
                assert_eq!(game.dim, outer_dim);
                game.server.chat_capture = None;
            })
            .unwrap();
        });
    });
}

#[test]
#[ignore = "daily"]
fn interleaved_guests_keep_their_vanilla_consume_draw_order() {
    use sunrise_core::random::RandomSource;

    mod_platform::testing::with_mods(Vec::<ModEntry>::new().into(), || {
        with_guests(|game| {
            game.world.chunks.clear(); // No candidate has a loaded block to land on.
            let mut level_random = game.world.random.clone();
            let stack = ItemStack::new("minecraft:apple", 1);
            let info = game.items().get(&stack.item).unwrap().clone();
            let consumable = parse_consumable(&serde_json::json!({
                "on_consume_effects": [
                    {"type": "minecraft:apply_effects", "probability": 0.5, "effects": [
                        {"id": "minecraft:water_breathing", "duration": 120}
                    ]},
                    {"type": "minecraft:teleport_randomly"}
                ]
            }))
            .unwrap();
            for (conn, seed) in [(12, 29), (13, 13)] {
                game.as_guest(conn, |game| {
                    game.player.as_mut().unwrap().random.set_seed(seed)
                })
                .unwrap();
            }

            // LegacyRandomSource: emission takes a boolean and three floats,
            // spawnItemParticles four floats per crumb (16), FoodProperties
            // three more floats, apply_effects one, and teleport three doubles
            // for each of Vanilla TeleportRandomlyConsumeEffect's 16 attempts.
            let expected = [
                (12, true, 5_428_492_878_003_447_481),
                (13, false, -1_008_312_113_262_950_709),
                (12, false, -2_516_599_419_424_563_847),
            ];
            for (user, breathing, next_random_long) in expected {
                game.as_guest(user, |game| {
                    let at = game.player.as_ref().unwrap().position;
                    game.remove_player_effects(None);
                    game.common.inventory[game.common.selected] = Some(stack.clone());
                    game.finish_consuming(&info, &consumable, &stack);
                    assert_eq!(
                        game.potions.effects.get("water_breathing").is_some(),
                        breathing
                    );
                    let player = game.player.as_ref().unwrap();
                    assert_eq!(player.position, at);
                    assert_eq!(player.random.clone().next_long(), next_random_long);
                })
                .unwrap();
            }
            assert_eq!(game.world.random.next_long(), level_random.next_long());
        });
    });
}

struct FoxConsumption(Arc<Mutex<Vec<(mod_api::EntityRef, mod_api::ItemStack)>>>);
#[mod_api::__subscriptions]
impl Hooks for FoxConsumption {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        match event.kind {
            "fox_meal:discard" => assert!(event.user.discard(host)),
            "fox_meal:remember" => {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.user, event.stack.clone()));
            }
            _ => {}
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_foxs_later_custom_effect_keeps_the_original_user_and_stack_after_discard() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "fox_meal",
        Box::new(FoxConsumption(heard.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let mut items = game.items().clone();
            let consumable = items
                .items
                .get_mut("minecraft:chorus_fruit")
                .unwrap()
                .consumable
                .as_mut()
                .unwrap();
            let effects = &mut Arc::make_mut(consumable).effects;
            assert!(
                effects
                    .iter()
                    .any(|effect| matches!(effect, ConsumeEffect::TeleportRandomly(_)))
            );
            let custom = |kind: &str| {
                parse_consumable(&serde_json::json!({"on_consume_effects": [{"type": kind}]}))
                    .unwrap()
                    .effects
                    .remove(0)
            };
            effects.insert(0, custom("fox_meal:discard"));
            effects.push(ConsumeEffect::PlaySound(ConsumeSound::Registered(
                "minecraft:entity.fox.eat".into(),
            )));
            effects.push(custom("fox_meal:remember"));
            game.server_items = Some(Arc::new(items));

            let kind = game.res.mob_types().get("minecraft:fox").unwrap().clone();
            let mut fox = Mob::new(21, kind, Vec3::new(8.5, 64.0, 8.5), 0.0);
            fox.ground.on_ground = true;
            fox.extra.ints[1] = 601;
            let mut stack = ItemStack::new("minecraft:chorus_fruit", 7);
            stack
                .data_mut()
                .set_loot_name("minecraft:custom_name", &serde_json::json!("Fox fruit"));
            let expected_stack = stack.try_into_api().unwrap();
            fox.extra.set_main_hand_stack(Some(stack));
            let decision =
                sunrise_sim::mobs::fox::decide_consumption(&fox, &game.mob_world_base(&[]))
                    .unwrap();
            fixture_resident_column(&mut game.world, (0, 0));
            game.mobs.push(fox);
            let user = game.entity(21).unwrap();
            assert!(!game.apply_fox_consumption(&decision));
            assert!(game.entity(21).is_none());
            assert_eq!(*heard.lock().unwrap(), [(user, expected_stack)]);
        });
    });
}

struct HealingMeal;
#[mod_api::__subscriptions]
impl Hooks for HealingMeal {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        let living = host.living(event.user).unwrap();
        assert!(living.heal(host, 2.0));
    }
}

struct PanickingHealListener(std::sync::atomic::AtomicBool);
#[mod_api::__subscriptions]
impl Hooks for PanickingHealListener {
    fn living_heal(
        &self,
        _: &dyn mod_api::LoopContext,
        _: &mut mod_api::LivingHealEvent,
    ) -> mod_api::Flow {
        if self.0.swap(false, std::sync::atomic::Ordering::Relaxed) {
            panic!("foreign heal listener");
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_foreign_heal_listener_is_quarantined_without_disabling_the_consume_callback() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::consume_effect_dispatch_tests::a_foreign_heal_listener_is_quarantined_without_disabling_the_consume_callback",
        "SUNRISE_CONSUME_HEAL_HOOKS_CHILD",
        &["foreign_heal"],
    ) {
        return;
    }
    crate::entry::install_client_hooks();
    let mods = vec![
        ModEntry::new("healing_meal", Box::new(HealingMeal)),
        ModEntry::new(
            "heal_listener",
            Box::new(PanickingHealListener(true.into())),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_guests(|game| {
            game.as_guest(12, |game| {
                let id = game.player.as_ref().unwrap().entity_id as u32;
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert!(living.set_health(game, 10.0));
                let stack = ItemStack::new("minecraft:apple", 1);
                let info = game.items().get(&stack.item).unwrap().clone();
                let consumable = parse_consumable(&serde_json::json!({
                    "on_consume_effects": [
                        {"type": "healing_meal:effect"},
                        {"type": "minecraft:apply_effects", "effects": [
                            {"id": "minecraft:water_breathing", "duration": 120}
                        ]}
                    ]
                }))
                .unwrap();
                for expected_health in [12.0, 14.0] {
                    game.common.inventory[game.common.selected] = Some(stack.clone());
                    game.finish_consuming(&info, &consumable, &stack);
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        expected_health
                    );
                    assert!(game.held().is_none());
                }
                assert!(game.potions.effects.get("water_breathing").is_some());
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "heal_listener",
                        hook: "living_heal",
                    }]
                );
            })
            .unwrap();
        });
    });
}

struct PanickingModeListener(std::sync::atomic::AtomicBool);
#[mod_api::__subscriptions]
impl Hooks for PanickingModeListener {
    fn player_game_mode_changed(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::PlayerGameModeChangedEvent<'_>,
    ) -> mod_api::Flow {
        if self.0.swap(false, std::sync::atomic::Ordering::Relaxed) {
            panic!("foreign mode listener");
        }
        mod_api::Flow::Continue
    }
}

struct FeedbackMeal(Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::__subscriptions]
impl Hooks for FeedbackMeal {
    fn consume_effect(&self, host: &mut dyn ServerHost, _: &mut ConsumeEffectEvent<'_>) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (sender, feedback) = std::sync::mpsc::channel();
        let ran = host
            .commands()
            .run_command_with_feedback("gamemode creative guest12", sender);
        let lines: Vec<String> = feedback
            .try_iter()
            .map(|f| match f.unwrap() {
                mod_api::CommandFeedback::Success(t)
                | mod_api::CommandFeedback::Failure(t)
                | mod_api::CommandFeedback::System(t) => {
                    format!("{t:?}")
                }
                _ => String::new(),
            })
            .collect();
        let _ = (ran, lines);
    }
}

/// `run_command_with_feedback` runs the console command inside `call_host`: a foreign listener's
/// panic under it is attributed to that listener and quarantined, and the consume callback that
/// issued the command is not disabled.
#[test]
#[ignore = "daily"]
fn a_foreign_listener_panicking_under_a_feedback_command_does_not_disable_the_consume_callback() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::consume_effect_dispatch_tests::a_foreign_listener_panicking_under_a_feedback_command_does_not_disable_the_consume_callback",
        "SUNRISE_CONSUME_FEEDBACK_CHILD",
        &["feedback"],
    ) {
        return;
    }
    crate::entry::install_client_hooks();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods = vec![
        ModEntry::new("feedback_meal", Box::new(FeedbackMeal(calls.clone()))),
        ModEntry::new(
            "mode_listener",
            Box::new(PanickingModeListener(true.into())),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_guests(|game| {
            game.as_guest(12, |game| {
                let stack = ItemStack::new("minecraft:apple", 1);
                let info = game.items().get(&stack.item).unwrap().clone();
                let consumable = parse_consumable(&serde_json::json!({
                    "on_consume_effects": [{"type": "feedback_meal:effect"}]
                }))
                .unwrap();
                for _ in 0..2 {
                    game.common.inventory[game.common.selected] = Some(stack.clone());
                    game.finish_consuming(&info, &consumable, &stack);
                }
                assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "mode_listener",
                        hook: "player_game_mode_changed",
                    }]
                );
            })
            .unwrap();
        });
    });
}

struct PanickingProjectileJoinListener(std::sync::atomic::AtomicBool);
#[mod_api::__subscriptions]
impl Hooks for PanickingProjectileJoinListener {
    fn entity_join_level(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::EntityJoinLevelEvent<'_>,
    ) -> mod_api::Flow {
        if event.kind == "minecraft:snowball"
            && self.0.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            panic!("projectile join listener");
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_panicking_guest_turn_restores_cross_level_state_and_quarantines_its_join_listener() {
    let mods = vec![ModEntry::new(
        "launch_listener",
        Box::new(PanickingProjectileJoinListener(true.into())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_guests(|game| {
            let world = tempfile::tempdir().unwrap();
            game.server.world_root = world.path().to_path_buf();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.dimension = Dim::Nether.id().into())
                .unwrap();
            let outer_dim = game.dim;
            game.as_guest(12, |game| {
                use sunrise_game_data::text::Text;
                use sunrise_sim::{
                    mobs::Who,
                    projectiles::{Projectile, ProjectileKind},
                };
                game.server.chat_capture = Some(vec![Text::literal("outer guest")]);
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.for_player(
                        sunrise_gameplay_common::game::commands_common::Target::Guest(13),
                        |game| {
                            game.server
                                .chat_capture
                                .as_mut()
                                .unwrap()
                                .push(Text::literal("inner guest"));
                            game.add_fresh_projectile(Projectile::new(
                                ProjectileKind::Snowball,
                                Vec3::ZERO,
                                Some(Who::Player),
                            ));
                            panic!("cross-level command body");
                        },
                    );
                }))
                .unwrap_err();
                assert_eq!(
                    panic.downcast_ref::<&str>(),
                    Some(&"cross-level command body")
                );
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "launch_listener",
                        hook: "entity_join_level",
                    }]
                );
                assert_eq!(game.server.lan_acting, Some(12));
                assert_eq!(game.server.lan_guest_swaps, [12]);
                assert_eq!(game.dim, outer_dim);
                assert_eq!(game.server.chat_capture.as_ref().unwrap().len(), 1);
                assert_eq!(
                    game.server.chat_capture.as_ref().unwrap()[0].plain(),
                    "outer guest"
                );
                assert_eq!(
                    game.for_player(
                        sunrise_gameplay_common::game::commands_common::Target::Guest(13),
                        |game| {
                            game.creatures
                                .projectiles
                                .iter()
                                .any(|projectile| projectile.guest == Some(13))
                        },
                    ),
                    Some(true)
                );
                assert_eq!(game.dim, outer_dim);
                assert_eq!(game.server.lan_acting, Some(12));
                assert_eq!(game.server.lan_guest_swaps, [12]);
                game.server.chat_capture = None;
            })
            .unwrap();
        });
    });
}

struct DropsRejectedTask {
    fill_queue: bool,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for DropsRejectedTask {
    fn consume_effect(&self, host: &mut dyn ServerHost, _: &mut ConsumeEffectEvent<'_>) {
        if self.fill_queue {
            for _ in 0..mod_api::MAX_PENDING_TASKS {
                host.schedule_task(1, Box::new(|_| {})).unwrap();
            }
        }
        struct Capture(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Capture {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Relaxed);
                panic!("rejected task capture");
            }
        }
        let capture = Capture(self.dropped.clone());
        let _ = host.schedule_task(1, Box::new(move |_| drop(capture)));
    }
}

#[test]
#[ignore = "daily"]
fn rejected_task_captures_unwind_only_the_invoking_consume_callback() {
    use std::sync::atomic::{AtomicBool, Ordering};

    for world_ready in [false, true] {
        let dropped = Arc::new(AtomicBool::new(false));
        let mods = vec![ModEntry::new(
            "task_meal",
            Box::new(DropsRejectedTask {
                fill_queue: world_ready,
                dropped: dropped.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                if world_ready {
                    game.native_lifecycle.world_open_ready().unwrap();
                } else {
                    game.native_lifecycle.begin_close();
                }
                game.as_guest(12, |game| {
                    let user = game.entity(12).unwrap();
                    let stack = ItemStack::new("minecraft:apple", 1);
                    let consumable = parse_consumable(&serde_json::json!({
                        "on_consume_effects": [{ "type": "task_meal:effect" }]
                    }))
                    .unwrap();
                    let [ConsumeEffect::Native(effect)] = consumable.effects.as_slice() else {
                        panic!("expected native effect");
                    };
                    assert!(!game.native_consume_effect(effect, &stack, user));
                    assert!(dropped.load(Ordering::Relaxed));
                    assert_eq!(
                        take_hook_panics(),
                        [HookPanicReport {
                            mod_id: "task_meal",
                            hook: "consume_effect",
                        }]
                    );
                    assert!(!game.native_consume_effect(effect, &stack, user));
                    assert!(take_hook_panics().is_empty());
                })
                .unwrap();
            });
        });
    }
}

struct CatchesEngineRelay(Arc<std::sync::atomic::AtomicUsize>);
#[mod_api::__subscriptions]
impl Hooks for CatchesEngineRelay {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        let commit = event.kind == "transaction_meal:commit";
        let result = host.transaction(|host, _| {
            host.add_objective(
                "staged",
                "dummy",
                Text::literal("Staged"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            assert!(host.objective("staged").is_some());
            let relay = std::panic::catch_unwind(|| {
                CallbackOwner::call_host(|| panic!("engine fault during consumption"));
            });
            assert!(relay.is_err());
            if commit { Ok(()) } else { Err(()) }
        });
        assert_eq!(
            result,
            Err(if commit {
                mod_api::TransactionError::CommitRefused
            } else {
                mod_api::TransactionError::Aborted(())
            })
        );
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[test]
#[ignore = "daily"]
fn catching_a_delegated_fault_aborts_the_journal_even_if_the_callback_returns_ok() {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::atomic::{AtomicUsize, Ordering},
    };

    let closed = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "transaction_meal",
        Box::new(CatchesEngineRelay(closed.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_guests(|game| {
            game.as_guest(12, |game| {
                let user = game.entity(12).unwrap();
                let stack = ItemStack::new("minecraft:apple", 1);
                for (attempt, kind) in ["transaction_meal:abort", "transaction_meal:commit"]
                    .into_iter()
                    .enumerate()
                {
                    let consumable = parse_consumable(&serde_json::json!({
                        "on_consume_effects": [{ "type": kind }]
                    }))
                    .unwrap();
                    let [ConsumeEffect::Native(effect)] = consumable.effects.as_slice() else {
                        panic!("expected native effect");
                    };
                    let panic = catch_unwind(AssertUnwindSafe(|| {
                        game.native_consume_effect(effect, &stack, user);
                    }))
                    .unwrap_err();
                    assert_eq!(
                        panic.downcast_ref::<&str>(),
                        Some(&"engine fault during consumption")
                    );
                    assert_eq!(closed.load(Ordering::Relaxed), attempt + 1);
                    assert!(game.transactions.is_empty());
                    assert!((game as &dyn ServerHost).objective("staged").is_none());
                    assert!(take_hook_panics().is_empty());
                }
            })
            .unwrap();
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
