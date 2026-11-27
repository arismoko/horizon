//! The `sunrise:levels` play payloads, routed: an `open` builds the level's world from the
//! dimension type the server synced for it, a `run`'s packets go to that world, and a `close`
//! drops it. A payload the protocol refuses closes the connection with its named reason.
use super::{
    entities::MAX_TABLE,
    worlds::{Binding, ViewedWorld, stored_state},
};
use crate::game::remote_chunks::section_blocks;
use sunrise_items::blocks::BlockFacts;
use sunrise_level::world::World;
use sunrise_protocol::{
    Error,
    connection_groups::Section,
    packets::play::{Clientbound as C, SpawnInfo},
    registry::DimensionType,
    viewed_levels::{
        CHANNEL, Level, MAX_COLUMNS, MAX_ENTITIES, Message, REVISION, SOUNDS, ScopedPacket,
        ViewedLevelsError,
    },
};
use sunrise_sim::viewed_levels_net::MAX_LEVELS;
use sunrise_world::{
    dimension_type::{CardinalLight, DimensionFacts},
    registry::BlockStateId,
};
use {
    crate::game::client_side::remote_client::Remote,
    crate::game::native_connection_groups::block_groups,
    crate::game::native_connection_groups::check_group_updates,
};

/// Why a `sunrise:levels` play payload closes the connection.
#[derive(Clone, Debug, PartialEq)]
pub enum RouteError {
    /// The payload's bytes, refused by the codec.
    Codec(ViewedLevelsError),
    Groups(Error),
    /// A level named by a Login index, which this client's revision does not use. (An `open`
    /// whose dimension is not a level id is the codec's `BadOpen`.)
    UnknownLevel,
    /// An `open` of a dimension type the server did not sync.
    UnsyncedType,
    /// An `open` of a level the client views already in another slot.
    ViewedTwice(i32),
    /// An `open` of the player's own level.
    OwnLevel(i32),
    /// A slot past the levels the client answered it can view.
    OverCap {
        level: i32,
        cap: u8,
    },
    /// An `open` whose epoch is not newer than every epoch given before.
    OldEpoch(i32),
    /// A `run` or `close` at an epoch newer than any `open` gave.
    UnknownEpoch {
        level: i32,
        epoch: i32,
    },
    /// A `run`'s chunk of a new column in a world that holds its cap of columns already.
    TooManyColumns(u32),
    /// A `far` past `viewed_levels::MAX_COLUMNS` columns of the player's level kept.
    TooManyFar,
    /// An `open` whose column cap passes the client's ceiling, `viewed_levels::MAX_COLUMNS`.
    CapPastCeiling(u32),
    /// A `swap` the client made no travel swap for.
    UnexpectedSwap,
    /// A `run`'s `AddEntity` of a new entity in a world that holds `MAX_ENTITIES` already.
    TooManyEntities,
    /// An entity's effect or attribute of a new kind past `entities::MAX_TABLE` of them.
    EntityTableFull,
    /// A `SetTime` whose game time, or whose clock's total moved on to it, passes the range of a
    /// long.
    ClockOverflow,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => write!(f, "{error}"),
            Self::Groups(error) => write!(f, "{error}"),
            Self::UnknownLevel => write!(f, "{CHANNEL}: a level named by a Login index"),
            Self::UnsyncedType => {
                write!(
                    f,
                    "{CHANNEL}: open names a dimension type the server did not sync"
                )
            }
            Self::ViewedTwice(level) => {
                write!(
                    f,
                    "{CHANNEL}: an open in slot {level} of a level viewed already"
                )
            }
            Self::OwnLevel(level) => write!(f, "{CHANNEL}: level {level} is the player's own"),
            Self::OverCap { level, cap } => {
                write!(
                    f,
                    "{CHANNEL}: an open of level {level} past the client's {cap} levels"
                )
            }
            Self::OldEpoch(epoch) => {
                write!(
                    f,
                    "{CHANNEL}: an open of epoch {epoch}, not newer than the last"
                )
            }
            Self::UnknownEpoch { level, epoch } => {
                write!(
                    f,
                    "{CHANNEL}: level {level} at epoch {epoch}, which no open gave"
                )
            }
            Self::TooManyColumns(cap) => {
                write!(f, "{CHANNEL}: a viewed level past {cap} columns")
            }
            Self::TooManyFar => {
                write!(f, "{CHANNEL}: far columns past {MAX_COLUMNS}")
            }
            Self::CapPastCeiling(cap) => write!(
                f,
                "{CHANNEL}: an open of {cap} columns, past the client's {MAX_COLUMNS}"
            ),
            Self::UnexpectedSwap => write!(f, "{CHANNEL}: a swap the client did not make"),
            Self::TooManyEntities => {
                write!(f, "{CHANNEL}: a viewed level past {MAX_ENTITIES} entities")
            }
            Self::EntityTableFull => {
                write!(
                    f,
                    "{CHANNEL}: an entity past {MAX_TABLE} effects or attributes"
                )
            }
            Self::ClockOverflow => write!(f, "{CHANNEL}: a viewed level's time past its range"),
        }
    }
}

