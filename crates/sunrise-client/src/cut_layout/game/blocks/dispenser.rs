#![cfg(test)]
use sunrise_core::geometry::Vec3;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::geometry::Direction;
    use sunrise_gameplay::dispense_rules::{CLICK, FAIL, Slot};
    use sunrise_gameplay::game::server_side::dispenser::mod_dispensed;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::equip_answers::EquipAnswersGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_items::items::ItemStack;

    use sunrise_gameplay::game::server_side::dispenser::tests::dispense_at_guest;

    use sunrise_gameplay::game::server_side::dispenser::tests::dress;

    use super::*;

    use sunrise_sim::mobs::Mob;

    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    use sunrise_protocol::packets::configuration::ClientInformation;

    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

    #[cfg(test)]
    /// What a dispense probe part heard: which part, and the dispenser it was for.
    #[derive(Clone, Default)]
    pub(crate) struct Heard(
        pub(in crate::game) std::sync::Arc<std::sync::Mutex<Vec<(&'static str, [i32; 3], u32)>>>,
    );
    #[cfg(test)]
    impl Heard {
        pub(crate) fn record(&self, part: &'static str, pos: [i32; 3], state: u32) {
            self.0.lock().unwrap().push((part, pos, state));
        }
        pub(crate) fn take(&self) -> Vec<(&'static str, [i32; 3], u32)> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }
    #[cfg(test)]
    /// Answers for the faced block for dirt and cobblestone, whatever the item's own part says.
    pub(crate) struct Front(pub(crate) Heard);
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Front {
        fn dispense_method(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::DispenseMethodEvent<'_>,
        ) -> Option<mod_api::Dispensed> {
            let state = sunrise_gameplay_common::game::api_values::TryIntoEngine::try_into_engine(
                &event.state,
                &(),
            )
            .unwrap()
            .0;
            self.0.record("block", event.pos, state);
            let item = event.stack.item();
            let answers = ["minecraft:dirt", "minecraft:cobblestone"]
                .iter()
                .any(|name| item == mod_api::__private::item_key(name).unwrap());
            answers.then_some((None, FAIL))
        }
    }
    #[cfg(test)]
    /// Answers for the item it is attached to, if the block in front let it.
    pub(crate) struct Item(pub(crate) Heard);
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for Item {
        fn dispense_item(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::DispenseItemEvent<'_>,
        ) -> Option<mod_api::Dispensed> {
            self.0.record("item", event.pos, 0);
            Some((None, CLICK))
        }
    }
    #[cfg(test)]
    pub(crate) struct Probe(pub(crate) Heard);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Probe {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, Front(self.0.clone()));
            for item in ["minecraft:stone", "minecraft:cobblestone"] {
                let item = r.item_key(item);
                r.item_part(item, Item(self.0.clone()));
            }
        }
    }
    #[cfg(test)]
    /// Dresses only the entity type it is told to: what the part of an iron helmet answers to
    /// `can_equip`, and what it was asked as `(slot, entity, kind, position)`, the position read
    /// from the entity's handle through the read-only `Host`.
    #[derive(Clone)]
    pub(crate) struct Fits {
        pub(in crate::game) accepts: Arc<std::sync::Mutex<&'static str>>,
        pub(in crate::game) asked:
            Arc<std::sync::Mutex<Vec<(String, u32, String, Option<[f64; 3]>)>>>,
    }
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for Fits {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            let at = event.entity.entity().position(host);
            let (slot, kind) = (event.slot.to_owned(), event.kind.to_owned());
            (self.asked.lock().unwrap()).push((slot, event.entity.id(), kind, at));
            event.can_equip = event.kind == *self.accepts.lock().unwrap();
        }
    }
    #[cfg(test)]
    pub(crate) struct Tailor(pub(crate) Fits);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Tailor {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
        }
    }
    #[cfg(test)]
    impl Fits {
        pub(crate) fn new(accepts: &'static str) -> Self {
            Self {
                accepts: Arc::new(std::sync::Mutex::new(accepts)),
                asked: Arc::default(),
            }
        }
        pub(crate) fn asked(&self) -> Vec<(String, u32, String, Option<[f64; 3]>)> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }
        /// Runs `test` with a mod whose part of the iron helmet is this one installed.
        pub(crate) fn installed(&self, test: impl FnOnce()) {
            let tailor = mod_api::ModEntry::new("tailor", Box::new(Tailor(self.clone())));
            let mods: mod_api::Mods = vec![tailor].into();
            crate::entry::install_client_hooks();
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, test);
        }
    }
    /// An iron helmet whose `equippable` is removed (`fields` None) or replaced by `fields`.
    pub fn helmet_with(fields: Option<Vec<(&str, sunrise_core::nbt::Tag)>>) -> ItemStack {
        use sunrise_core::nbt::Tag;
        let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
        let (id, value) = match fields {
            Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
            None => ("!minecraft:equippable", Tag::Compound(Default::default())),
        };
        helmet.data_mut().set_other(id, Some(value));
        helmet
    }
    /// A stand of entity id `id` standing at `x`, 200, `z`.
    pub fn stand_at(id: u32, x: f64, z: f64) -> sunrise_sim::entities::ArmorStand {
        let at = sunrise_core::geometry::Vec3::new(x, 200.0, z);
        sunrise_sim::entities::ArmorStand::new(id, at, 0.0)
    }

    #[test]
    #[ignore = "daily"]
    fn a_dispenser_drops_an_egg_without_prototype_entity_data() {
        use std::{fs, path::Path};
        use {
            sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };

        fn link_fixture_source(source: &Path, target: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(source, target).unwrap();
            #[cfg(not(unix))]
            if source.is_dir() {
                fs::create_dir(target).unwrap();
                for entry in fs::read_dir(source).unwrap() {
                    let entry = entry.unwrap();
                    link_fixture_source(&entry.path(), &target.join(entry.file_name()));
                }
            } else if fs::hard_link(source, target).is_err() {
                fs::copy(source, target).unwrap();
            }
        }

        let source = fixture_root().canonicalize().unwrap();
        let root = tempfile::tempdir().unwrap();
        for entry in fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != "items.json" {
                link_fixture_source(&entry.path(), &root.path().join(entry.file_name()));
            }
        }
        let egg = "minecraft:cow_spawn_egg";
        let mut items: serde_json::Value =
            serde_json::from_slice(&fs::read(source.join("items.json")).unwrap()).unwrap();
        assert!(
            items[egg]
                .as_object_mut()
                .unwrap()
                .remove("minecraft:entity_data")
                .is_some()
        );
        fs::write(
            root.path().join("items.json"),
            serde_json::to_vec(&items).unwrap(),
        )
        .unwrap();

        with_game_at(root.path().to_owned(), |game| {
            let stack = ItemStack::new(egg, 1);
            let (left, event) = game
                .dispense_default_method((4, 64, 4), Direction::Up, stack.clone())
                .unwrap();
            assert_eq!((left, event), (None, CLICK));
            assert!(game.mobs.is_empty());
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(game.dropped[0].stack, stack);
        });
    }
    /// `IItemExtension.canEquip` at the dispenser: vanilla dresses the zombie, the first in
    /// front; a part of the iron helmet that accepts only armour stands dresses the stand
    /// instead, and one that accepts only players dresses neither. The part reads where each
    /// one stands through its `Host`. An item with no part is as vanilla, and a handle that
    /// names no live entity is refused without asking the part.
    #[test]
    #[ignore = "daily"]
    fn a_can_equip_part_decides_which_entity_a_dispenser_dresses() {
        let fits = Fits::new("minecraft:armor_stand");
        fits.installed(|| {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                // No part answers for the chestplate: the component's answer stands.
                let chestplate = ItemStack::new("minecraft:iron_chestplate", 1);
                let (_, worn, stand, _) = dress(game, 0, chestplate);
                assert_eq!((worn, stand), (Some(Slot::Chest), false));
                assert!(fits.asked().is_empty());
                // The helmet's part accepts only armour stands.
                let helmet = ItemStack::new("minecraft:iron_helmet", 1);
                let (_, worn, stand, at) = dress(game, 1, helmet.clone());
                assert_eq!((worn, stand), (None, true));
                let asked = |id, kind: &str| ("head".to_owned(), id, kind.to_owned(), Some(at));
                assert_eq!(
                    fits.asked(),
                    [
                        asked(11, "minecraft:zombie"),
                        asked(21, "minecraft:armor_stand")
                    ]
                );
                // Or only players: neither stands in front.
                *fits.accepts.lock().unwrap() = "minecraft:player";
                let (done, worn, stand, _) = dress(game, 2, helmet.clone());
                assert_eq!((done, worn, stand), (false, None, false));
                assert_eq!(fits.asked().len(), 2);
                // A missing entity is refused, and the part never hears of it.
                let prepared = game.equip_stack(&helmet);
                assert!(!game.can_equip(&prepared, "head", 9_999, "minecraft:zombie", true));
                assert!(fits.asked().is_empty());
                // With no part to ask, the component's answer stands for any entity.
                let chestplate = game.equip_stack(&ItemStack::new("minecraft:iron_chestplate", 1));
                assert!(game.can_equip(&chestplate, "chest", 9_999, "minecraft:zombie", true));
            });
        });
    }
    /// The dispenser reads the stack's own `equippable`, with no part installed: a removed
    /// component dispenses nothing onto anyone, an override for the feet dresses the zombie's
    /// feet, `allowed_entities` naming only armour stands skips the zombie for the stand, an
    /// explicitly empty list allows nobody, and `dispensable: false` keeps the dispenser out.
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_reads_the_stacks_own_equippable() {
        use sunrise_core::nbt::Tag;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let slot = |name: &str| ("slot", Tag::String(name.into()));
            let allowed = |list: Vec<Tag>| ("allowed_entities", Tag::List(list));
            let dressed = |game: &mut sunrise_gameplay::game::ServerGame<'_>, row, stack| {
                let (done, worn, stand, _) = dress(game, row, stack);
                (done, worn, stand)
            };
            assert_eq!(dressed(game, 0, helmet_with(None)), (false, None, false));
            assert_eq!(
                dressed(game, 1, helmet_with(Some(vec![slot("feet")]))),
                (true, Some(Slot::Feet), false)
            );
            let stands = vec![Tag::String("armor_stand".into())];
            assert_eq!(
                dressed(
                    game,
                    2,
                    helmet_with(Some(vec![slot("head"), allowed(stands)]))
                ),
                (true, None, true)
            );
            assert_eq!(
                dressed(
                    game,
                    3,
                    helmet_with(Some(vec![slot("head"), allowed(Vec::new())]))
                ),
                (false, None, false)
            );
            let kept = ("dispensable", Tag::Byte(0));
            assert_eq!(
                dressed(game, 4, helmet_with(Some(vec![slot("head"), kept]))),
                (false, None, false)
            );
        });
    }
    /// The LAN guest in front of a dispenser: a part that accepts only players lets the helmet
    /// on, asked for the guest's own entity at its position; one that accepts only armour stands
    /// keeps it off; and with no part, a stack whose `allowed_entities` leaves the player out is
    /// not dispensed onto them.
    #[test]
    #[ignore = "daily"]
    fn a_can_equip_part_decides_whether_a_dispenser_dresses_a_guest() {
        use sunrise_core::nbt::Tag;
        // Before any part is installed: `install_parts` outlives `installed`, and a leftover part
        // that accepts players would answer for the stack in place of its component.
        {
            let mut resources = None;
            let mut game = server_with_guest(&mut resources);
            let zombies = vec![Tag::String("zombie".into())];
            let fields = vec![
                ("slot", Tag::String("head".into())),
                ("allowed_entities", Tag::List(zombies)),
            ];
            let mut stack = helmet_with(Some(fields));
            assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .armor[0]
                    .is_none()
            );
        }
        let fits = Fits::new("minecraft:armor_stand");
        fits.installed(|| {
            let mut resources = None;
            let mut game = server_with_guest(&mut resources);
            // Open the world and admit the guest as a join does, so its handle resolves.
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            game.reset_entity_residency();
            let guest = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(2)
                .unwrap()
                .entity_id as u32;
            assert_eq!(
                dispense_at_guest(&mut game, "minecraft:iron_helmet"),
                (false, 2)
            );
            let asked = ("head".to_owned(), guest, "minecraft:player".to_owned());
            let at = Some([8.5, 200.0, 8.5]);
            assert_eq!(
                fits.asked(),
                [(asked.0.clone(), asked.1, asked.2.clone(), at)]
            );
            *fits.accepts.lock().unwrap() = "minecraft:player";
            assert_eq!(
                dispense_at_guest(&mut game, "minecraft:iron_helmet"),
                (true, 1)
            );
            assert_eq!(fits.asked(), [(asked.0, asked.1, asked.2, at)]);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn bucket_ambient_extraction_keeps_the_bound_mob_owner() {
        use std::sync::{Arc, Mutex};
        use sunrise_mod_runtime::attachments::{
            AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
        };
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "ambient_lifetime",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cod").unwrap().clone();
            for id in [70, 71, 72] {
                game.mobs
                    .push(Mob::new(id, kind.clone(), Vec3::new(0.5, 70.0, 0.5), 0.0));
            }
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
            let original = game.mobs.update(1, |mob| {
                mob.attachment_lifetime().bind(&mut values, &index, holder);
                mob.attachment_lifetime().registration().unwrap()
            });
            let key = ("test".into(), "ambient_lifetime".into());
            let before = index.lock().unwrap().snapshot(&key);
            game.mob_placed(71, &ItemStack::new("minecraft:axolotl_bucket", 1))
                .unwrap();
            assert!(game.mobs[1].extra.from_bucket);
            assert_eq!(
                game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(),
                [70, 71, 72]
            );
            let current = game
                .mobs
                .update(1, |m| m.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current));
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            game.mobs.remove(1);
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn dispenser_stack_damage_preserves_break_remainder() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut shears = ItemStack::new("minecraft:shears", 2);
            let max = game.items().get(&shears.item).unwrap().max_damage;
            shears.damage = max - 1;
            let remainder = game.hurt_and_break(shears, 1).unwrap();
            assert_eq!(remainder.count, 1);
            assert_eq!(remainder.damage, max);
        });
    }
    /// `IItemExtension.damageItem` on the wear of a tool a dispenser uses: a brush (16 points a
    /// brushing) whose part halves it loses 8, one whose part cancels it keeps its durability, and
    /// the shears, which have no part, lose the whole point.
    #[test]
    #[ignore = "daily"]
    fn a_damage_item_part_changes_the_wear_of_a_dispensed_tool() {
        use {
            crate::game::server_side::damage_item_tests::Rule,
            crate::game::server_side::damage_item_tests::with_wear_part,
        };
        with_wear_part("minecraft:brush", Rule::Halve, |game, wear| {
            let damage = |game: &mut sunrise_gameplay::game::ServerGame<'_>, item: &str, points| {
                game.hurt_and_break(ItemStack::new(item, 1), points)
                    .unwrap()
                    .damage
            };
            assert_eq!(damage(game, "minecraft:brush", 16), 8);
            assert_eq!(wear.take_asked(), [16]);
            wear.set(Rule::Cancel);
            assert_eq!(damage(game, "minecraft:brush", 16), 0);
            assert_eq!(wear.take_asked(), [16]);
            assert_eq!(damage(game, "minecraft:shears", 1), 1);
            assert!(wear.take_asked().is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn sulfur_cube_dispenser_replacement_drops_the_full_old_stack() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let mut mob = Mob::new(
                12,
                kind,
                sunrise_core::geometry::Vec3::new(0.5, 0.0, 0.5),
                0.0,
            );
            let mut old = ItemStack::new("minecraft:stone", 3);
            old.damage = 7;
            old.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("kept".into()));
            mob.extra.set_body_stack(Some(old.clone()));
            game.mobs.push(mob);
            let incoming = ItemStack::new("minecraft:dirt", 5);
            assert!(game.sulfur_cube_swallow((0, 0, 0), &incoming));
            assert!(game.dropped.iter().any(|drop| drop.stack == old));
            assert_eq!(
                game.mobs[0].equipment_stacks()[6],
                Some(ItemStack::new("minecraft:dirt", 1))
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mod_dispenser_answers_preserve_fallback_and_own_consumption() {
        use sunrise_gameplay_common::game::api_values::{StackReadError, TryIntoApi};
        super::super::native_entity_refs::tests::with_game(|game| {
            assert_eq!(
                mod_dispensed((None, CLICK), game.items()),
                Ok((None, CLICK))
            );
            let stack = ItemStack::new("minecraft:stone", 7);
            let Ok((Some(left), event)) =
                mod_dispensed((Some(stack.try_into_api().unwrap()), FAIL), game.items())
            else {
                panic!("handled stack")
            };
            assert_eq!(left, stack, "the returned remainder is not consumed again");
            assert_eq!(event, FAIL, "the answer chooses its sound");
            // An empty engine value is refused before a mod answer can be constructed.
            assert_eq!(
                ItemStack { count: 0, ..stack }.try_into_api(),
                Err(StackReadError::Empty)
            );
            assert_eq!(mod_dispensed((None, 0), game.items()), Ok((None, 0)));
        });
    }
    /// A server with no player of its own, an open hub and one guest (conn 2)
    /// standing at (8.5, 200, 8.5) on a stone floor.
    pub(crate) fn server_with_guest(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_gameplay::game::ServerGame<'_> {
        let mut game = super::super::game_tests::tests::server_game(resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "dispenser-equip-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 208,
                name: "Wearer".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [8.5, 200.0, 8.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        game
    }
    /// `EquipmentDispenseItemBehavior`: a server's player, the integrated
    /// owner or a guest, takes the piece (a server has no player of its own).
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_equips_a_server_player_in_front() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        assert!(game.player.is_none(), "the server thread has no player");
        // The guest's feet are in block (8, 200, 8): the dispenser at y 201
        // facing down targets the block under it.
        let mut stack = ItemStack::new("minecraft:carved_pumpkin", 3);
        assert!(game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert_eq!(stack.count, 2);
        let guest = game.server.lan.as_ref().unwrap().hub.guest(2).unwrap();
        assert_eq!(
            guest.armor[0].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:carved_pumpkin", 1))
        );
        // The head is taken now: a second pumpkin is not dispensed onto them.
        assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert_eq!(stack.count, 2);
        // A boots piece still has its own empty slot.
        let (done, left) = dispense_at_guest(&mut game, "minecraft:iron_boots");
        assert_eq!((done, left), (true, 1), "boots go on the free feet slot");
        let guest = game.server.lan.as_ref().unwrap().hub.guest(2).unwrap();
        assert!(guest.armor[3].is_some() && guest.armor[1].is_none());
    }
    /// A spectator is not a target (`canEquipWithDispenser`: `!isSpectator()`).
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_skips_a_spectator_and_a_dead_player() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        for spectate in [true, false] {
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.game_mode = if spectate { 3 } else { 0 };
                g.dead = !spectate;
            });
            let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
            assert_eq!(stack.count, 1);
        }
    }
    /// `ArmorStand.canUseSlot` through `isDisabled`: the first stand in front
    /// that can wear the piece gets it. Bit `1 << filterBit` locks a slot
    /// (head 4, chest 3, legs 2, feet 1); a hand needs arms.
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_skips_an_armor_stand_whose_slot_is_locked() {
        use sunrise_sim::entities::ArmorStand;
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        game.server.lan.as_mut().unwrap().hub.guests.clear();
        let at = sunrise_core::geometry::Vec3::new(8.5, 200.0, 8.5);
        let mut stands = [(1, 1u32 << 4), (2, 0)].map(|(id, locked)| {
            let mut s = ArmorStand::new(id, at, 0.0);
            s.disabled_slots = locked as i32;
            s
        });
        stands[0].show_arms = true;
        game.armor_stands.clear();
        game.armor_stands.extend(stands.iter().cloned());
        // The head-locked first stand is skipped for a helmet; the second wears it.
        let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
        assert!(game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert!(game.armor_stands[0].equipment[5].is_none());
        assert!(game.armor_stands[1].equipment[5].is_some());
        // The same stand takes chest armour: only its head is locked.
        let mut stack = ItemStack::new("minecraft:iron_chestplate", 1);
        assert!(game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert!(game.armor_stands[0].equipment[4].is_some());
        // Every stand locked for that slot: nothing takes it (the dispenser ejects it).
        game.armor_stands[0].disabled_slots = 1 << 4;
        game.armor_stands[1].disabled_slots = 1 << 4;
        game.armor_stands[1].equipment[5] = None;
        let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
        assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert_eq!(stack.count, 1);
    }
    /// A guest whose client is still on its loading screen (joined or
    /// respawned, no ServerboundPlayerLoadedPacket yet) is in the level all the
    /// same: `getEntitiesOfClass` finds it and it wears the piece.
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_equips_a_player_whose_client_is_still_loading() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            (g.loaded, g.loading) = (false, true);
        });
        let (done, left) = dispense_at_guest(&mut game, "minecraft:iron_helmet");
        assert_eq!((done, left), (true, 1));
        let guest = game.server.lan.as_ref().unwrap().hub.guest(2).unwrap();
        let head = guest.armor[0].as_ref().map(|s| &*s.item);
        assert_eq!(head, Some("minecraft:iron_helmet"));
    }
    /// Players are entities of the level: in one section they go in join
    /// order (entity ids) with the mobs and stands, not after them.
    #[test]
    #[ignore = "daily"]
    fn a_player_and_a_stand_in_front_go_in_join_order() {
        // (stand id, guest id) -> who wears the helmet: the lower id joined first.
        for (stand_id, guest_id, guest_first) in [(7, 5, true), (3, 5, false)] {
            let mut resources = None;
            let mut game = server_with_guest(&mut resources);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.entity_id = guest_id);
            game.armor_stands.clear();
            game.armor_stands.extend(vec![stand_at(stand_id, 8.5, 8.5)]);
            let (done, _) = dispense_at_guest(&mut game, "minecraft:iron_helmet");
            assert!(done);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(2).unwrap();
            assert_eq!(
                guest.armor[0].is_some(),
                guest_first,
                "stand {stand_id}, guest {guest_id}"
            );
            assert_eq!(game.armor_stands[0].equipment[5].is_some(), !guest_first);
        }
    }
    /// `forEachAccessibleNonEmptySection` walks sections before join order:
    /// a stand whose feet are in section x 1 but whose box reaches into block
    /// x 15 comes after one in section x 0, even with the lower id.
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_walks_entity_sections_before_ids() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        game.server.lan.as_mut().unwrap().hub.guests.clear();
        // Stand boxes are 0.5 wide: x 16.2 spans 15.95..16.45.
        game.armor_stands.clear();
        game.armor_stands
            .extend(vec![stand_at(1, 16.2, 8.5), stand_at(2, 15.5, 8.5)]);
        let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
        assert!(game.dispense_equipment((15, 201, 8), Direction::Down, &mut stack));
        assert!(game.armor_stands[0].equipment[5].is_none());
        assert!(game.armor_stands[1].equipment[5].is_some());
    }
    /// `DispenserBlock.getDispenseMethod`: the block the dispenser faces
    /// answers first and wins for any item; with no answer from it the item's
    /// part decides; with neither, vanilla drops one item as before.
    #[test]
    #[ignore = "daily"]
    fn the_faced_block_answers_before_the_item_and_vanilla_stands_when_no_part_listens() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        let heard = Heard::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "dispense_probe",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let air = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .unwrap();
                game.world.set(6, 64, 5, stone).unwrap();
                let dispense = |game: &mut sunrise_gameplay::game::ServerGame<'_>, item: &str| {
                    sunrise_gameplay::game::server_side::dispense_fixture::start_recording();
                    let out = game
                        .dispense_item((5, 64, 5), Direction::East, ItemStack::new(item, 1))
                        .unwrap()
                        .continue_value()
                        .unwrap();
                    let events =
                        sunrise_gameplay::game::server_side::dispense_fixture::stop_recording();
                    (out, events)
                };

                let (out, events) = dispense(game, "minecraft:dirt");
                assert_eq!(out, None);
                assert_eq!(heard.take(), [("block", [5, 64, 5], stone.0)]);
                assert!(events.iter().any(|e| e == "L 1001 5,64,5 0"), "{events:?}");
                assert!(game.dropped.is_empty());

                let (out, events) = dispense(game, "minecraft:cobblestone");
                assert_eq!(out, None);
                assert_eq!(heard.take(), [("block", [5, 64, 5], stone.0)]);
                assert!(events.iter().any(|e| e == "L 1001 5,64,5 0"), "{events:?}");
                assert!(game.dropped.is_empty());

                let (out, events) = dispense(game, "minecraft:stone");
                assert_eq!(out, None);
                assert_eq!(
                    heard.take(),
                    [("block", [5, 64, 5], stone.0), ("item", [5, 64, 5], 0)]
                );
                assert!(events.iter().any(|e| e == "L 1000 5,64,5 0"), "{events:?}");
                assert!(game.dropped.is_empty());

                game.world.set(6, 64, 5, air).unwrap();
                let (out, events) = dispense(game, "minecraft:stick");
                assert_eq!(out, None);
                assert!(heard.take().is_empty());
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].stack, ItemStack::new("minecraft:stick", 1));
                assert!(events.iter().any(|e| e == "L 1000 5,64,5 0"), "{events:?}");
            });
        });
    }
}

#[cfg(test)]
#[path = "dispenser/decision_tests.rs"]
pub(crate) mod decision_tests;
