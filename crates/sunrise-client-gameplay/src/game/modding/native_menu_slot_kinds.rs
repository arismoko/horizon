//! Client layout roles; never inspect stacks or server container identities.
use super::Screen;
use mod_api::{MenuRef, MenuSlotKind};
use sunrise_level::containers::{ModMenuType, mod_menu_type};

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn menu_slot_kinds(&self, menu: MenuRef) -> Option<Vec<MenuSlotKind>> {
        if !matches!(self.client.screen, Screen::Inventory | Screen::Creative) {
            return None;
        }
        self.client
            .remote
            .as_ref()
            .filter(|remote| remote.has_level())?;
        let name = self.native_client_menu_name(menu)?;
        let content = mod_menu_type(name);
        let count = match content {
            Some(content) => content.slot_count(),
            None => self.menu_slot_count(menu)?,
        };
        Some(layout_kinds(name, count, content, |slot| {
            sunrise_gameplay_common::game::inventory_screen_slot(name, slot as i32)
        }))
    }
}

fn layout_kinds(
    name: &str,
    count: usize,
    content: Option<&ModMenuType>,
    layout: impl Fn(usize) -> Option<usize>,
) -> Vec<MenuSlotKind> {
    use MenuSlotKind::*;
    let mut kinds = vec![Other; count];
    if content.is_none()
        && sunrise_gameplay_common::game::remote::inventory::remote_station_kind(name).is_some()
    {
        return kinds;
    }
    for (slot, kind) in kinds.iter_mut().enumerate() {
        let internal = if let Some(content) = content {
            if slot < content.slots {
                300 + slot
            } else if slot < content.slots + 27 {
                9 + slot - content.slots
            } else {
                slot - content.slots - 27
            }
        } else {
            let Some(internal) = layout(slot) else {
                continue;
            };
            internal
        };
        *kind = if internal < 36 {
            Player(internal as u32)
        } else if let Some(content) = content {
            if content.results.contains(&slot) || content.crafting.is_some_and(|c| c.result == slot)
            {
                Result
            } else if content
                .crafting
                .is_some_and(|c| c.grid_slots().contains(&slot))
            {
                CraftingGrid
            } else if slot < content.slots {
                Storage
            } else {
                Other
            }
        } else {
            match name {
                "inventory" => match slot {
                    0 => Result,
                    1..=4 => CraftingGrid,
                    5..=8 => Player(44 - slot as u32),
                    45 => Player(40),
                    _ => Other,
                },
                "minecraft:crafting" => {
                    if slot == 0 {
                        Result
                    } else {
                        CraftingGrid
                    }
                }
                "minecraft:merchant" => {
                    if slot == 2 {
                        Result
                    } else {
                        Other
                    }
                }
                "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                    if slot == 2 { Result } else { Input }
                }
                "minecraft:brewing_stand" => Input,
                "minecraft:crafter_3x3" => CraftingGrid,
                "minecraft:generic_9x1"
                | "minecraft:generic_9x2"
                | "minecraft:generic_9x3"
                | "minecraft:generic_9x4"
                | "minecraft:generic_9x5"
                | "minecraft:generic_9x6"
                | "minecraft:generic_3x3"
                | "minecraft:shulker_box"
                | "minecraft:hopper" => Storage,
                m if m.starts_with("sunrise:mount") => {
                    if slot < 2 {
                        Equipment
                    } else {
                        Storage
                    }
                }
                _ => Other,
            }
        };
    }
    if let Some(content) = content {
        for &slot in &content.inputs {
            kinds[slot] = Input;
        }
    }
    kinds
}

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use MenuSlotKind::*;
    use mod_api::__private::game::Fields;

    const PLAYER: [MenuSlotKind; 36] = [
        Player(9),
        Player(10),
        Player(11),
        Player(12),
        Player(13),
        Player(14),
        Player(15),
        Player(16),
        Player(17),
        Player(18),
        Player(19),
        Player(20),
        Player(21),
        Player(22),
        Player(23),
        Player(24),
        Player(25),
        Player(26),
        Player(27),
        Player(28),
        Player(29),
        Player(30),
        Player(31),
        Player(32),
        Player(33),
        Player(34),
        Player(35),
        Player(0),
        Player(1),
        Player(2),
        Player(3),
        Player(4),
        Player(5),
        Player(6),
        Player(7),
        Player(8),
    ];

    #[test]
    fn layout_roles_in_menu_order() {
        for (name, own) in [
            (
                "inventory",
                vec![
                    Result,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    Player(39),
                    Player(38),
                    Player(37),
                    Player(36),
                ],
            ),
            (
                "minecraft:crafting",
                vec![
                    Result,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                    CraftingGrid,
                ],
            ),
            ("minecraft:furnace", vec![Input, Input, Result]),
            ("minecraft:blast_furnace", vec![Input, Input, Result]),
            ("minecraft:smoker", vec![Input, Input, Result]),
            ("minecraft:brewing_stand", vec![Input; 5]),
            ("minecraft:hopper", vec![Storage; 5]),
            (
                "minecraft:generic_9x3",
                vec![
                    Storage, Storage, Storage, Storage, Storage, Storage, Storage, Storage,
                    Storage, Storage, Storage, Storage, Storage, Storage, Storage, Storage,
                    Storage, Storage, Storage, Storage, Storage, Storage, Storage, Storage,
                    Storage, Storage, Storage,
                ],
            ),
            (
                "sunrise:mount1",
                vec![Equipment, Equipment, Storage, Storage, Storage],
            ),
            ("minecraft:anvil", vec![Other, Other, Other]),
        ] {
            let mut expected = own;
            expected.extend(PLAYER);
            if name == "inventory" {
                expected.push(Player(40));
            }
            if name == "minecraft:anvil" {
                expected = vec![Other; 39];
            }
            let got = layout_kinds(name, expected.len(), None, |s| {
                sunrise_gameplay_common::game::inventory_screen_slot(name, s as i32)
            });
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    fn content_declaration_roles() {
        use sunrise_level::containers::{ModCrafting, ModLinks};
        let content = ModMenuType {
            id: "test:roles",
            owner: "test",
            slots: 9,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![7],
            inputs: vec![5, 8],
            crafting: Some(ModCrafting {
                grid: 0,
                size: 2,
                result: 4,
            }),
            links: Some(ModLinks { from: 5, count: 2 }),
        };
        let mut expected = vec![
            CraftingGrid,
            CraftingGrid,
            CraftingGrid,
            CraftingGrid,
            Result,
            Input,
            Storage,
            Result,
            Input,
        ];
        expected.extend(PLAYER);
        assert_eq!(
            layout_kinds("test:roles", 45, Some(&content), |_| None),
            expected
        );
    }

    #[test]
    fn owned_processing_layout_roles() {
        let content = ModMenuType {
            id: "test:processor",
            owner: "test",
            slots: 3,
            player_inventory: true,
            data_slots: 0,
            owned_slots: Some(vec![64, 64, 64]),
            results: vec![2],
            inputs: vec![0, 1],
            crafting: None,
            links: None,
        };
        let mut expected = vec![Input, Input, Result];
        expected.extend(PLAYER);
        assert_eq!(
            layout_kinds("test:processor", 39, Some(&content), |_| None),
            expected
        );
    }

    #[test]
    #[ignore = "daily"]
    fn live_menu_validation_and_creative() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "SlotKindsTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let player = game.local_player().unwrap();
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(game.menu_slot_kinds(menu), None);
            game.client.screen = Screen::Inventory;
            let inventory = game.menu_slot_kinds(menu).unwrap();
            assert_eq!(inventory.len(), game.menu_slot_count(menu).unwrap());
            game.client.screen = Screen::Creative;
            assert_eq!(game.menu_slot_kinds(menu), Some(inventory));
            let fields = menu.fields();
            let stale =
                mod_api::__private::game::menu_ref(player, fields.window, fields.generation + 1);
            assert_eq!(game.menu_slot_kinds(stale), None);
            let other =
                mod_api::__private::game::menu_ref(player, fields.window + 1, fields.generation);
            assert_eq!(game.menu_slot_kinds(other), None);
            for (name, count) in [
                ("minecraft:crafting", 46),
                ("minecraft:furnace", 39),
                ("minecraft:generic_9x3", 63),
                ("sunrise:mount1", 41),
                ("minecraft:anvil", 39),
            ] {
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .menu
                    .set(Some((3, name.into())));
                let menu = game.player_container_menu(player).unwrap();
                let kinds = game.menu_slot_kinds(menu).unwrap();
                assert_eq!(kinds.len(), count, "{name}");
                assert_eq!(Some(kinds.len()), game.menu_slot_count(menu));
                if name == "minecraft:anvil" {
                    assert_eq!(kinds, vec![Other; 39]);
                }
            }
            game.client
                .remote
                .as_mut()
                .unwrap()
                .menu
                .set(Some((4, "test:missing".into())));
            let remote = game.client.remote.as_ref().unwrap();
            let missing = mod_api::__private::game::menu_ref(player, 4, remote.menu.generation);
            assert_eq!(game.menu_slot_kinds(missing), None);
            assert_eq!(game.menu_slot_count(missing), None);
            game.client.remote = None;
            assert_eq!(game.menu_slot_kinds(menu), None);
        });
    }
}
