//! Game modes beyond survival and creative (`GameType`): adventure players
//! cannot build, spectators fly through blocks and only watch.
#[allow(unused_imports)]
use super::Screen;
use sunrise_command::args::GameMode;
use sunrise_level::collisions::SceneCollisions;
use sunrise_world::world_movement::in_range_of_ground;

#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::game_mode_common::{
    inventory_after_infinite_materials, loads_chunks, mode_from_id, projectile_predicate_supported,
    relative,
};

impl crate::game::ServerGame<'_> {
    pub fn set_mode_id(&mut self, id: i32) {
        self.set_mode(mode_from_id(id));
    }
    /// `ServerPlayer.setGameMode` on a local/server world.
    pub fn set_mode(&mut self, mode: GameMode) {
        debug_assert!(
            self.is_logical_server(),
            "set_mode is server-only; remote clients use set_local_mode"
        );
        let changed = mode != self.common.mode;
        if changed {
            self.common.previous_mode = Some(self.common.mode);
        }
        self.common.mode = mode;
        self.common.creative = mode == GameMode::Creative;
        // GameType.updatePlayerAbilities applies at once.
        if let Some(a) = self.server.acting_abilities.as_mut() {
            a.update_for_mode(mode.id());
        }
        if let Some(p) = self.common.player.as_mut() {
            p.invulnerable = mode == GameMode::Creative || mode == GameMode::Spectator;
            p.may_fly = p.invulnerable;
        }
        if mode == GameMode::Spectator {
            self.common.mining = None;
            self.stop_active_item_use();
            self.server_set_acting_attack(false);
            self.dismount_for_spectator();
            if let Some(p) = self.common.player.as_mut() {
                p.flying = true;
                p.spectator = true;
                p.local_spectator = true;
            }
        } else if let Some(p) = self.common.player.as_mut() {
            p.spectator = false;
            p.local_spectator = false;
            if !self.common.creative {
                p.flying = false;
            }
        }
        // ServerPlayerGameMode.changeGameModeForPlayer: a player left flying
        // by the change lands when the ground is within a block.
        let flying = self
            .common
            .player
            .as_ref()
            .filter(|p| p.flying)
            .map(|p| p.bounds());
        if changed
            && mode != GameMode::Spectator
            && let Some(bounds) = flying
        {
            let mut scene = SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            };
            if in_range_of_ground(&mut scene, bounds)
                && let Some(p) = self.common.player.as_mut()
            {
                p.flying = false;
                if let Some(a) = self.server.acting_abilities.as_mut() {
                    a.flying = false;
                }
            }
        }
    }
    /// `ServerPlayer.setGameMode(SPECTATOR)`: `stopRiding`.
    pub(in crate::game::server_side) fn dismount_for_spectator(&mut self) {
        let Some(id) = self.common.riding else {
            return;
        };
        if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
            self.dismount(i);
        } else if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
            self.dismount_cart(i);
        } else {
            self.common.riding = None;
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    #[test]
    fn face_offsets_wrap_at_integer_boundaries() {
        use sunrise_core::geometry::Direction::*;
        for (face, axis, positive) in [
            (East, 0, true),
            (West, 0, false),
            (Up, 1, true),
            (Down, 1, false),
            (South, 2, true),
            (North, 2, false),
        ] {
            for boundary in [i32::MIN, i32::MAX] {
                let mut pos = [5; 3];
                pos[axis] = boundary;
                let mut expected = pos;
                expected[axis] = match (positive, boundary) {
                    (true, i32::MAX) => i32::MIN,
                    (false, i32::MIN) => i32::MAX,
                    (true, _) => i32::MIN + 1,
                    (false, _) => i32::MAX - 1,
                };
                assert_eq!(
                    super::relative(pos, face),
                    expected,
                    "{face:?} at {boundary}"
                );
            }
        }
    }

    use super::{
        Screen, inventory_after_infinite_materials, loads_chunks, projectile_predicate_supported,
    };

    // Added after the frozen old Game proof: unsupported conjuncts must not
    // become permission from their otherwise matching blocks portion.
    #[test]
    fn projectile_predicates_require_a_complete_supported_match() {
        for extra in [
            "nbt:{marker:1}",
            "components:{}",
            "predicates:{}",
            "state:{age:{min:1}}",
        ] {
            let text = format!("{{blocks:\"minecraft:chorus_flower\",{extra}}}");
            let (tag, _) = sunrise_core::nbt::parse_snbt(&text).unwrap();
            assert!(!projectile_predicate_supported(&tag), "{text}");
        }
        for text in ["7", "[]", "{blocks:[7]}"] {
            let (tag, _) = sunrise_core::nbt::parse_snbt(text).unwrap();
            assert!(!projectile_predicate_supported(&tag), "{text}");
        }
        let (tag, _) = sunrise_core::nbt::parse_snbt(
            "{blocks:\"minecraft:chorus_flower\",state:{age:{min:\"0\",max:\"5\"}}}",
        )
        .unwrap();
        assert!(projectile_predicate_supported(&tag));
    }

    /// `InventoryScreen.containerTick` / `CreativeModeInventoryScreen.containerTick`:
    /// both directions follow `hasInfiniteMaterials` only for player-inventory screens,
    /// leaving container menus and other screens alone.
    #[test]
    fn open_inventory_reconciles_both_directions_with_infinite_materials() {
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, true, true),
            Screen::Creative
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, true, false),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, false, true),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, false, false),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Creative, false, false),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Creative, false, true),
            Screen::Creative
        );
        for screen in [
            Screen::Playing,
            Screen::Paused,
            Screen::Stats,
            Screen::Advancements,
        ] {
            assert_eq!(
                inventory_after_infinite_materials(screen, false, true),
                screen
            );
            assert_eq!(
                inventory_after_infinite_materials(screen, false, false),
                screen
            );
        }
    }

    /// `ChunkMap.skipPlayer`: only a spectator with `spectators_generate_chunks`
    /// off stops loading chunks around itself.
    #[test]
    fn a_skipped_spectator_loads_no_chunks() {
        assert!(!loads_chunks(false, true), "spectator, rule off");
        assert!(loads_chunks(true, true), "spectator, rule on");
        assert!(
            loads_chunks(false, false),
            "survival, creative or adventure"
        );
        assert!(
            loads_chunks(true, false),
            "survival, creative or adventure, rule on"
        );
    }
}
