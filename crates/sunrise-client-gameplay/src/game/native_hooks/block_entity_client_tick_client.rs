//! Client ticks of mod block entities (`ClientLevel.tickBlockEntities`): the client's own
//! ticker list, bound from its registered type and state.
use super::*;
use mod_api::{
    BlockEntityClientAccess, BlockEntityClientScope, BlockEntityReadView, BlockEntityRef,
    ClientHost,
};
use sunrise_client::animate_tick::campfire_particle_tick;
use sunrise_gameplay_common::game::api_values::{TryIntoApi, block_state::mod_ticker_states};
use sunrise_gameplay_common::game::native_hooks::block_entity_read::ResidentReadBlockEntity;
use sunrise_level::{block_entities::Ticking, containers::BlockEntity};

/// Shared sequential client block-entity view + mutable `ClientHost` backend.
#[cfg(feature = "client")]
pub(in crate::game) struct ClientBlockEntityAccess<'g, 'w> {
    pub game: &'g mut crate::game::ClientGame<'w>,
    pub subject: BlockEntityRef,
}

#[cfg(feature = "client")]
impl BlockEntityClientAccess for ClientBlockEntityAccess<'_, '_> {
    fn route_kind(&self) -> Option<&str> {
        use mod_api::__private::game::Fields;
        let target = self.subject.fields();
        let pos = (target.position[0], target.position[1], target.position[2]);
        if self.game.native_client_level() != Some(target.level)
            || self.game.world.block_entities.generation(&pos) != Some(target.generation)
        {
            return None;
        }
        match self.game.world.block_entities.get(&pos) {
            Some(BlockEntity::Mod(entity)) => Some(entity.id()),
            _ => None,
        }
    }

