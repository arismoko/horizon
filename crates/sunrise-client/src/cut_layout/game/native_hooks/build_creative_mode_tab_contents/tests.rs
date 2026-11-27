#[cfg(feature = "client")]
use mod_api::Compound;
use mod_api::CreativeTabPermissions;
#[cfg(feature = "client")]
use mod_api::LogicalSide;
use mod_api::Tag;
use mod_api::prelude::ItemsHost;
#[cfg(feature = "client")]
use mod_api::{BuildCreativeModeTabContentsEvent, Hooks, HostSide};
#[cfg(feature = "client")]
use mod_api::{
    CreativeModeTabs, CreativeTabEditError, CreativeTabList, CreativeTabPlacement,
    CreativeTabReadError, CreativeTabVisibility,
};
use std::sync::Arc;
#[cfg(feature = "client")]
use std::sync::Mutex;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
#[cfg(test)]
use sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::snapshot;
use sunrise_items::creative_catalog::Catalog;
use {
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
};

#[cfg(feature = "client")]
#[derive(Default)]
pub(crate) struct Seen {
    pub(in crate::game) events: Vec<(LogicalSide, CreativeTabPermissions, String)>,
}

#[cfg(feature = "client")]
pub(crate) struct Edit(pub(crate) Arc<Mutex<Seen>>);

pub(crate) fn stack(id: &str, count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(mod_api::__private::item_key(id).unwrap(), count).unwrap()
}

#[cfg(feature = "client")]
pub(crate) fn stone_key(max_stack: i32) -> mod_api::ItemStack {
    let mut stone = stack("minecraft:stone", 42);
    stone
        .set_components(Compound::from([(
            "minecraft:max_stack_size".into(),
            Tag::Int(max_stack),
        )]))
        .unwrap();
    stone
}

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Edit {
    fn build_creative_mode_tab_contents(
        &self,
        host: HostSide,
        event: &mut BuildCreativeModeTabContentsEvent<'_>,
    ) {
        let (host, side): (&mut dyn mod_api::Host, _) = match host {
            HostSide::Client(host) => (host, LogicalSide::Client),
            HostSide::Server(host) => (host, LogicalSide::Server),
        };
        {
            let mut seen = self.0.lock().unwrap();
            seen.events
                .push((side, event.permissions, event.tab.to_owned()));
        }
        if event.tab != "test:a" {
            return;
        }
        let previous = host.creative_tabs(event.permissions).unwrap();
        let b = &previous.tab("test:b").unwrap().display_items;
        match event.permissions {
            CreativeTabPermissions::Normal => assert!(b.is_empty()),
            CreativeTabPermissions::Operator => {
                assert_eq!(b, &[stack("minecraft:sand", 1)]);
                let published = host.built_creative_tabs().unwrap().unwrap();
                assert_eq!(published.permissions, CreativeTabPermissions::Normal);
                assert_eq!(
                    published.tab("test:b").unwrap().display_items,
                    [stack("minecraft:sand", 1)]
                );
            }
        }
        let other = match event.permissions {
            CreativeTabPermissions::Normal => CreativeTabPermissions::Operator,
            CreativeTabPermissions::Operator => CreativeTabPermissions::Normal,
        };
        assert!(matches!(
            host.creative_tabs(other),
            Err(CreativeTabReadError::Reentrant)
        ));
        assert_eq!(
            event.output.insert(
                CreativeTabPlacement::Last,
                stack("minecraft:dirt", 2),
                CreativeTabVisibility::Both
            ),
            Err(CreativeTabEditError::Count),
        );
        assert_eq!(
            event.output.insert(
                CreativeTabPlacement::Before(stone_key(64)),
                stack("minecraft:dirt", 1),
                CreativeTabVisibility::Both
            ),
            Err(CreativeTabEditError::MissingAnchor(CreativeTabList::Search)),
        );
        let mut absent = stack("minecraft:iron_sword", 50);
        absent.set_damage(1000);
        // As in NeoForge, a missing anchor is refused before checking for a duplicate.
        assert_eq!(
            event.output.insert(
                CreativeTabPlacement::After(absent.clone()),
                stack("minecraft:stone", 1),
                CreativeTabVisibility::Parent,
            ),
            Err(CreativeTabEditError::MissingAnchor(CreativeTabList::Parent))
        );
        event
            .output
            .remove(&absent, CreativeTabVisibility::Both)
            .unwrap();
        absent.set_damage(u32::MAX);
        assert_eq!(
            event.output.remove(&absent, CreativeTabVisibility::Both),
            Err(CreativeTabEditError::Stack(
                mod_api::StackWriteError::InvalidComponents {
                    reason: mod_api::StackError::InvalidDamage,
                },
            )),
        );
        assert_eq!(
            event.output.insert(
                CreativeTabPlacement::Last,
                stack("minecraft:stick", 1),
                CreativeTabVisibility::Both,
            ),
            Err(CreativeTabEditError::Duplicate(CreativeTabList::Search))
        );
        // The disabled barrier still anchors the insertion during the event.
        event
            .output
            .insert(
                CreativeTabPlacement::Before(stack("minecraft:barrier", 2)),
                stack("minecraft:torch", 1),
                CreativeTabVisibility::Search,
            )
            .unwrap();
        event
            .output
            .remove(&stack("minecraft:stick", 47), CreativeTabVisibility::Search)
            .unwrap();
    }
}

