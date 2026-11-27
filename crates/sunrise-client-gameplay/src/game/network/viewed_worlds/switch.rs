//! A travel into a viewed level (#3716, 9e): the Respawn of `ServerPlayer.teleport(TeleportTransition)`
//! (keep 3, all data) into a level the client views swaps the two worlds, as Immersive Portals'
//! `ClientTeleportationManager.changePlayerDimension` moves the client's world reference: the
//! viewed world's columns and block entities become the player's, the player's world becomes the
//! viewed world of the level left with all it holds, and no loading screen starts. The server sends
//! neither level's chunks again; it sends both levels' entities again, as the client drops them.
//!
//! Revision 4 of `sunrise:levels` binds the swap explicitly: a kind-3 marker immediately after
//! Respawn names the slot and epoch holding the entered world and the epoch for the world left.
//! This client validates that binding before swapping. A death, End exit, or ordinary travel
//! without a matching binding drops the views and enters the level normally. Older served
//! clients retain their revision's implicit swap behavior; this client answers revision 4.
use super::{routing::default_clock, worlds::ViewedWorld};
use crate::game::ClientGame;
use crate::game::client_side::remote_client::RemoteDimensionEntry;
use std::sync::Arc;
use sunrise_client::level_loading::Reason;
use sunrise_protocol::packets::{
    Empty,
    play::{self, Serverbound as S},
};

/// `ClientboundRespawnPacket.KEEP_ALL_DATA`: the Respawn of a travel.
const KEEP_ALL_DATA: i8 = 3;

/// A travel into a viewed level: its world, taken out of the viewed worlds, and its slot, where
/// the server's `swap` binds the level left (`sunrise_protocol::viewed_levels`).
struct Travel {
    entered: ViewedWorld,
    slot: i32,
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `handleRespawn`'s new level: a travel into a viewed level swaps the worlds; any other
    /// Respawn drops every viewed world (the server opens again what it still views) and enters
    /// the level with its loading screen.
    pub(in crate::game::client_side) fn enter_respawn_level(
        &mut self,
        p: &play::Respawn,
        entry: RemoteDimensionEntry,
        reason: Reason,
    ) {
        let Some(travel) = self.take_travel(p) else {
            if let Some(r) = self.client.remote.as_mut() {
                r.viewed.clear();
            }
            self.enter_remote_dimension(&p.spawn, entry, reason);
            return;
        };
        self.keep_as_viewed(travel.slot, travel.entered.columns, p.spawn.seed);
        self.enter_remote_dimension(&p.spawn, entry, reason);
        self.adopt_viewed_world(travel.entered);
    }

    /// The viewed world `p` travels into, taken out, if it is one: a Respawn keeping all data
    /// into a level the client views, of the shape it was opened with.
    fn take_travel(&mut self, p: &play::Respawn) -> Option<Travel> {
        if p.keep != KEEP_ALL_DATA {
            return None;
        }
        let r = self.client.remote.as_mut()?;
        if p.spawn.dimension == r.level {
            return None;
        }
        let slot = r.viewed.slot_of(&p.spawn.dimension)?;
        let opened = r.viewed.get_mut(slot)?.world.shape;
        // A level whose dimension type changed since its `open` (a server's error) is entered
        // afresh: its columns would not fit.
        if r.level_format(&p.spawn).shape != opened {
            return None;
        }
        let entered = r.viewed.take(slot)?;
        Some(Travel { entered, slot })
    }

    /// The player's world becomes the viewed world of the level left, whole: its columns, light
    /// and block entities, of at most `cap` columns, the cap of the level entered. It waits
    /// for the server's `swap` to bind it to `slot`. Its columns leave the player's level for the
    /// mods, as when it goes (`forget_all_remote_columns`); a placeholder stands until
    /// `enter_remote_dimension` makes the new level's world.
    fn keep_as_viewed(&mut self, slot: i32, cap: u32, seed: i64) {
        // Hidden ring slots are not part of the view the server is replacing this level with.
        let mut hidden = self
            .common
            .world
            .chunks
            .resident_keys()
            .filter(|key| !self.common.world.chunks.contains_key(key))
            .collect();
        self.forget_remote_columns(&mut hidden, false);
        let mut columns: Vec<(i32, i32)> = self.common.world.chunks.keys().copied().collect();
        columns.sort_unstable();
        for (x, z) in columns {
            self.client_chunk_unload(x, z, false);
        }
        // The mods' extra block models were for the player's level.
        self.common.world.clear_extra_models();
        let air = self.common.res.facts().air;
        let world = std::mem::replace(
            &mut self.common.world,
            sunrise_gameplay_common::game::world_for_mods(air, seed),
        );
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let clock = default_clock(r, &r.dimension);
        let mut kept = ViewedWorld::new(r.level.clone(), r.chunk_format.clone(), clock, world, cap);
        kept.nether_shading = self.client.streaming.nether_shading;
        r.viewed.hold_left(slot, kept);
    }

    /// The new level's world takes the viewed world's columns, with their light, and its block
    /// entities: those a world that was the player's kept, each following the block now there,
    /// then those its chunk packets carried, with the data sent since. Each column then loads for
    /// the mods, as a chunk packet's does. The level is on the client already: no loading screen
    /// waits for it, and the server hears at once that it is loaded.
    fn adopt_viewed_world(&mut self, entered: ViewedWorld) {
        let ViewedWorld {
            mut world,
            block_entities: mut sent,
            ..
        } = entered;
        self.common.world.weather = world.weather;
        let mut columns: Vec<(i32, i32)> = world.chunks.keys().copied().collect();
        columns.sort_unstable();
        for key in &columns {
            if let Some(chunk) = world.chunks.remove(key) {
                self.client
                    .streaming
                    .insert_remote(&mut self.common.world, Arc::unwrap_or_clone(chunk));
            }
        }
        let mut kept: Vec<(i32, i32, i32)> = world.block_entities.keys().copied().collect();
        kept.sort_unstable();
        if !kept.is_empty() {
            let _load = sunrise_level::spans::raw_flight_guard("load_block_entities");
            for pos in kept {
                if let Some(entity) = world.block_entities.remove(&pos) {
                    self.common.world.block_entities.insert(pos, entity);
                    sunrise_level::spans::note_block_entity_loads(1);
                    let state = self.common.world.block(pos.0, pos.1, pos.2);
                    self.sync_block_entity(pos, None, state);
                }
            }
        }
        for key in columns {
            if let Some(infos) = sent.remove(&key) {
                self.load_block_entities(key, &infos);
            }
            self.ask_chunk_light_emission(key);
            self.client_chunk_load(key.0, key.1);
        }
        if let Some(r) = self.client.remote.as_mut() {
            r.awaiting_viewed_chunk_center = true;
            r.level_load.finish();
        }
        self.send(S::PlayerLoaded(Empty));
    }
}

use sunrise_gameplay_common::game::light_emission::LightEmissionGameExt1 as _;