    fn with_view(&mut self, call: &mut dyn FnMut(BlockEntityReadView<'_>)) {
        use mod_api::__private::game::Fields;
        let target = self.subject.fields();
        let [x, y, z] = target.position;
        let pos = (x, y, z);
        if self.game.native_client_level() != Some(target.level)
            || !self.game.world.loaded(x, z)
            || self.game.world.block_entities.generation(&pos) != Some(target.generation)
        {
            return;
        }
        let Some(BlockEntity::Mod(entity)) = self.game.world.block_entities.get(&pos) else {
            return;
        };
        let state_id = self.game.world.block(x, y, z);
        let Ok(state) = state_id.try_into_api() else {
            return;
        };
        let slot_count = self
            .game
            .world
            .block_entities
            .mod_container(state_id.0)
            .map_or(0, |c| c.slots);
        let data = ResidentReadBlockEntity { entity, slot_count };
        call(BlockEntityReadView::new(self.subject, state, &data));
    }

    fn host(&mut self) -> &mut dyn ClientHost {
        self.game
    }
}

/// A client tick of this client's resident mod block entity (`ClientLevel.tickBlockEntities`):
/// the scope lends read-only views of the resident copy sequentially with the mutable
/// `ClientHost`. Never called on a server, and never again once removed or unloaded.
pub(in crate::game) fn native_block_entity_client_tick(
    subject: mod_api::BlockEntityClientScope<'_>,
) {
    crate::game::native_hooks::fire::fire_block_entity_client_tick(subject);
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ClientLevel.tickBlockEntities` on a server's client, for the block entities with a client
    /// ticker: spawners, trial spawners, vaults, the animated ones, campfires, beacons, potent
    /// sulfur and mod ones. The client's ticker list
    /// (`Level.blockEntityTickers`, kept in its world's block entities as a server keeps its own)
    /// is taken once, at the tick's start, so a ticker made during the tick waits for the next;
    /// each live one inside the world border then ticks in its order, as they all draw from the
    /// one level random. A mod one is told only while a part has a client tick and there is a
    /// client level.
    pub fn tick_client_block_entities(&mut self) {
        self.ensure_client_ticking_states();
        let level = mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityClientTick)
            .then(|| self.native_client_level())
            .flatten();
        let tickers = self.common.world.block_entity_tickers();
        let mut anim = self.begin_block_entity_anims();
        for &ticker in tickers.iter() {
            let key @ (x, y, z) = ticker.0;
            // Ticks before this one can remove it (`TickingBlockEntity.isRemoved`).
            if !self.common.world.block_entities.is_live(ticker) {
                continue;
            }
            // A mod's block entity ticks by its own type, whatever its block is named
            // (`BlockEntityTicker` of the registered type): never a vanilla animation.
            let animated = match self.common.world.block_entities.get(&key) {
                Some(BlockEntity::Mod(_)) => None,
                _ => self
                    .client
                    .be_anims
                    .ticker(self.common.world.block(x, y, z).0),
            };
            // One outside the border waits (`LevelChunk.isTicking`).
            if !self.common.border.contains(f64::from(x), f64::from(z)) {
                continue;
            }
            if let Some(kind) = animated {
                self.tick_block_entity_anim([x, y, z], kind, &mut anim);
                continue;
            }
            match self.common.world.block_entities.get(&key) {
                Some(BlockEntity::Beacon(_)) => self.client_tick_beacon(key),
                Some(BlockEntity::Campfire(_)) => self.client_tick_campfire(key),
                Some(BlockEntity::PotentSulfur(_)) => self.client_tick_potent_sulfur(key),
                Some(BlockEntity::Spawner(_)) => self.client_tick_spawner(key),
                Some(BlockEntity::TrialSpawner(_)) => self.client_tick_trial_spawner(key),
                // VaultBlock.getTicker on the client: VaultBlockEntity.Client.tick.
                Some(BlockEntity::Vault(_)) => self.client_tick_vault(key),
                Some(BlockEntity::Mod(_)) => {
                    if let Some(level) = level {
                        self.client_tick_mod_block_entity(level, key);
                    }
                }
                _ => {}
            }
        }
        self.finish_block_entity_anims();
        self.forget_spawner_displays();
    }

    /// The client tick of the resident mod copy at `pos`, lending its storage directly.
    pub(in crate::game) fn client_tick_mod_block_entity(
        &mut self,
        level: mod_api::LevelRef,
        pos: (i32, i32, i32),
    ) {
        let (x, y, z) = pos;
        let block_entities = &self.common.world.block_entities;
        let (Some(BlockEntity::Mod(entity)), Some(generation)) =
            (block_entities.get(&pos), block_entities.generation(&pos))
        else {
            return;
        };
        let state = self.common.world.block(x, y, z);
        if !entity
            .numeric_id()
            .is_some_and(|kind| block_entities.mod_ticking(state.0, kind) == Ticking::Yes)
        {
            return;
        }
        let subject = mod_api::__private::game::block_entity_ref(level, [x, y, z], generation);
        let mut access = ClientBlockEntityAccess {
            game: self,
            subject,
        };
        native_block_entity_client_tick(BlockEntityClientScope::new(&mut access));
    }
    /// Builds the table of the states that tick on the client (once), so a block entity's
    /// ticker is decided when its block changes (`LevelChunk.updateBlockEntityTicker`).
    pub(in crate::game) fn ensure_client_ticking_states(&mut self) {
        if !self.common.world.block_entities.has_states() {
            self.common
                .world
                .block_entities
                .set_mod_ticker_states(mod_ticker_states(mod_api::LogicalSide::Client));
            let states = self.client_ticking_states();
            self.common.world.block_entities.set_states(states);
        }
    }

    /// `BlockState.getTicker` with the client level, for every vanilla block state: spawners, trial
    /// spawners, vaults, beacons, the animated block entities, lit campfires and non-dry potent
    /// sulfur. Mod block entities select from their separate type-and-state table.
    pub(in crate::game) fn client_ticking_states(&self) -> Vec<Ticking> {
        let registry = self.common.pack.assets.registry;
        (0..registry.len())
            .map(|i| {
                let name: &str = &self.common.res.facts().get(BlockStateId(i as u32)).name;
                if name.starts_with("minecraft:") {
                    let property = |key| {
                        registry
                            .state(BlockStateId(i as u32))
                            .and_then(|s| s.properties.get(key))
                            .map(String::as_str)
                    };
                    return match name {
                        "minecraft:spawner"
                        | "minecraft:trial_spawner"
                        | "minecraft:vault"
                        | "minecraft:beacon" => Ticking::Yes,
                        // CampfireBlock.getTicker: the particles of a lit one.
                        "minecraft:campfire" | "minecraft:soul_campfire" => {
                            if property("lit") == Some("true") {
                                Ticking::Yes
                            } else {
                                Ticking::Idle
                            }
                        }
                        // Every state but dry has a ticker.
                        "minecraft:potent_sulfur" => {
                            if property("potent_sulfur_state") == Some("dry") {
                                Ticking::Idle
                            } else {
                                Ticking::Yes
                            }
                        }
                        // Chests, shulker boxes, bells, conduits, the enchanting table, the
                        // gateway and dragon and piglin heads.
                        _ if self.client.be_anims.ticker(i as u32).is_some() => Ticking::Yes,
                        _ => Ticking::No,
                    };
                }
                Ticking::No
            })
            .collect()
    }
}

impl crate::game::ClientGame<'_> {
    /// `CampfireBlockEntity.particleTick` of the campfire at `key`.
    pub(in crate::game) fn client_tick_campfire(&mut self, (x, y, z): (i32, i32, i32)) {
        let cooking = match self.common.world.block_entities.get(&(x, y, z)) {
            Some(BlockEntity::Campfire(c)) => c.items.each_ref().map(Option::is_some),
            _ => return,
        };
        let state = self.common.world.block(x, y, z).0;
        campfire_particle_tick(
            crate::game::client_side::ClientState::expect_client_res(&self.client.client_res)
                .animate(),
            &mut self.client.particles.0,
            self.common.pack.assets.registry,
            [x, y, z],
            state,
            cooking,
        );
    }
}