impl From<ViewedLevelsError> for RouteError {
    fn from(error: ViewedLevelsError) -> Self {
        Self::Codec(error)
    }
}

/// Whether a block of `state` has a block entity (`EntityBlock`), as `block_entity_for_block`
/// decides it for the player's world: a declared type's block, a sign, or a vanilla block entity's
/// block. A mod's state-aware block is asked at the travel, where its block entity is made.
fn block_entity_block(facts: &BlockFacts, state: BlockStateId) -> bool {
    let block = facts.get(state);
    sunrise_game_data::block_entity_types::type_for(&block.name).is_some()
        || block.sign_form().is_some()
        || sunrise_level::containers::BlockEntity::for_block(&block.name).is_some()
}

/// Where a block update puts another block than `world` holds, and its state: the block entity
/// there goes and the new block's comes (`LevelChunk.setBlockState`); a block's own state change
/// keeps it.
fn other_blocks(facts: &BlockFacts, world: &World, packet: &C) -> Vec<([i32; 3], BlockStateId)> {
    let changes: Vec<([i32; 3], i32)> = match packet {
        C::BlockUpdate(p) => vec![(p.pos, p.state)],
        C::SectionBlocksUpdate(p) => section_blocks(p).collect(),
        _ => return Vec::new(),
    };
    let name = |state| &facts.get(state).name;
    (changes.into_iter())
        .map(|(pos, state)| (pos, stored_state(state)))
        .filter(|&([x, y, z], state)| name(world.block(x, y, z)) != name(state))
        .collect()
}

