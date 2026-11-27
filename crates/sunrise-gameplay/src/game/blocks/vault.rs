//! Vaults in the game: `VaultBlockEntity.Server.tick` for every vault
//! (`vault`), the block's state as it changes, the rewards it ejects, the
//! keys players insert (`VaultBlock.useItemOn`) and the display item spinning
//! inside (`VaultRenderer`).
use crate::game::ServerGame;
use sunrise_core::random::{LegacyRandom, LevelRandom};
use sunrise_game_data::Result;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    block_entities::BlockEntityLease,
    containers::BlockEntity,
    vault::{Vault, VaultEvent, VaultLevel, VaultPlayer, VaultState},
};

pub(in crate::game) type Pos = (i32, i32, i32);

pub(in crate::game) struct TakenVault {
    pub(in crate::game) lease: BlockEntityLease,
    pub(in crate::game) vault: Box<Vault>,
    pub(in crate::game) current: VaultState,
    pub(in crate::game) ominous: bool,
}

/// What a key insertion supplies to the reward roll
/// (`VaultBlockEntity.resolveItemsToEject`): the player, the inserted stack and
/// the player's luck.
pub(in crate::game) struct VaultEject {
    pub(in crate::game) player: sunrise_world::loot::LootEntity,
    pub(in crate::game) tool: sunrise_world::loot::LootStack,
    pub(in crate::game) luck: f32,
}

