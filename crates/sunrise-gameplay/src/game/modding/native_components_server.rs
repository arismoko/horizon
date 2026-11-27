use crate::game::ServerGame;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    pub(in crate::game) fn server_item_combat_actor(
        &self,
        actor: mod_api::LivingRef,
    ) -> Option<(bool, bool, f64, bool)> {
        use super::commands::Target;
        let target = self.server_living_read_target(actor)?;
        Some(match target {
            Target::Player => {
                let p = self.common.player.as_ref()?;
                (true, self.common.creative, p.fall_distance, p.fall_flying)
            }
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                let p = guest.motion.as_ref()?;
                (true, guest.creative(), p.fall_distance, p.fall_flying)
            }
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
                (false, false, mob.fall_distance, mob.extra.fall_flying)
            }
            Target::Stand(_) | Target::Dragon(_) => (false, false, 0.0, false),
            _ => return None,
        })
    }

    /// `ItemStack.hurtAndBreak(amount, level, player, onBreak)` on a stack nobody wears: the
    /// stack after the wear, None once it broke away. A named player is checked as the other
    /// player reads check it (`player_handle_read`) and must be in `level`, so the wear and its
    /// random draws stay in the level in place; the player is then swapped in for the wear
    /// (their infinite materials, their durability trigger) without changing levels. Refused
    /// off the server, for another level than the one in place, or for a player who is not
    /// here or is in another level.
    pub(in crate::game) fn item_stack_hurt_and_break(
        &mut self,
        stack: &ItemStack,
        level: mod_api::LevelRef,
        amount: i32,
        player: Option<mod_api::PlayerRef>,
    ) -> Result<Option<ItemStack>, mod_api::StackWriteError> {
        use super::commands::Target;
        use super::durability::Wearer;
        use mod_api::StackWriteError::Unavailable;
        if !self.common.is_logical_server()
            || level.dimension() != self.common.dim.key()
            || self.native_level_world(level).is_none()
        {
            return Err(Unavailable);
        }
        let (worn, _broken) = match player {
            None => self.damage_stack_worn(stack, amount, Wearer::Nobody),
            Some(player) => {
                self.player_handle_read(player).ok_or(Unavailable)?;
                let target = self
                    .native_entity_target(player.entity())
                    .ok_or(Unavailable)?;
                let here = match target {
                    Target::Player => self.acting_in_level_in_place(),
                    Target::Guest(conn) => self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|lan| lan.hub.guest(conn))
                        .is_some_and(|guest| {
                            sunrise_gameplay_common::game::Dim::from_id(&guest.dimension)
                                == self.common.dim
                        }),
                    _ => false,
                };
                if !here {
                    return Err(Unavailable);
                }
                self.for_player(target, |game| {
                    game.damage_stack_worn(stack, amount, Wearer::Player)
                })
                .ok_or(Unavailable)?
            }
        };
        Ok(worn)
    }

    /// Roll through the game's enchantment registry and the active level's random source.
    pub(in crate::game) fn item_resource_enchant_randomly(
        &mut self,
        stack: &ItemStack,
        level: mod_api::LevelRef,
        power: i32,
        pool: &mod_api::Identifier,
    ) -> Result<Option<ItemStack>, mod_api::StackWriteError> {
        if level.dimension() != self.common.dim.key() || self.native_level_world(level).is_none() {
            return Ok(None);
        }
        let items = server_world_items!(self);
        let enchantments = std::sync::Arc::clone(self.item_enchantments());
        let pool = enchantments.tag(&pool.to_string());
        if pool.is_empty() {
            return Ok(None);
        }
        let value = enchantments.enchantable_of(stack);
        let picked = enchantments.select_with_value(
            items,
            &mut self.common.world.random,
            &stack.item,
            power,
            pool,
            value,
        );
        Ok(sunrise_sim::stations::apply_enchantment_list(stack, picked))
    }

    /// `EnchantmentHelper.modifyDurabilityToRepairFromXp` for `stack` and `points`, by the world's
    /// enchantment definitions, drawing from the active level's random as Mending's repair does,
    /// behind `ItemResource::durability_to_repair_from_xp`. API @API:item_components@.
    pub(in crate::game) fn item_resource_durability_to_repair_from_xp(
        &mut self,
        stack: &ItemStack,
        points: i32,
    ) -> u32 {
        super::enchant_effects::repair_from_xp(
            &server_world_stations!(self).enchantments,
            Some(server_world_items!(self)),
            stack,
            points,
            &mut self.common.world.random,
        )
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
