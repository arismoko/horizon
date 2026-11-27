//! Pistons carry a mod block entity while its block moves (#3184), when the
//! block's part lets it move (`piston_carries_block_entity`): the game's side of the level's
//! `carry_block_entity` and `land_block_entity`, the carried block entity in
//! its moving piston's saved tag, and a client's copy. `BlockEntities` holds
//! what is carried (`carry`, `land`).
use super::GameLevel;
use crate::game::piston_carry_common::{PositionChange, land_position_data, take_position_data};
pub use sunrise_core::{geometry::Direction, nbt::Tag, position::BlockPos};
use sunrise_level::block_entities::Carried;
pub use sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, Attachments, attachment_compound};
pub use sunrise_sim::save::ChunkExtras;
pub use sunrise_world::{level::MovingPiston, registry::BlockStateId};

/// The moving piston's saved key for the block entity it carries, with its
/// attachments. Sunrise's own, written only while it carries one, so a
/// vanilla moving piston's tag is unchanged; vanilla reads only its own keys.
pub const CARRIED_KEY: &str = "sunrise:carried_block_entity";

impl GameLevel<'_> {
    /// Whether the piston at `piston` may push `push` one step `direction` and
    /// destroy `destroy`: on an authoritative server, what `piston_pre`'s
    /// listeners leave of it.
    pub(in crate::game) fn piston_move_allowed(
        &self,
        piston: BlockPos,
        direction: Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) -> bool {
        let mut canceled = self.world.carrier_fences.holds_move(push, destroy);
        if !canceled && self.native_authoritative {
            let step = direction.normal();
            self.native_piston_pre(mod_api::__private::PistonPreCall {
                dimension: self.dimension,
                piston,
                direction: step,
                extending,
                push,
                destroy,
                canceled: &mut canceled,
            });
        }
        !canceled
    }
    /// The piston at `piston` pushed `push` one step `direction` and destroyed
    /// `destroy`: on an authoritative server, `piston_post`'s listeners hear it.
    pub(in crate::game) fn piston_moved(
        &self,
        piston: BlockPos,
        direction: Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) {
        if self.native_authoritative {
            let step = direction.normal();
            self.native_piston_post(self.dimension, piston, step, extending, push, destroy);
        }
    }
    /// `piston_carries_block_entity` for the block `state` at `pos`: what
    /// its part answers, or None.
    pub(in crate::game) fn piston_carries_block_entity(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<bool> {
        let mut movable = None;
        self.native_piston_carries_block_entity(self.dimension, pos, state, &mut movable);
        if movable == Some(true) {
            let at = (pos.x, pos.y, pos.z);
            if self.world.block_entities.is_taken(&at) {
                return Some(false);
            }
            // A live owned payload is required. Raw-only/lazy native stores cannot be
            // transferred by BlockEntities::carry and must remain refused, with their data intact.
            if (self.facts.get(state).block_entity
                || self.world.block_entities.stateful_entity(state.0))
                && self.world.block_entities.get(&at).is_none()
            {
                return Some(false);
            }
        }
        movable
    }
    /// Takes the block entity at `from` out of the world for the moving piston
    /// at `to`, with what removing it would take from its position (its custom
    /// name, its loaded tag) but none of its side effects.
    pub fn carry_mod_block_entity(&mut self, from: BlockPos, to: BlockPos) {
        let (from, to) = ((from.x, from.y, from.z), (to.x, to.y, to.z));
        let partner = self
            .registry
            .state(self.world.block(from.0, from.1, from.2))
            .and_then(|state| {
                sunrise_gameplay_common::chests::chest_connected_pos(
                    state,
                    [from.0, from.1, from.2],
                )
            })
            .map(|p| (p[0], p[1], p[2]));
        let mut data = take_position_data(self.world, from);
        let cached = self.positions.hoppers.remove(&from).map(|h| h.cooldown);
        let is_hopper = sunrise_game_data::block_entity_types::factory_block(
            &self
                .facts
                .get(self.world.block(from.0, from.1, from.2))
                .name,
        ) == "minecraft:hopper";
        let cooldown = if is_hopper {
            cached.or_else(|| {
                data.get(&"Source".into())
                    .and_then(|t| t.get("TransferCooldown"))
                    .and_then(|t| match t {
                        Tag::Int(n) => Some(*n),
                        _ => None,
                    })
            })
        } else {
            None
        };
        if let Some(n) = cooldown {
            data.insert("HopperCooldown".into(), Tag::Int(n));
        }
        self.world.block_entities.carry(from, to);
        if let Some(carried) = self.world.block_entities.carried_mut(&to) {
            carried.position_data = data;
        }
        self.lid_openers.remove(&from);
        self.world.chest_openers.remove(&from);
        self.world.shulkers.remove(&[from.0, from.1, from.2]);
        self.spill
            .piston_positions
            .push(PositionChange::Carry { from, to, partner });
    }
    /// Installs the block entity the moving piston at `pos` carried there; its
    /// custom name is the block's again, as for one loaded there, and on a
    /// server its synced attachments go whole to the players who see it.
    pub fn land_carried_block_entity(&mut self, pos: BlockPos) -> bool {
        let key = (pos.x, pos.y, pos.z);
        let data = self
            .world
            .block_entities
            .carried(&key)
            .map(|c| c.position_data.clone())
            .unwrap_or_default();
        if !self.world.block_entities.land(key) {
            return false;
        }
        land_position_data(self.world, key, &data);
        let cooldown = match data.get(&"HopperCooldown".into()) {
            Some(Tag::Int(n)) => Some(*n),
            _ => None,
        };
        self.positions.hoppers.remove(&key);
        if let Some(n) = cooldown {
            self.positions.hoppers.insert(
                key,
                sunrise_gameplay_common::hoppers::HopperState::with_cooldown(n),
            );
        }
        self.spill
            .piston_positions
            .push(PositionChange::Land { at: key });
        if self.native_authoritative && self.world.block_entities.attachments(key).is_some() {
            self.world.attachment_landings.insert(
                sunrise_mod_runtime::attachments::Holder::BlockEntity([pos.x, pos.y, pos.z]),
            );
        }
        true
    }
}