#[cfg(feature = "client")]
pub(crate) struct Observe;

#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Observe {
    fn build_creative_mode_tab_contents(
        &self,
        _host: HostSide,
        event: &mut BuildCreativeModeTabContentsEvent<'_>,
    ) {
        if event.tab != "test:a" {
            return;
        }
        let mut parent = Vec::new();
        event
            .output
            .entries(CreativeTabList::Parent, &mut |entry| {
                parent.push(entry.project().unwrap());
            })
            .unwrap();
        assert_eq!(
            parent,
            [
                stack("minecraft:dirt", 1),
                stack("minecraft:stone", 1),
                stack("minecraft:gravel", 1),
                stack("minecraft:barrier", 1),
                stack("minecraft:clay", 1),
                stack("minecraft:stick", 1),
            ]
        );
        let mut search = Vec::new();
        event
            .output
            .entries(CreativeTabList::Search, &mut |entry| {
                search.push(entry.item());
            })
            .unwrap();
        assert_eq!(
            search,
            [
                "minecraft:gravel",
                "minecraft:torch",
                "minecraft:barrier",
                "minecraft:clay"
            ]
            .map(|id| stack(id, 1).item())
        );
        let mut stone = stone_key(64);
        stone.set_count(1).unwrap();
        event
            .output
            .insert(
                CreativeTabPlacement::Last,
                stone,
                CreativeTabVisibility::Search,
            )
            .unwrap();

        let gravel = stack("minecraft:gravel", 1).item();
        let clay = stack("minecraft:clay", 1).item();
        let mut visited = Vec::new();
        event
            .output
            .remove_if(CreativeTabVisibility::Both, &mut |entry| {
                visited.push(entry.item());
                entry.item() == gravel || entry.item() == clay
            })
            .unwrap();
        assert_eq!(
            visited,
            [
                "minecraft:dirt",
                "minecraft:stone",
                "minecraft:gravel",
                "minecraft:barrier",
                "minecraft:clay",
                "minecraft:stick",
                "minecraft:gravel",
                "minecraft:torch",
                "minecraft:barrier",
                "minecraft:clay",
                "minecraft:stone",
            ]
            .map(|id| stack(id, 1).item())
        );
    }
}