/// What a vault reads: the game time and the players. The players are listed
/// at the first ask: a vault looks for them only when its state may change
/// (`stateUpdatingResumesAt`, once a second), not at every tick.
pub(in crate::game) struct Players<'g, 'a> {
    pub(in crate::game) time: i64,
    pub(in crate::game) game: &'g crate::game::ServerGame<'a>,
    pub(in crate::game) players: std::cell::OnceCell<Vec<VaultPlayer>>,
}
impl VaultLevel for Players<'_, '_> {
    fn game_time(&self) -> i64 {
        self.time
    }
    fn players(&self) -> Vec<VaultPlayer> {
        self.players
            .get_or_init(|| self.game.vault_players())
            .clone()
    }
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn vault_players(&self) -> Vec<VaultPlayer> {
        let host = self.common.player.as_ref().map(|p| VaultPlayer {
            uuid: self.acting_uuid(),
            block: [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32),
            spectator: self.common.spectator(),
        });
        let guests = self.lan_present().into_iter().map(|g| VaultPlayer {
            uuid: g.uuid,
            block: g.position.map(|v| v.floor() as i32),
            spectator: g.spectator,
        });
        host.into_iter().chain(guests).collect()
    }
    /// The vault at `key` with its block's state and ominous flag.
    pub(in crate::game) fn take_vault(&mut self, key: Pos) -> Option<TakenVault> {
        let state = self.common.world.block(key.0, key.1, key.2);
        let block = self.common.pack.assets.registry.state(state)?;
        if sunrise_game_data::block_entity_types::factory_block(&block.name) != "minecraft:vault" {
            return None;
        }
        let current = block
            .properties
            .get("vault_state")
            .and_then(|s| VaultState::from_name(s))
            .unwrap_or_default();
        let ominous = block.properties.get("ominous").is_some_and(|v| v == "true");
        let Some((lease, BlockEntity::Vault(vault))) = self.common.world.block_entities.take(&key)
        else {
            return None;
        };
        Some(TakenVault {
            lease,
            vault,
            current,
            ominous,
        })
    }
    /// Runs `f` on the vault at `key` with the level random and a loot roll,
    /// then applies its events.
    pub(in crate::game::server_side) fn with_vault<T>(
        &mut self,
        key: Pos,
        eject: Option<&VaultEject>,
        f: impl FnOnce(
            &mut Vault,
            &Players<'_, '_>,
            &mut LegacyRandom,
            VaultState,
            bool,
            &mut sunrise_level::vault::Roll<LegacyRandom>,
            &mut Vec<VaultEvent>,
        ) -> T,
    ) -> Option<Result<T>> {
        let TakenVault {
            lease,
            mut vault,
            current,
            ominous,
        } = self.take_vault(key)?;
        let mut random = std::mem::replace(
            &mut self.common.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let mut sequences = std::mem::take(&mut self.server.rand_sequences);
        let loot = self.server.server_loot.0.get();
        let players = Players {
            time: self.common.world.game_time,
            game: self,
            players: std::cell::OnceCell::new(),
        };
        let seed = self.common.level.seed;
        let mut events = Vec::new();
        let out = {
            // LootTable.getRandomItems, stacks split; the reward rolls with the
            // table's random sequence.
            let mut roll = |table: &str, random: &mut LegacyRandom, sequence: bool| {
                let Some(loot) = &loot else {
                    return Vec::new();
                };
                // `LootContextParamSets.VAULT`: the vault's centre is the ORIGIN of
                // every roll; the reward adds THIS_ENTITY, TOOL and luck.
                let reward = eject.filter(|_| sequence);
                let context = sunrise_world::loot::ContainerLootContext {
                    luck: reward.map_or(0.0, |e| e.luck),
                    entity: reward.map(|e| &e.player),
                    ..Default::default()
                };
                let caller = players.game.caller_loot(
                    Some([key.0, key.1, key.2].map(|v| f64::from(v) + 0.5)),
                    reward.map(|e| e.tool.clone()),
                );
                let drops = caller
                    .with(|| match loot.random_sequence(table).filter(|_| sequence) {
                        Some(s) => loot.roll_container_with(table, sequences.get(s, seed), context),
                        None => loot.roll_container_with(table, random, context),
                    })
                    .unwrap_or_default();
                drops
                    .iter()
                    .map(sunrise_sim::structure_blocks::loot_item_with_components)
                    .collect()
            };
            f(
                &mut vault,
                &players,
                &mut random,
                current,
                ominous,
                &mut roll,
                &mut events,
            )
        };
        drop(players);
        self.common.world.random = random;
        self.server.rand_sequences = sequences;
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Vault(vault));
        Some(self.vault_events(key, events).map(|()| out))
    }
    /// The server tick of the vault at `key`, then its client tick when there
    /// is a window.
    pub fn tick_vault(&mut self, key: Pos) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let ticked = self.with_vault(key, None, |v, l, random, state, ominous, roll, events| {
            v.server_tick(l, random, pos, state, ominous, roll, events);
        });
        if let Some(result) = ticked {
            result?;
        }
        Ok(())
    }
    /// `VaultBlock.useItemOn` with `held` in the hand that acts: true when the
    /// vault took the click (an active vault, with anything in the hand).
    pub(in crate::game::server_side) fn use_vault(
        &mut self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> Result<bool> {
        let Some(stack) = held.cloned() else {
            return Ok(false);
        };
        let player = self.acting_uuid();
        let eject = VaultEject {
            player: self.killer_loot_entity(None),
            tool: sunrise_gameplay_common::game::creatures::to_loot(&stack),
            luck: self.acting_luck(),
        };
        let used = self.with_vault(
            (pos[0], pos[1], pos[2]),
            Some(&eject),
            |v, l, random, state, ominous, roll, events| {
                v.use_item_on(
                    l,
                    random,
                    pos,
                    state,
                    ominous,
                    player,
                    Some(&stack),
                    roll,
                    events,
                )
            },
        );
        let Some(taken) = used.transpose()?.flatten() else {
            return Ok(false);
        };
        if taken > 0 {
            // awardStat(ITEM_USED), then stackToInsert.consume(count, player).
            self.award_stat("used", &stack.item, 1);
            for _ in 0..taken {
                self.consume_held();
            }
        }
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, &stack);
        Ok(true)
    }
    /// `setBlockAndUpdate` with the new `vault_state`.
    pub(in crate::game::server_side) fn set_vault_state(
        &mut self,
        key: Pos,
        state: VaultState,
    ) -> Result<()> {
        let current = self.common.world.block(key.0, key.1, key.2);
        let Some(mut block) = self.common.pack.assets.registry.state(current).cloned() else {
            return Ok(());
        };
        block
            .properties
            .insert("vault_state".into(), state.name().into());
        let id = self.common.pack.assets.registry.resolve(&block)?;
        self.level_set([key.0, key.1, key.2], id)?;
        self.native_vault_change_state(self.common.dim.id(), [key.0, key.1, key.2], state.name());
        Ok(())
    }
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn vault_events(
        &mut self,
        key: Pos,
        events: Vec<VaultEvent>,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let center = pos.map(|v| f64::from(v) + 0.5);
        for event in events {
            match event {
                VaultEvent::State(state) => self.set_vault_state(key, state)?,
                VaultEvent::BlockUpdated => self.mark_block_entity_updated(key),
                VaultEvent::LevelEvent(id, data) => self.server_level_event_at(id, pos, data),
                VaultEvent::Sound(name, volume, pitch) => {
                    self.server_play_sound(&format!("minecraft:{name}"), center, volume, pitch)
                }
                // Vec3.atBottomCenterOf(pos).relative(UP, 1.2), lowered by spawnItem.
                VaultEvent::Eject(stack, v) => {
                    self.native_vault_loot(pos, &stack.item);
                    self.spawn_item_thrown(
                        stack,
                        [center[0], center[1] + 0.7 - 0.125, center[2]],
                        v,
                    )
                }
            }
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
