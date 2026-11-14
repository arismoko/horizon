//! The game's side of `engine_access`: entity reads, entity identity and the level operations the
//! host bodies in `horizon-mod-answers` are written against. Everything here reads and writes
//! `Game`'s private state; the answering crate sees only the traits.
use super::{Game, commands::Target};
use engine_access::{EntityIdentity, EntityRead, EntityView, LevelRead, LevelWrite, UuidState};
use horizon_world::registry::BlockStateId;
use mod_api::{
    BlockUpdateFlags, ClipBlock, ClipFluid, ClipHit, EntityRef, LevelRef, LevelWriteError,
};

/// A handle resolved once: the entity the game simulates, the copy a client tracks, or the
/// position an `entity_section` hook is in the middle of writing. The getters read the game
/// through the shared borrow the view holds; none of them resolves again.
pub(in crate::game) struct EntityAt<'a, 'g> {
    game: &'a Game<'g>,
    entity: EntityRef,
    written: Option<[f64; 3]>,
    target: Option<Target>,
    tracked: Option<(&'a str, [f64; 3])>,
}

impl EntityView for EntityAt<'_, '_> {
    fn position(&self) -> Option<[f64; 3]> {
        if let Some(position) = self.written {
            return Some(position);
        }
        match self.target {
            Some(target) => self.game.target_position(target),
            None => self.tracked.map(|(_, position)| position),
        }
    }

    fn rotation(&self) -> Option<[f32; 2]> {
        Some(self.game.target_rotation(self.target?))
    }

    fn kind(&self) -> Option<String> {
        match self.target {
            Some(target) => Some(self.game.target_type(target)),
            None => self.tracked.map(|(kind, _)| kind.to_owned()),
        }
    }

    fn level(&self) -> Option<LevelRef> {
        let game = self.game;
        let key = match self.target {
            Some(target) if game.is_logical_server() => game.target_level(target).key(),
            Some(_) => game.native_level_key(),
            None => self.tracked.map(|_| game.native_level_key())?,
        };
        Some(game.level_ref(self.entity.epoch(), key))
    }
}

impl<'g> EntityRead for Game<'g> {
    type View<'a>
        = EntityAt<'a, 'g>
    where
        Self: 'a;

    fn resolve(&self, entity: EntityRef) -> Option<EntityAt<'_, 'g>> {
        let written = if self.native_lifecycle.epoch() == Some(entity.epoch())
            && let Some((id, position)) = self.native_lifecycle.entity_sections.write_position
            && id == entity.id()
        {
            Some(position)
        } else {
            None
        };
        let target = self.native_entity_read_target(entity);
        let tracked = match target {
            Some(_) => None,
            None => self.native_client_tracked(entity),
        };
        (written.is_some() || target.is_some() || tracked.is_some()).then_some(EntityAt {
            game: self,
            entity,
            written,
            target,
            tracked,
        })
    }
}

/// A saved UUID's four ints (`UUIDUtil.uuidFromIntArray`) as one number.
pub(super) fn uuid_from_words(words: [i32; 4]) -> u128 {
    words
        .iter()
        .fold(0, |uuid, w| (uuid << 32) | u128::from(*w as u32))
}

impl EntityIdentity for Game<'_> {
    fn uuid_state(&self, entity: EntityRef) -> Option<UuidState> {
        let known = |words: [i32; 4]| UuidState::Known(uuid_from_words(words));
        if let Some(r) = &self.remote {
            let id = i32::try_from(entity.id()).ok()?;
            if self.native_entity_read_target(entity).is_none()
                && self.native_client_tracked(entity).is_none()
            {
                return None;
            }
            if let Some(words) =
                self.known_resident_entity_uuid_at(self.native_level_key(), entity.id())
            {
                return Some(known(words));
            }
            if let Some((uuid, _)) = r.level_player(id) {
                return Some(UuidState::Known(uuid));
            }
            return self.persist.uuids.get(&entity.id()).copied().map(known);
        }
        match self.native_entity_target(entity)? {
            Target::Player => {
                let dimension = self.current_player_resident_location()?.owner.dimension;
                self.known_resident_entity_uuid_at(dimension, entity.id())
                    .map(known)
            }
            Target::Guest(conn) => self.command_guest(conn).map(|g| UuidState::Known(g.uuid)),
            _ => Some(
                self.known_resident_entity_uuid_at(self.dim.key(), entity.id())
                    .map_or(UuidState::Unassigned, known),
            ),
        }
    }

    fn assign_uuid(&mut self, entity: EntityRef) -> u128 {
        uuid_from_words(self.uuid_for(entity.id()))
    }
}

impl LevelRead for Game<'_> {
    fn level_of(&self, dimension: &str) -> Option<LevelRef> {
        self.level(dimension)
    }

    fn block_has_tag(&self, pos: [i32; 3], tag: &str) -> bool {
        let block = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]));
        self.named_block_in_tag(tag, &block.name)
    }

    fn clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
    ) -> Option<ClipHit> {
        self.level_clip(level, from, to, blocks, fluids)
    }
}

impl LevelWrite for Game<'_> {
    fn set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError> {
        self.level_set_block(level, pos, state, flags)
    }
}
