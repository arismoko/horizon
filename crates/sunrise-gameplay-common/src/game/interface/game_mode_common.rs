//! Common game-mode queries, adventure predicates, chat abilities, and player abilities.
use super::Screen;
use mod_api::{PlayerAbilities, PlayerRef};
use sunrise_command::args::GameMode;
use sunrise_items::items::ItemStack;

#[allow(unused_imports)]
pub use sunrise_gameplay_common::game_modes::{
    loads_chunks, mode_from_id, projectile_predicate_supported,
};

/// Reconciles an open player inventory with `Player.hasInfiniteMaterials`.
pub fn inventory_after_infinite_materials(
    screen: Screen,
    player_inventory: bool,
    infinite_materials: bool,
) -> Screen {
    match (screen, player_inventory, infinite_materials) {
        (Screen::Inventory, true, true) => Screen::Creative,
        (Screen::Creative, _, false) => Screen::Inventory,
        _ => screen,
    }
}

/// `BlockPos.relative` with Java's wrapping integer arithmetic.
pub fn relative(pos: [i32; 3], face: sunrise_core::geometry::Direction) -> [i32; 3] {
    let [dx, dy, dz] = face.normal();
    [
        pos[0].wrapping_add(dx),
        pos[1].wrapping_add(dy),
        pos[2].wrapping_add(dz),
    ]
}

impl crate::game::CommonGame<'_> {
    /// `MultiPlayerGameMode.isSpectator()` on a client; `ServerPlayer.isSpectator()` on a server.
    pub fn spectator(&self) -> bool {
        self.mode == GameMode::Spectator
    }
    /// `MultiPlayerGameMode.hasMissTime()` (`!localPlayerMode.isCreative()`).
    pub fn has_miss_time(&self) -> bool {
        self.mode != GameMode::Creative
    }
    pub fn mode_id(&self) -> i32 {
        self.mode.id()
    }
    /// A `HolderSet<Block>` entry: a block id or a `#tag`.
    pub(in crate::game) fn block_in_holder_set(&self, entry: &str, block: &str) -> bool {
        let namespaced = |id: &str| {
            if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            }
        };
        match entry.strip_prefix('#') {
            Some(tag) => self.res.items().block_in_tag(&namespaced(tag), block),
            None => namespaced(entry) == block,
        }
    }
}