pub(crate) fn definition(max_level: i32) -> serde_json::Value {
    serde_json::json!({
        "description": {"text":"Fixture levels"}, "supported_items":"minecraft:iron_sword",
        "weight": 1, "anvil_cost": 1, "max_level": max_level,
        "min_cost": {"base": 1,"per_level_above_first": 1},
        "max_cost": {"base": 10,"per_level_above_first": 1}, "slots": ["mainhand"], "effects": {},
    })
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::native_hooks_server::build_creative_mode_tab_contents::tests::install_dedicated_holders;

#[cfg(feature = "client")]
pub(crate) fn replace_client_definition(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    max_level: i32,
) {
    let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
    registries.registries.insert(
        "minecraft:enchantment".into(),
        vec![(
            "test:levels".into(),
            Some(sunrise_game_data::scoreboard::json_to_tag(&definition(
                max_level,
            ))),
        )],
    );
    game.set_registries(Arc::new(registries)).unwrap();
}

#[cfg(feature = "client")]
pub(crate) fn book_level(snapshot: &CreativeModeTabs) -> i32 {
    let books = &snapshot.tab("test:holders").unwrap().display_items;
    let levels: Vec<_> = books
        .iter()
        .filter_map(|book| {
            let Tag::Int(level) = book
                .components()?
                .as_nbt()
                .get(&"minecraft:stored_enchantments".into())?
                .get("test:levels")?
            else {
                return None;
            };
            Some(*level)
        })
        .collect();
    let [level] = levels.as_slice() else {
        panic!("expected one highest-level fixture book, got {levels:?}")
    };
    *level
}

#[test]
fn a_tab_reads_back_its_title_as_a_translatable_with_the_title_as_fallback() {
    let catalog = Catalog::parse(&serde_json::json!({
        "schema":4, "item_features":{}, "potion_features":{},
        "tabs":[
            {"id":"minecraft:building_blocks","title":"Building Blocks","type":"CATEGORY","build":[]},
            {"id":"ruby:gems","title":"Ruby Gems","type":"CATEGORY","build":[]},
        ]
    }))
    .unwrap();
    let lists = [Arc::default(), Arc::default()];
    let tabs = snapshot(&catalog, CreativeTabPermissions::Normal, &lists);
    let name = |tab: &str| match &tabs.tab(tab).unwrap().display_name.content {
        mod_api::text::TextContent::Translatable {
            key,
            fallback,
            arguments,
        } => (key.clone(), fallback.clone(), arguments.is_empty()),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        name("minecraft:building_blocks"),
        (
            "itemGroup.buildingBlocks".to_owned(),
            Some("Building Blocks".to_owned()),
            true
        )
    );
    assert_eq!(
        name("ruby:gems"),
        (
            "itemGroup.ruby.gems".to_owned(),
            Some("Ruby Gems".to_owned()),
            true
        )
    );
}

/// A dedicated server's `creative_tabs` builds vanilla's tabs from the prepared catalog: the
/// building blocks tab starts with oak log, as the client's creative screen shows it.
#[test]
#[ignore = "daily"]
fn a_servers_creative_tabs_hold_vanillas_items() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
            use sunrise_sim::save::{Level, write_dat};

            let save = tempfile::tempdir().unwrap();
            write_dat(
                &save.path().join("level.dat"),
                Tag::compound([(
                    "Data",
                    Tag::compound([("DataVersion", Tag::Int(sunrise_core::version::DATA_VERSION))]),
                )]),
            )
            .unwrap();
            game.level = Level::read(save.path()).unwrap();
            install_dedicated_holders(game, 2);
            for permissions in [
                CreativeTabPermissions::Normal,
                CreativeTabPermissions::Operator,
            ] {
                let tabs = ItemsHost::creative_tabs(game, permissions).unwrap();
                let building = tabs.tab("minecraft:building_blocks").unwrap();
                let oak_log = stack("minecraft:oak_log", 1);
                assert_eq!(building.display_items.first(), Some(&oak_log));
                assert!(building.search_items.contains(&oak_log));
                assert!(
                    !tabs
                        .tab("minecraft:natural_blocks")
                        .unwrap()
                        .display_items
                        .is_empty()
                );
                assert!(
                    !tabs
                        .tab("minecraft:search")
                        .unwrap()
                        .display_items
                        .is_empty()
                );
            }
        });
    });
}
