use crate::game::ClientGame;
use sunrise_client::{block_entity_anim::Ticker, block_entity_render::BlockAnim};

/// What a client tick of the animated block entities shares between them.
pub(in crate::game) struct AnimTick {
    /// The conduits' frame blocks, copied when the first of them ticks.
    pub(in crate::game) frames: Option<std::collections::HashSet<String>>,
}

impl ClientGame<'_> {
    /// Start a tick of animated block entities, dropping stale owners and removed pots.
    pub(in crate::game) fn begin_block_entity_anims(&mut self) -> AnimTick {
        let world = &self.common.world;
        self.client.be_anims.retain_owned(
            |p, g| world.block_entities.generation(&(p[0], p[1], p[2])) == Some(g),
            |p| world.block(p[0], p[1], p[2]).0,
        );
        AnimTick { frames: None }
    }
    /// Renderer's view of the animated block entity at `pos`, including pot sherds.
    pub fn block_anim(&self, pos: [i32; 3], partial: f32) -> BlockAnim {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        // A replacement may draw before its first ConduitBlockEntity.clientTick.
        if matches!(self.client.be_anims.ticker(state.0), Some(Ticker::Conduit))
            && let Some(&owner) = self.client.be_anims.owners.get(&pos)
            && self
                .common
                .world
                .block_entities
                .generation(&(pos[0], pos[1], pos[2]))
                != Some(owner)
        {
            return BlockAnim::default();
        }
        let sherds = if &*self.common.res.facts().get(state).name == "minecraft:decorated_pot" {
            sunrise_gameplay_common::game::block_entity_anims_common::pot_sherds(
                self.common.saved_tag(pos),
            )
        } else {
            Default::default()
        };
        self.client
            .be_anims
            .block_anim(pos, self.common.world.game_time, partial, sherds)
    }
    /// `ChestLidController.getOpenness(partial)` at `pos`.
    pub(in crate::game) fn chest_lid(&self, pos: [i32; 3], partial: f32) -> f32 {
        self.client.be_anims.lid(pos, partial)
    }
    /// End of the block-entity animation tick: sync client shulker collision snapshots.
    pub(in crate::game) fn finish_block_entity_anims(&mut self) {
        if !self.common.is_logical_server() {
            self.sync_shulkers();
        }
    }
    /// Client chest ticker, also used when replaying a moved chest.
    pub fn tick_chest_lid(&mut self, pos: [i32; 3]) {
        self.client.be_anims.chests.entry(pos).or_default().tick();
    }

    /// `BlockEntity.triggerEvent(a, b)` on the block entity at `pos`.
    pub fn block_entity_event(&mut self, pos: [i32; 3], a: i32, b: i32) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]).0;
        let time = self.common.world.game_time;
        // PotentSulfurBlock.triggerEvent.
        if self
            .common
            .potent_sulfur_eruption_event((pos[0], pos[1], pos[2]))
        {
            return;
        }
        self.claim_anim(pos);
        match self.client.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                self.client
                    .be_anims
                    .chests
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b);
            }
            Some(Ticker::Shulker { .. }) => {
                let shulker = self.client.be_anims.shulkers.entry(pos).or_default();
                if shulker.trigger_event(a, b) {
                    // Publish shulker lid status change to the collision snapshot immediately.
                    self.common.world.shulkers.insert(pos, shulker.clone());
                }
            }
            Some(Ticker::Bell) => {
                let gathered = self.client_bell_gather(pos);
                self.client
                    .be_anims
                    .bells
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b, time, || gathered);
            }
            Some(Ticker::Gateway) => {
                let initial = self.common.gateway_loaded(pos);
                self.client
                    .be_anims
                    .gateways
                    .entry(pos)
                    .or_insert(initial)
                    .trigger_event(a, b);
            }
            Some(Ticker::Pot) => {
                self.client
                    .be_anims
                    .pots
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b, time);
            }
            _ => {}
        }
    }

    /// Bind animation state at `pos` to the current block entity generation.
    pub(in crate::game) fn claim_anim(&mut self, pos: [i32; 3]) {
        if let Some(generation) = self
            .common
            .world
            .block_entities
            .generation(&(pos[0], pos[1], pos[2]))
        {
            self.client.be_anims.claim(pos, generation);
        }
    }

    /// Copies the shulker box lids into the world, where the collision queries
    /// read them (`SceneCollisions::shulker_collision_state`), as
    /// `sync_moving_pistons` does for pistons.
    pub(in crate::game) fn sync_shulkers(&mut self) {
        self.common.world.shulkers.clear();
        self.common.world.shulkers.extend(
            self.client
                .be_anims
                .shulkers
                .iter()
                .map(|(p, s)| (*p, s.clone())),
        );
    }
}