pub trait GameModeCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `AbstractClientPlayer.gameMode()` on a client (`PlayerInfo.getGameMode()`), or `self.mode` on a server.
    fn player_info_game_mode(&self) -> Option<GameMode> {
        self.side_player_info_game_mode()
    }
    /// `Player.isSpectator()`: reads `PlayerInfo` on a client and `self.mode` on a server.
    fn own_player_is_spectator(&self) -> bool {
        self.player_info_game_mode() == Some(GameMode::Spectator)
    }
    /// `Player.isCreative()`: reads `PlayerInfo` on a client and `self.mode` on a server.
    fn own_player_is_creative(&self) -> bool {
        self.player_info_game_mode() == Some(GameMode::Creative)
    }
    /// `Abilities.mayBuild`.
    fn may_build(&self) -> bool {
        self.side_may_build()
    }
    /// `Abilities.instabuild`. A client reads its live `Remote::abilities`.
    fn instabuild(&self) -> bool {
        self.side_instabuild()
    }
    /// `Player.getAbilities`; Paper `Player#getAllowFlight/isFlying/getFlySpeed/getWalkSpeed`.
    fn player_abilities(&self, player: PlayerRef) -> Option<PlayerAbilities> {
        self.side_player_abilities(player)
    }
    /// `ItemStack.canBreakBlockInAdventureMode` / `canPlaceOnBlockInAdventureMode` for the main hand.
    fn adventure_predicate(&self, component: &str, pos: [i32; 3]) -> bool {
        self.held()
            .is_some_and(|stack| self.stack_adventure_predicate(stack, component, pos))
    }
    /// `adventure_predicate` for any `stack`, not only the main hand's.
    fn stack_adventure_predicate(&self, stack: &ItemStack, component: &str, pos: [i32; 3]) -> bool {
        let Some(tag) = stack.data().and_then(|d| d.other(component)) else {
            return false;
        };
        self.adventure_predicate_at(tag, pos)
    }
    /// Supported block/state/NBT portion of `AdventureModePredicate.test`.
    fn adventure_predicate_at(&self, tag: &sunrise_core::nbt::Tag, pos: [i32; 3]) -> bool {
        use sunrise_core::nbt::Tag;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = &self.res.facts().get(state).name;
        let properties = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| &s.properties);
        let matches = |p: &Tag| {
            let block_ok = match p.get("blocks") {
                None => true,
                Some(Tag::String(id)) => self.block_in_holder_set(&id.to_string_lossy(), name),
                Some(Tag::List(ids)) => ids.iter().any(|id| match id {
                    Tag::String(id) => self.block_in_holder_set(&id.to_string_lossy(), name),
                    _ => false,
                }),
                Some(_) => false,
            };
            // StatePropertiesPredicate: every property exact, or in a min/max range.
            let state_ok = match p.get("state") {
                None => true,
                Some(Tag::Compound(want)) => want.iter().all(|(key, matcher)| {
                    let Some(have) = properties.and_then(|p| p.get(&key.to_string_lossy())) else {
                        return false;
                    };
                    match matcher {
                        Tag::String(value) => *have == value.to_string_lossy(),
                        Tag::Compound(_) => {
                            let bound = |k: &str| match matcher.get(k) {
                                Some(Tag::String(v)) => Some(v.to_string_lossy()),
                                _ => None,
                            };
                            let in_range = |have: &str, v: Option<String>, above: bool| {
                                v.is_none_or(|v| match (have.parse::<i64>(), v.parse::<i64>()) {
                                    (Ok(h), Ok(b)) if above => h >= b,
                                    (Ok(h), Ok(b)) => h <= b,
                                    _ => false,
                                })
                            };
                            in_range(have, bound("min"), true)
                                && in_range(have, bound("max"), false)
                        }
                        _ => false,
                    }
                }),
                Some(_) => false,
            };
            // NbtPredicate (TagParser.LENIENT_CODEC): SNBT or compound, compareNbt(..., true).
            let nbt_ok = || match p.get("nbt") {
                None => true,
                Some(Tag::String(snbt)) => sunrise_core::nbt::parse_snbt(&snbt.to_string_lossy())
                    .is_ok_and(|(pattern, _)| {
                        sunrise_game_data::nbt_path::compare_nbt(
                            &pattern,
                            self.side_block_entity_nbt(pos).as_ref(),
                        )
                    }),
                Some(pattern @ Tag::Compound(_)) => sunrise_game_data::nbt_path::compare_nbt(
                    pattern,
                    self.side_block_entity_nbt(pos).as_ref(),
                ),
                Some(_) => false,
            };
            block_ok && state_ok && nbt_ok()
        };
        match tag {
            Tag::List(list) => list.iter().any(matches),
            single => matches(single),
        }
    }
    /// `Player.blockActionRestricted` for the block at `pos`.
    fn block_action_restricted(&self, pos: [i32; 3]) -> bool {
        if self.spectator() {
            return true;
        }
        !self.may_build() && !self.can_break_in_adventure(pos)
    }
    /// `ItemStack.canBreakBlockInAdventureMode`.
    fn can_break_in_adventure(&self, pos: [i32; 3]) -> bool {
        self.adventure_predicate("minecraft:can_break", pos)
    }
    /// `ItemStack.useOn` in adventure mode (`Player.mayUseItemAt`).
    fn can_place_on_in_adventure(&self, clicked: [i32; 3]) -> bool {
        self.adventure_predicate("minecraft:can_place_on", clicked)
    }
    /// `Player.mayUseItemAt(pos, face, stack)`.
    fn may_use_item_at(
        &self,
        may_build: bool,
        pos: [i32; 3],
        face: sunrise_core::geometry::Direction,
        stack: &ItemStack,
    ) -> bool {
        may_build
            || self.stack_adventure_predicate(
                stack,
                "minecraft:can_place_on",
                relative(pos, face.opposite()),
            )
    }
    /// `Entity.doWaterSplashEffect` for a dropped item: server plays sound, client adds particles.
    fn apply_item_splash(&mut self, splash: sunrise_sim::entities::ItemSplash) {
        self.side_apply_item_splash(splash);
    }
    /// `Level.getMinY`: the current dimension type's minimum y.
    fn level_min_y(&self) -> i32 {
        self.side_level_min_y()
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameModeCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::player::PlayerSideGameExt as _;

use crate::game::side_ops::ui::UiSideGameExt as _;
