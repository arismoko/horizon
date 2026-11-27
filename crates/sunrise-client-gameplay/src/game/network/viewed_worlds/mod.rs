//! Viewed levels, the client's worlds (#3716, 9c): a world for each level a server opens on
//! `sunrise:levels`, as Immersive Portals' `ClientWorldLoader` keeps a `ClientLevel` per
//! dimension. The server wraps each packet of such a level with it (`PacketRedirection`); here
//! a `run`'s packets change that level's world and nothing of the player's own: its `World`,
//! its renderer and its sounds stay as they are. A client mod reads a viewed level's blocks
//! through `host.level` (gap 10, `modding/native_viewed_levels.rs`) and never writes it.
//! A viewed level's sound plays only where a mod's `viewed_level_sound` puts it (`sounds`).
//!
//! The client views at most `viewed_levels_net::MAX_LEVELS` levels besides its own, the number
//! its capability answer gives. A travel into one of them swaps it with the player's world
//! (`switch`); a Login, any other Respawn or a new configuration phase drops every viewed world,
//! and the server opens again what it still views. A level a world view names is meshed into a
//! set of its own (`meshing`); nothing of a viewed level is drawn yet: drawing is gap 13's world
//! view. Its entities live in its world (`entities`), apart from the player's own.
mod entities;
mod meshing;
pub mod routing;
pub mod sounds;
mod switch;
mod worlds;
pub(in crate::game) use worlds::ViewedWorlds;
