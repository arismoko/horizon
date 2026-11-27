use crate::game::native_hooks::build_creative_mode_tab_contents::tests::Edit;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::Observe;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::Seen;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::book_level;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::replace_client_definition;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::stack;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::stone_key;
use crate::game::native_hooks::build_creative_mode_tab_contents::tests::*;
use mod_api::LogicalSide;
use mod_api::ModEntry;
use mod_api::prelude::ItemsHost;
use mod_api::{CreativeModeTabs, CreativeTabPermissions, CreativeTabReadError};
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
};

#[test]
#[ignore = "daily"]
fn creative_catalog_builds_lazily_and_edits_before_filtering_on_both_sides() {
    use sunrise_client::creative::Creative;
    use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
    use sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::GameHooks as _;
    let row = |item: &str, visibility: &str| {
        serde_json::json!({
            "kind":"stack", "snbt":format!(r#"{{id:"{item}",count:1}}"#), "visibility":visibility,
        })
    };
    let both = "PARENT_AND_SEARCH_TABS";
    let catalog = serde_json::json!({
        "schema":4, "item_features":{"minecraft:barrier":["test:disabled"]}, "potion_features":{},
        "tabs":[
            {"id":"test:a","title":"test:a","type":"CATEGORY","build":[
                row("minecraft:stone", "PARENT_TAB_ONLY"), row("minecraft:stick", "SEARCH_TAB_ONLY"),
                row("minecraft:gravel", both), row("minecraft:barrier", both), row("minecraft:clay", both)
            ]},
            {"id":"minecraft:hotbar","title":"minecraft:hotbar","type":"HOTBAR","build":[]},
            {"id":"test:b","title":"test:b","type":"CATEGORY","build":[row("minecraft:sand", both)],"operator_build":[row("minecraft:glass", both)]},
            {"id":"test:holders","title":"test:holders","type":"CATEGORY","build":[{"kind":"enchantments","all_levels":false,"visibility":both}]},
            {"id":"minecraft:search","title":"minecraft:search","type":"SEARCH","build":[]},
            {"id":"minecraft:inventory","title":"minecraft:inventory","type":"INVENTORY","build":[]},
        ]
    });
    let seen = Arc::new(Mutex::new(Seen::default()));
    let setups = vec![
        ModEntry::new("first_edit", Box::new(Edit(seen.clone()))),
        ModEntry::new("later_edit", Box::new(Observe)),
    ]
    .into();
    fn check<'g, G>(
        game: &mut G,
        side: LogicalSide,
        seen: &Arc<Mutex<Seen>>,
        set_features: impl Fn(&mut G, [String; 2]),
        mut before_build: impl FnMut(&mut G),
        mut after_build: impl FnMut(&mut G),
    ) -> Arc<CreativeModeTabs>
    where
        G: sunrise_gameplay_common::game::SideGame<'g> + mod_api::Host + ItemsHost,
    {
        let flags = |game: &mut G, ids: [&str; 2]| {
            set_features(game, ids.map(str::to_owned));
        };
        flags(game, ["test:alpha", "test:beta"]);
        before_build(game);
        assert!(seen.lock().unwrap().events.is_empty());
        {
            // A plain read before any build answers nothing and builds nothing.
            let reader: &dyn mod_api::Host = game;
            assert_eq!(reader.built_creative_tabs(), Ok(None));
            assert!(seen.lock().unwrap().events.is_empty());
        }
        sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::GameHooks::build_creative(game, CreativeTabPermissions::Normal).unwrap();
        let built_events = seen.lock().unwrap().events.len();
        let reader: &dyn mod_api::Host = game;
        let normal = reader.built_creative_tabs().unwrap().unwrap();
        assert_eq!(normal.permissions, CreativeTabPermissions::Normal);
        let cached = ItemsHost::creative_tabs(game, CreativeTabPermissions::Normal).unwrap();
        assert!(Arc::ptr_eq(&normal, &cached));
        assert_eq!(seen.lock().unwrap().events.len(), built_events);
        after_build(game);
        let a = normal.tab("test:a").unwrap();
        assert_eq!(
            a.display_items,
            [
                stack("minecraft:dirt", 1),
                stack("minecraft:stone", 1),
                stack("minecraft:stick", 1)
            ]
        );
        assert_eq!(
            a.search_items,
            [stack("minecraft:torch", 1), stack("minecraft:stone", 1)]
        );
        flags(game, ["test:beta", "test:alpha"]);
        let host: &mut dyn mod_api::Host = game;
        assert!(!a.contains(host, &stack("minecraft:dirt", 1)));
        assert!(a.contains(host, &stack("minecraft:torch", 8)));
        assert_eq!(
            normal
                .tabs_showing(host, &stack("minecraft:dirt", 1))
                .next()
                .unwrap()
                .id,
            "test:a"
        );
        assert!(
            normal
                .tabs_showing(host, &stack("minecraft:torch", 1))
                .next()
                .is_none()
        );
        assert!(a.contains(host, &stone_key(64)));
        assert_eq!(
            normal.tabs_showing(host, &stone_key(64)).next().unwrap().id,
            "test:a"
        );
        assert!(!a.contains(host, &stone_key(32)));
        assert!(normal.tabs_showing(host, &stone_key(32)).next().is_none());
        let cached = host.creative_tabs(CreativeTabPermissions::Normal).unwrap();
        assert!(Arc::ptr_eq(&normal, &cached));
        let operator = host
            .creative_tabs(CreativeTabPermissions::Operator)
            .unwrap();
        assert!(!Arc::ptr_eq(&normal, &operator));
        assert_eq!(
            operator.tab("minecraft:search").unwrap().display_items[..3],
            [
                stack("minecraft:torch", 1),
                stack("minecraft:stone", 1),
                stack("minecraft:glass", 1)
            ]
        );
        assert_eq!(
            normal.tab("test:b").unwrap().display_items,
            [stack("minecraft:sand", 1)]
        );
        let actual = std::mem::take(&mut seen.lock().unwrap().events);
        let expected: Vec<_> = [
            CreativeTabPermissions::Normal,
            CreativeTabPermissions::Operator,
        ]
        .into_iter()
        .flat_map(|permissions| {
            [
                "test:a",
                "test:b",
                "test:holders",
                "minecraft:hotbar",
                "minecraft:search",
                "minecraft:inventory",
            ]
            .map(|tab| (side, permissions, tab.to_owned()))
        })
        .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            mod_api::__private::poisoned_listeners(mod_api::HookId::BuildCreativeModeTabContents),
            0
        );

        assert_eq!(book_level(&normal), 2);
        normal
    }
    fn check_replacement<'g, G>(game: &mut G, old: &CreativeModeTabs, seen: &Arc<Mutex<Seen>>)
    where
        G: sunrise_gameplay_common::game::SideGame<'g> + ItemsHost,
    {
        // Registry installation must not dispatch; the next demand uses the changed definition.
        assert!(seen.lock().unwrap().events.is_empty());
        let fresh = ItemsHost::creative_tabs(game, CreativeTabPermissions::Normal).unwrap();
        assert_eq!(book_level(&fresh), 4);
        assert_eq!(book_level(old), 2);
        assert_eq!(seen.lock().unwrap().events.len(), 6);
        seen.lock().unwrap().events.clear();
    }
    let run = || {
        mod_platform::testing::with_mods(setups, || {
            let mut dedicated = None;
            with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
                install_dedicated_holders(game, 2);
                let creative = Creative::from_json(&catalog).unwrap();
                game.test_set_creative_catalog(creative.catalog().clone());
                assert!(game.creative_contents.published.is_none());
                dedicated = Some(check(
                    game,
                    LogicalSide::Server,
                    &seen,
                    |game, flags| game.level.enabled_features = flags.into(),
                    |game| {
                        assert_eq!(
                            creative.stock_order(&game.stock_creative_lists()).of(
                                "minecraft:stick",
                                sunrise_items::items::ItemStack::new("minecraft:stick", 1)
                                    .merge_kind()
                            ),
                            Some(0)
                        );
                    },
                    |game| {
                        assert_eq!(
                            creative.stock_order(&game.stock_creative_lists()).of(
                                "minecraft:torch",
                                sunrise_items::items::ItemStack::new("minecraft:torch", 1)
                                    .merge_kind()
                            ),
                            Some(0)
                        );
                    },
                ));
            });
            with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
                install_dedicated_holders(game, 4);
                game.test_set_creative_catalog(
                    Creative::from_json(&catalog).unwrap().catalog().clone(),
                );
                check_replacement(game, dedicated.as_deref().unwrap(), &seen);
            });
            with_client_game(|game| {
                let _server = logged_client(game);
                game.native_lifecycle.begin_open(None);
                replace_client_definition(game, 2);
                game.client.creative_screen.0 = Creative::from_json(&catalog).unwrap();
                assert!(!game.client.creative_screen.0.has_contents());
                let old = check(
                    game,
                    LogicalSide::Client,
                    &seen,
                    |game, flags| {
                        if let Some(remote) = &mut game.client.remote {
                            remote.features = flags.into();
                        } else {
                            game.level.enabled_features = flags.into();
                        }
                    },
                    |game| {
                        assert_eq!(
                            game.stock_sort_order().of(
                                "minecraft:stick",
                                sunrise_items::items::ItemStack::new("minecraft:stick", 1)
                                    .merge_kind()
                            ),
                            Some(0)
                        );
                    },
                    |game| {
                        assert_eq!(
                            game.stock_sort_order().of(
                                "minecraft:torch",
                                sunrise_items::items::ItemStack::new("minecraft:torch", 1)
                                    .merge_kind()
                            ),
                            Some(0)
                        );
                    },
                );
                replace_client_definition(game, 4);
                check_replacement(game, &old, &seen);
                game.native_lifecycle.begin_close();
                game.native_lifecycle_world_closed();
                assert!(game.native_lifecycle.epoch().is_none());
                assert!(game.creative_contents.published.is_some());
                let host: &dyn mod_api::Host = game;
                assert_eq!(
                    host.built_creative_tabs(),
                    Err(CreativeTabReadError::Unavailable)
                );
            });
        })
    };
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::native_hooks_client::build_creative_mode_tab_contents::tests::creative_catalog_builds_lazily_and_edits_before_filtering_on_both_sides",
        run,
    );
}

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
