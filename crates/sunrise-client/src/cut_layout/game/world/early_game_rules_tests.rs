#![cfg(test)]
use core::ops::ControlFlow;
use mod_api::{GameRuleValue, Hooks, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_game_data::gamerules::Value;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;

fn collect_rules<E>(
    enumerate: impl FnOnce(
        &mut dyn FnMut(&str, GameRuleValue) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, E>,
) -> Vec<(String, GameRuleValue)>
where
    E: core::fmt::Debug,
{
    let mut rules = Vec::new();
    assert_eq!(
        enumerate(&mut |id, value| {
            rules.push((id.to_owned(), value));
            ControlFlow::Continue(())
        })
        .unwrap(),
        ControlFlow::Continue(())
    );
    rules
}

struct Observe(Arc<Mutex<Vec<&'static str>>>);
fn assert_rules(host: &dyn ServerHost) {
    assert_eq!(
        host.game_rule("minecraft:keep_inventory"),
        Ok(mod_api::GameRuleValue::Bool(true))
    );
    assert_eq!(
        host.game_rule("minecraft:random_tick_speed"),
        Ok(mod_api::GameRuleValue::Int(7))
    );
    assert_eq!(
        host.game_rule("minecraft:not_a_rule"),
        Err(mod_api::GameRuleReadError::UnknownRule)
    );
    let rules = collect_rules(|visit| host.visit_game_rules(visit));
    assert!(rules.contains(&(
        "minecraft:keep_inventory".into(),
        mod_api::GameRuleValue::Bool(true)
    )));
    assert!(rules.contains(&(
        "minecraft:random_tick_speed".into(),
        mod_api::GameRuleValue::Int(7)
    )));
}
#[mod_api::__subscriptions]
impl Hooks for Observe {
    fn server_about_to_start(
        &self,
        host: &mut dyn ServerHost,
        _: &mut mod_api::ServerAboutToStartEvent,
    ) {
        assert_rules(host);
        self.0.lock().unwrap().push("about");
    }
    fn server_starting(&self, host: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
        assert_rules(host);
        self.0.lock().unwrap().push("starting");
    }
    fn game_rule_changed(&self, _: &mut dyn ServerHost, _: &mut mod_api::GameRuleChangedEvent<'_>) {
        self.0.lock().unwrap().push("changed");
    }
}

#[test]
#[ignore = "daily"]
fn early_server_game_rules_saved_and_creation_values() {
    for dedicated in [false, true] {
        for saved in [false, true] {
            let scratch = tempfile::tempdir().unwrap();
            let registry = Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla());
            let mut create = sunrise_sim::world_options::WorldOptions::default();
            assert!(create.rules.set("keep_inventory", Value::Bool(true)));
            assert!(create.rules.set("random_tick_speed", Value::Int(7)));
            if saved {
                let mut level = sunrise_sim::save::Level {
                    game_rules: create.rules.by_name(),
                    ..Default::default()
                };
                level.write(scratch.path(), &Default::default()).unwrap();
                create.rules.save(scratch.path()).unwrap();
            }
            let observed = Arc::new(Mutex::new(Vec::new()));
            mod_platform::testing::with_mods(
                vec![ModEntry::new(
                    "rule_read",
                    Box::new(Observe(observed.clone())),
                )]
                .into(),
                || {
                    with_game(|game| {
                        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                            "rules",
                            Some("rules"),
                            Some(1),
                            true,
                            (!saved).then_some(&create),
                            registry.clone(),
                        )
                        .unwrap();
                        request.save_root = scratch.path().to_owned();
                        let prepared = if dedicated {
                            game.prepare_world_and_rules(request).unwrap()
                        } else {
                            let loader = sunrise_gameplay::game::world_loading::WorldLoader::start(
                                request,
                                game.res.clone(),
                                game.pack.clone(),
                                game.options.clone(),
                                Arc::new(sunrise_world::datafix::progress::UpgradeProgress::new()),
                            )
                            .unwrap();
                            game.prepare_early_game_rules(loader).unwrap()
                        };
                        game.native_server_about_to_start(dedicated);
                        game.finish_open_world(prepared).unwrap();
                        game.native_server_starting(dedicated);
                        assert_rules(game);
                        let level = game.level(game.dim.id()).unwrap();
                        assert_eq!(
                            level.game_rule(game, "keep_inventory"),
                            Ok(Some(mod_api::GameRuleValue::Bool(true)))
                        );
                        assert_eq!(
                            level.game_rule(game, "random_tick_speed"),
                            Ok(Some(mod_api::GameRuleValue::Int(7)))
                        );
                        assert_eq!(
                            collect_rules(|visit| level.visit_game_rules(game, visit)),
                            collect_rules(|visit| (game as &dyn ServerHost).visit_game_rules(visit))
                        );
                    });
                },
            );
            assert_eq!(*observed.lock().unwrap(), ["about", "starting"]);
        }
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