/// The id of the world clock a level of `dimension` follows (its `default_clock`) in the
/// server's registry.
pub(super) fn default_clock(r: &Remote, dimension: &DimensionType) -> Option<i32> {
    let name = dimension.default_clock.as_deref()?;
    r.registries.id("minecraft:world_clock", name)
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A `sunrise:levels` play payload's data: routed, or the connection closes with the reason.
    pub(in crate::game::client_side) fn viewed_levels_payload(
        &mut self,
        data: &[u8],
    ) -> sunrise_gameplay_common::game::Result<()> {
        let Err(error) = self.route_viewed_levels(data) else {
            return Ok(());
        };
        self.refuse_remote_input(error)
    }

    /// Route the scoped variant of a completed vanilla-first group bundle.
    /// Its first run holds one vanilla companion; the remaining runs only
    /// hold group data for that same slot and epoch.
    pub(in crate::game::client_side) fn route_viewed_bundle(
        &mut self,
        runs: &[(&C, &[u8])],
        negotiated: bool,
    ) -> Result<(), RouteError> {
        let sounds = super::sounds::listened();
        let messages = runs
            .iter()
            .map(|(_, data)| Message::decode_with(data, REVISION, sounds))
            .collect::<Result<Vec<_>, _>>()?;
        let has_groups = messages.iter().any(|message| {
            matches!(message, Message::Run { packets, .. }
                if packets.iter().any(|packet| matches!(packet, ScopedPacket::ConnectionGroups(_))))
        });
        if !has_groups {
            for ((outer, _), message) in runs.iter().zip(messages) {
                self.observe_client_packet(outer);
                self.route_viewed_message(message)?;
            }
            return Ok(());
        }
        let invalid = || RouteError::Groups(Error::new("connection groups: invalid viewed bundle"));
        if !negotiated {
            return Err(RouteError::Groups(Error::new(
                "connection groups: channel was not negotiated",
            )));
        }
        let [Message::Run { level, packets }, rest @ ..] = messages.as_slice() else {
            return Err(invalid());
        };
        let [ScopedPacket::Play(id, body)] = packets.as_slice() else {
            return Err(invalid());
        };
        if rest.is_empty() {
            return Err(invalid());
        }
        let (slot, epoch) = bound(*level)?;
        let mut sections = Vec::new();
        for run in rest {
            let Message::Run {
                level: scoped,
                packets,
            } = run
            else {
                return Err(invalid());
            };
            if scoped != level {
                return Err(invalid());
            }
            for packet in packets {
                let ScopedPacket::ConnectionGroups(body) = packet else {
                    return Err(invalid());
                };
                sections.push(Section::decode(body).map_err(RouteError::Groups)?);
            }
        }
        if sections.is_empty() {
            return Err(invalid());
        }
        let packet = C::decode(*id, body).map_err(RouteError::Groups)?;
        let [(primary, _), rest @ ..] = runs else {
            return Err(invalid());
        };
        self.observe_client_packet(primary);
        if self.live(slot, epoch)? {
            self.apply_viewed_world_packet(slot, packet, &sections)?;
        }
        for (outer, _) in rest {
            self.observe_client_packet(outer);
        }
        Ok(())
    }

    pub fn route_viewed_levels(&mut self, data: &[u8]) -> Result<(), RouteError> {
        // A run's sounds are kept only for a listener: without one they are skipped unread.
        self.route_viewed_message(Message::decode_with(
            data,
            REVISION,
            super::sounds::listened(),
        )?)
    }

    fn route_viewed_message(&mut self, message: Message<'_>) -> Result<(), RouteError> {
        match message {
            Message::Open {
                level,
                columns,
                spawn,
            } => {
                let (slot, epoch) = bound(level)?;
                if columns > MAX_COLUMNS {
                    return Err(RouteError::CapPastCeiling(columns));
                }
                self.open_viewed_level(slot, epoch, columns, &spawn)
            }
            Message::Swap { level } => {
                let (slot, epoch) = bound(level)?;
                let Some(r) = self.client.remote.as_mut() else {
                    return Ok(());
                };
                if !r.viewed.is_new(epoch) {
                    return Err(RouteError::OldEpoch(epoch));
                }
                r.viewed
                    .bind_left(slot, epoch)
                    .then_some(())
                    .ok_or(RouteError::UnexpectedSwap)
            }
            Message::Run { level, packets } => {
                let (level, epoch) = bound(level)?;
                if !self.live(level, epoch)? {
                    return Ok(());
                }
                for packet in packets {
                    let ScopedPacket::Play(id, body) = packet else {
                        return Err(RouteError::Groups(Error::new(
                            "connection groups: payload outside its bundle",
                        )));
                    };
                    // A sound plays in the player's level, if a mod says where.
                    if SOUNDS.contains(&id) {
                        self.viewed_level_sound(level, id, body)?;
                        continue;
                    }
                    let packet =
                        C::decode(id, body).map_err(|_| ViewedLevelsError::MalformedPacket(id))?;
                    self.apply_viewed_world_packet(level, packet, &[])?;
                }
                Ok(())
            }
            Message::Far { kept, released } => self
                .remote_far_columns(&kept, &released)
                .then_some(())
                .ok_or(RouteError::TooManyFar),
            Message::Close { level } => {
                let (level, epoch) = bound(level)?;
                if self.live(level, epoch)?
                    && let Some(r) = self.client.remote.as_mut()
                {
                    r.viewed.remove(level);
                }
                Ok(())
            }
        }
    }

    fn apply_viewed_world_packet(
        &mut self,
        level: i32,
        packet: C,
        sections: &[Section],
    ) -> Result<(), RouteError> {
        let facts = self.common.res.facts();
        let group_limit = if sections.is_empty() {
            0
        } else {
            self.connection_group_limit()
        };
        let Some(world) = self
            .client
            .remote
            .as_mut()
            .and_then(|r| r.viewed.get_mut(level))
        else {
            return Ok(());
        };
        let (groups, snapshot) =
            if sections.is_empty() || matches!(packet, C::LevelChunkWithLight(_)) {
                (Default::default(), sections)
            } else {
                let groups =
                    block_groups(&world.world, &packet, sections).map_err(RouteError::Groups)?;
                check_group_updates(
                    &world.world,
                    groups.iter().map(|(&pos, &group)| (pos, group)),
                    group_limit,
                )
                .map_err(RouteError::Groups)?;
                (groups, &[][..])
            };
        if let C::BlockEntityData(p) = packet {
            let [x, y, z] = p.pos;
            let entity_block = block_entity_block(facts, world.world.block(x, y, z));
            world.block_entity_data(p, entity_block);
            return Ok(());
        }
        for (pos, state) in other_blocks(facts, &world.world, &packet) {
            world.forget_block_entity(pos);
            if block_entity_block(facts, state) {
                world.placed_block_entity(pos);
            }
        }
        world.apply(packet, groups, snapshot, group_limit)
    }

    /// Whether a `run` or `close` of `slot` at `epoch` goes to its world: false for one of a level
    /// the slot no longer holds, which is dropped (`ViewedWorlds::binding`).
    fn live(&self, slot: i32, epoch: i32) -> Result<bool, RouteError> {
        let Some(r) = self.client.remote.as_ref() else {
            return Ok(false);
        };
        match r.viewed.binding(slot, epoch) {
            Binding::Live => Ok(true),
            Binding::Stale => Ok(false),
            Binding::Unknown => Err(RouteError::UnknownEpoch { level: slot, epoch }),
        }
    }

    /// `open`: a world for the level `spawn` names, empty, with the shape and facts of the
    /// dimension type the server synced for it, bound to `slot` at `epoch`. An `open` of a slot
    /// bound already starts it afresh.
    fn open_viewed_level(
        &mut self,
        slot: i32,
        epoch: i32,
        columns: u32,
        spawn: &SpawnInfo,
    ) -> Result<(), RouteError> {
        let air = self.common.res.facts().air;
        let Some(r) = self.client.remote.as_mut() else {
            return Ok(());
        };
        if !r.viewed.is_new(epoch) {
            return Err(RouteError::OldEpoch(epoch));
        }
        if r.registries
            .name("minecraft:dimension_type", spawn.dimension_type)
            .is_none()
        {
            return Err(RouteError::UnsyncedType);
        }
        if r.viewed
            .slot_of(&spawn.dimension)
            .is_some_and(|other| other != slot)
        {
            return Err(RouteError::ViewedTwice(slot));
        }
        if spawn.dimension == r.level {
            return Err(RouteError::OwnLevel(slot));
        }
        let format = r.level_format(spawn);
        let default_clock = default_clock(r, &format.dimension);
        let facts = format.facts.clone().unwrap_or_else(|| {
            let key = sunrise_core::LevelKey::parse(&spawn.dimension)
                .unwrap_or(sunrise_core::LevelKey::OVERWORLD);
            std::sync::Arc::new(DimensionFacts::of_builtin_level(key).clone())
        });
        let mut world = World::new(air, spawn.seed);
        world.chunks.track_connection_groups(r.group_count.clone());
        world.shape = format.shape;
        world.fast_lava = facts.fast_lava;
        world.infiniburn = Some(self.resolve_level_infiniburn(&facts));
        world.golem_heads = Some(self.resolve_golem_heads());
        let mut world = ViewedWorld::new(
            spawn.dimension.clone(),
            format.chunk_format,
            default_clock,
            world,
            columns,
        );
        world.nether_shading = facts.cardinal_light == CardinalLight::Nether;
        if let Some(r) = self.client.remote.as_mut() {
            r.viewed.insert(slot, epoch, world);
        }
        Ok(())
    }
}

/// The slot and epoch of `level`, as this client's revision names a viewed level: a slot past
/// the levels it can view is refused, and so is a Login index, which only older revisions use.
fn bound(level: Level) -> Result<(i32, i32), RouteError> {
    match level {
        Level::Bound { slot, epoch } if slot < i32::from(MAX_LEVELS) => Ok((slot, epoch)),
        Level::Bound { slot, .. } => Err(RouteError::OverCap {
            level: slot,
            cap: MAX_LEVELS,
        }),
        Level::Index(_) => Err(RouteError::UnknownLevel),
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
