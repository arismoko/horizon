//! The `horizon:levels` play payloads, routed: an `open` builds the level's world from the
//! dimension type the server synced for it, a `run`'s packets go to that world, and a `close`
//! drops it. A payload the protocol refuses closes the connection with its named reason.
use super::worlds::{MAX_COLUMNS, ViewedWorld};
use crate::game::Game;
use horizon_client::{text::Text, viewed_levels_net::MAX_LEVELS, world::World};
use horizon_protocol::{
    packets::play::{Clientbound as C, SpawnInfo},
    viewed_levels::{CHANNEL, Message, ViewedLevelsError},
};
use horizon_world::dimension_type::DimensionFacts;

/// Why a `horizon:levels` play payload closes the connection.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum RouteError {
    /// The payload's bytes, refused by the codec.
    Codec(ViewedLevelsError),
    /// An `open` of a level that is not the one at its index in the Login packet's `levels`. The
    /// reason names neither: the dimension is the server's text.
    UnknownLevel,
    /// An `open` of the player's own level.
    OwnLevel(i32),
    /// An `open` past the levels the client answered it can view.
    OverCap { level: i32, cap: u8 },
    /// A `run` or `close` of a level not open.
    NotOpen(i32),
    /// A `run`'s chunk of a new column in a world that holds `worlds::MAX_COLUMNS` already.
    TooManyColumns,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => write!(f, "{error}"),
            Self::UnknownLevel => {
                write!(f, "{CHANNEL}: open does not match the Login level index")
            }
            Self::OwnLevel(level) => write!(f, "{CHANNEL}: level {level} is the player's own"),
            Self::OverCap { level, cap } => {
                write!(
                    f,
                    "{CHANNEL}: an open of level {level} past the client's {cap} levels"
                )
            }
            Self::NotOpen(level) => write!(f, "{CHANNEL}: level {level} is not open"),
            Self::TooManyColumns => {
                write!(f, "{CHANNEL}: a viewed level past {MAX_COLUMNS} columns")
            }
        }
    }
}

impl From<ViewedLevelsError> for RouteError {
    fn from(error: ViewedLevelsError) -> Self {
        Self::Codec(error)
    }
}

impl Game<'_> {
    /// A `horizon:levels` play payload's data: routed, or the connection closes with the reason.
    pub(in crate::game) fn viewed_levels_payload(
        &mut self,
        data: &[u8],
    ) -> crate::game::Result<()> {
        let Err(error) = self.route_viewed_levels(data) else {
            return Ok(());
        };
        log::warn!("Horizon: {error}");
        let reason = Text::translate(
            "disconnect.genericReason",
            &[Text::literal(error.to_string())],
        );
        self.leave_server(Some(reason.plain()))
    }

    pub(in crate::game) fn route_viewed_levels(&mut self, data: &[u8]) -> Result<(), RouteError> {
        match Message::decode(data)? {
            Message::Open { level, spawn } => self.open_viewed_level(level, &spawn),
            Message::Run { level, packets } => {
                let Some(world) = self.remote.as_mut().and_then(|r| r.viewed.get_mut(level)) else {
                    return Err(RouteError::NotOpen(level));
                };
                for (id, body) in packets {
                    // The codec read each one exactly already.
                    let packet =
                        C::decode(id, body).map_err(|_| ViewedLevelsError::MalformedPacket(id))?;
                    world.apply(packet)?;
                }
                Ok(())
            }
            Message::Close { level } => {
                let closed = self.remote.as_mut().is_some_and(|r| r.viewed.remove(level));
                closed.then_some(()).ok_or(RouteError::NotOpen(level))
            }
        }
    }

    /// `open`: a world for `level`, empty, with the shape and facts of the dimension type the
    /// server synced for it. An `open` of a level open already starts it afresh.
    fn open_viewed_level(&mut self, level: i32, spawn: &SpawnInfo) -> Result<(), RouteError> {
        let air = self.res.facts().air;
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let named = usize::try_from(level).ok().and_then(|i| r.levels.get(i));
        if named != Some(&spawn.dimension) {
            return Err(RouteError::UnknownLevel);
        }
        if spawn.dimension == r.level {
            return Err(RouteError::OwnLevel(level));
        }
        if !r.viewed.contains(level) && r.viewed.len() >= usize::from(MAX_LEVELS) {
            return Err(RouteError::OverCap {
                level,
                cap: MAX_LEVELS,
            });
        }
        let format = r.level_format(spawn, self.res.environment());
        let default_clock = format
            .dimension
            .default_clock
            .as_deref()
            .and_then(|name| r.registries.id("minecraft:world_clock", name));
        let facts = format.facts.clone().unwrap_or_else(|| {
            let key = horizon_core::LevelKey::parse(&spawn.dimension)
                .unwrap_or(horizon_core::LevelKey::OVERWORLD);
            std::sync::Arc::new(DimensionFacts::of_builtin_level(key).clone())
        });
        let mut world = World::new(air, spawn.seed);
        world.shape = format.shape;
        world.fast_lava = facts.fast_lava;
        world.infiniburn = Some(self.resolve_level_infiniburn(&facts));
        let world = ViewedWorld::new(
            spawn.dimension.clone(),
            format.chunk_format,
            default_clock,
            world,
        );
        if let Some(r) = self.remote.as_mut() {
            r.viewed.insert(level, world);
        }
        Ok(())
    }
}