impl crate::game::CommonGame<'_> {
    /// `PistonMovingBlockEntity.saveAdditional` for a chunk save, with the
    /// block entity the piston carries under [`CARRIED_KEY`].
    pub fn saved_moving_piston(
        &self,
        pos: BlockPos,
        piston: &MovingPiston,
    ) -> Result<Tag, sunrise_mod_runtime::attachments::AttachmentEncodeError> {
        let mut tag = self.moving_piston_to_nbt(pos, piston);
        if let Some(Carried {
            entity,
            attachments,
            position_data,
        }) = self.world.block_entities.carried(&(pos.x, pos.y, pos.z))
            && let Tag::Compound(map) = &mut tag
        {
            let block = &self.res.facts().get(piston.moved_state).name;
            let mut carried = sunrise_sim::save::block_entity_encode::encode(
                (pos.x, pos.y, pos.z),
                entity,
                block,
                self.res.items(),
            );
            if let Tag::Compound(fields) = &mut carried {
                fields.insert(
                    "sunrise:position_data".into(),
                    Tag::Compound(position_data.clone()),
                );
                match attachments
                    .as_ref()
                    .map(Attachments::try_to_nbt)
                    .transpose()?
                {
                    Some(data) if !data.is_empty() => {
                        fields.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
                    }
                    _ => {
                        fields.remove(&ATTACHMENTS_KEY.into());
                    }
                }
            }
            map.insert(CARRIED_KEY.into(), carried);
        }
        Ok(tag)
    }
    /// `PistonMovingBlockEntity.loadAdditional`: the moving piston at `pos`
    /// as a new ticker, holding again the block entity its saved tag in
    /// `saved` carries. One that can't be read is logged and left out: the
    /// block lands without it.
    pub fn restore_moving_piston(
        &mut self,
        pos: BlockPos,
        piston: MovingPiston,
        saved: Option<&ChunkExtras>,
    ) {
        self.block_ticks.load_moving_piston(pos, piston);
        let Some(tag) = saved
            .into_iter()
            .flat_map(|s| &s.block_entities)
            .find(|tag| crate::game::piston_nbt::block_entity_pos(tag) == [pos.x, pos.y, pos.z])
            .and_then(|tag| tag.get(CARRIED_KEY))
        else {
            return;
        };
        let block = &self.res.facts().get(piston.moved_state).name;
        match sunrise_sim::save::decode_block_entity_for_block(tag, block) {
            Some((_, entity)) => {
                let attachments = match attachment_compound(tag.get(ATTACHMENTS_KEY)) {
                    Ok(Some(data)) => Some(Attachments::from_nbt(data.clone())),
                    Ok(None) => None,
                    Err(_) => {
                        log::warn!(
                            "Sunrise: unreadable attachments on the block entity in the moving piston at {} {} {}; it lands without them",
                            pos.x,
                            pos.y,
                            pos.z
                        );
                        None
                    }
                };
                self.world.block_entities.hold_carried(
                    (pos.x, pos.y, pos.z),
                    Carried {
                        entity,
                        position_data: match tag.get("sunrise:position_data") {
                            Some(Tag::Compound(data)) => data.clone(),
                            _ => Default::default(),
                        },
                        attachments,
                    },
                );
            }
            None => log::warn!(
                "Sunrise: unreadable block entity in the moving piston at {} {} {}; it lands without it",
                pos.x,
                pos.y,
                pos.z
            ),
        }
    }
}
