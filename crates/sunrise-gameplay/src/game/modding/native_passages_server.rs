use mod_api::{LevelRef, Passage, PassageId, PassageRegistrationError as Refused, Vec3};
use sunrise_gameplay_common::game::native_passages::{
    Dirty, MAX_PER_CHUNK, MAX_PER_LEVEL, check, form, passage_carried, passage_chunk_of,
    passage_turned, passage_within_reach, passage_yaw_turn,
};
use sunrise_protocol::passage_sync::Message;

impl crate::game::ServerGame<'_> {
    /// `LevelRef::register_passage`: checks `passage`, numbers it and keeps it under its
    /// chunk; the clients holding that chunk hear of it in `flush_passages`.
    pub fn level_register_passage(
        &mut self,
        level: LevelRef,
        passage: Passage,
    ) -> Result<PassageId, Refused> {
        self.native_level_world_result(level)?;
        check(&passage)?;
        if let Some(to) = passage.destination_level {
            let open =
                mod_api::__private::dimension_id(to).is_some_and(|id| self.level(id).is_some());
            if to == level.dimension() || !open {
                return Err(Refused::UnknownLevel);
            }
        }
        let chunk = passage.chunk();
        let passages = &mut self.common.passages;
        let held = passages.levels.entry(level.dimension()).or_default();
        if held.count >= MAX_PER_LEVEL {
            return Err(Refused::LevelFull {
                limit: MAX_PER_LEVEL,
            });
        }
        if held.chunks.get(&chunk).map_or(0, Vec::len) >= MAX_PER_CHUNK as usize {
            return Err(Refused::ChunkFull {
                limit: MAX_PER_CHUNK,
            });
        }
        let Some(next) = passages.last.checked_add(1) else {
            return Err(Refused::NoIdsLeft);
        };
        passages.last = next;
        let id = PassageId::from_raw(next);
        held.chunks.entry(chunk).or_default().push((id, passage));
        held.count += 1;
        passages.index.insert(id, (level.dimension(), chunk));
        passages.dirty.insert(id, Dirty::Added { told: false });
        Ok(id)
    }

    /// `LevelRef::remove_passage`: true when `id` was one of the level's passages. The
    /// clients holding its chunk hear it is gone in `flush_passages`, unless they were never
    /// told of it.
    pub fn level_remove_passage(&mut self, level: LevelRef, id: PassageId) -> bool {
        if self.native_level_world_result(level).is_err() {
            return false;
        }
        let passages = &mut self.common.passages;
        let key = level.dimension();
        let Some(&(at, chunk)) = passages.index.get(&id).filter(|(at, _)| *at == key) else {
            return false;
        };
        passages.index.remove(&id);
        if let Some(held) = passages.levels.get_mut(&at) {
            if let Some(list) = held.chunks.get_mut(&chunk) {
                list.retain(|(held, _)| *held != id);
                if list.is_empty() {
                    held.chunks.remove(&chunk);
                }
            }
            held.count -= 1;
        }
        if passages.dirty.remove(&id) != Some(Dirty::Added { told: false }) {
            passages.dirty.insert(id, Dirty::Removed(at, chunk));
        }
        true
    }

    /// The changes since the last call, by number, told to the Sunrise clients that track
    /// the passage's chunk (`ChunkMap.isChunkTracked`). While a passage lives, the hub
    /// records the chunks it sends (`ChunkWatchLog::record_for_passages`), a new hub after
    /// the LAN reopens too, so a client that gets the chunk later hears of it with the
    /// chunk. Kept for the next call while the hub is lent out.
    pub fn flush_passages(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let hub = &mut host.hub;
        if !self.common.passages.index.is_empty() {
            hub.chunk_watches.record_for_passages();
        }
        if self.common.passages.dirty.is_empty() {
            return;
        }
        let passages = &mut self.common.passages;
        let mut changes: Vec<_> = passages.dirty.drain().collect();
        changes.sort_unstable_by_key(|(id, _)| *id);
        for (id, dirty) in changes {
            let (key, chunk, message) = match dirty {
                Dirty::Added { .. } => {
                    let Some(&(key, chunk)) = passages.index.get(&id) else {
                        continue;
                    };
                    let Some(p) = passages.find(key, id) else {
                        continue;
                    };
                    if p.destination_level.is_some() {
                        continue;
                    }
                    (key, chunk, Message::Add(vec![form(id, &p)]))
                }
                Dirty::Removed(key, chunk) => (key, chunk, Message::Remove(vec![id.raw()])),
            };
            let Some(dimension) = mod_api::__private::dimension_id(key) else {
                continue;
            };
            for g in hub.guests.iter() {
                if g.tracks_chunk(dimension, (chunk[0], chunk[1])) {
                    g.send_passage_sync(&message);
                }
            }
        }
    }

    /// `chunk` of `level` went to player `uuid` (the hub's `chunk_sent`): its passages go
    /// after it, if it has any and the player still tracks it there.
    pub(in crate::game) fn send_chunk_passages(
        &mut self,
        level: LevelRef,
        uuid: u128,
        chunk: [i32; 2],
    ) {
        let Some(list) = self
            .common
            .passages
            .levels
            .get(&level.dimension())
            .and_then(|l| l.chunks.get(&chunk))
        else {
            return;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.uuid == uuid) else {
            return;
        };
        if !g.tracks_chunk(level.dimension_id(), (chunk[0], chunk[1])) {
            return;
        }
        let told: Vec<_> = list
            .iter()
            .filter(|(_, p)| p.destination_level.is_none())
            .collect();
        if told.is_empty() {
            return;
        }
        let forms = told.iter().map(|(id, p)| form(*id, p)).collect();
        g.send_passage_sync(&Message::Add(forms));
        for (id, _) in told {
            if let Some(Dirty::Added { told }) = self.common.passages.dirty.get_mut(id) {
                *told = true;
            }
        }
    }

    /// A guest's crossing that passed the hub's move check (`Hub::passage_leg`), decided
    /// from the `from` it names (`decide_passage_cross`).
    pub(in crate::game) fn guest_passage_cross(&mut self, conn: u32, data: &[u8]) {
        use sunrise_protocol::passage_sync::decode_cross;
        use sunrise_sim::lan::hub::passage_cross::crossing_source;

        let Ok(cross) = decode_cross(data) else {
            return;
        };
        let id = PassageId::from_raw(cross.id);
        let Some(gi) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
        else {
            return;
        };
        let from = crossing_source(cross.from);
        let decided = self.decide_passage_cross(gi, id, from);
        let host = self
            .server
            .lan
            .as_mut()
            .expect("the guest was found in the hub");
        match decided {
            Ok((p, to)) => host
                .hub
                .follow_passage(gi, to, passage_yaw_turn(&p), |v| passage_turned(&p, v)),
            Err(why) => host.hub.refuse_passage(gi, cross.id, why),
        }
    }

    fn decide_passage_cross(
        &mut self,
        gi: usize,
        id: PassageId,
        from: [f64; 3],
    ) -> Result<(Passage, [f64; 3]), mod_api::PassageCrossRefusal> {
        use mod_api::PassageCrossRefusal as Refusal;
        self.check_passage_cross(gi, id, from)?;
        let host = self.server.lan.as_ref().expect("checked");
        let g = &host.hub.guests[gi];
        let epoch = self
            .common
            .native_lifecycle
            .epoch()
            .ok_or(Refusal::UnknownPassage)?;
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, g.entity_id as u32),
            g.uuid,
        );
        let mut cancel = false;
        let at = Vec3::new(from[0], from[1], from[2]);
        self.native_player_passage_cross(player, id, at, &mut cancel);
        if cancel {
            return Err(Refusal::Vetoed);
        }
        let p = self.check_passage_cross(gi, id, from)?;
        let to = passage_carried(&p, from);
        if !self.guest_holds_chunk(gi, passage_chunk_of(to)) {
            return Err(Refusal::FarEndNotTracked);
        }
        Ok((p, to))
    }

    fn check_passage_cross(
        &self,
        gi: usize,
        id: PassageId,
        from: [f64; 3],
    ) -> Result<Passage, mod_api::PassageCrossRefusal> {
        use mod_api::PassageCrossRefusal as Refusal;
        use sunrise_sim::lan::hub::passage_cross::REACH;
        let host = self.server.lan.as_ref().ok_or(Refusal::UnknownPassage)?;
        let g = &host.hub.guests[gi];
        if let Some(why) = host.hub.passage_refusal(gi) {
            return Err(why);
        }
        if g.position != from {
            return Err(Refusal::MoveRefused);
        }
        let level = self.level(&g.dimension).ok_or(Refusal::UnknownPassage)?;
        let p = self
            .common
            .passages
            .find(level.dimension(), id)
            .ok_or(Refusal::UnknownPassage)?;
        if p.destination_level.is_some() {
            return Err(Refusal::FarLevel);
        }
        if !passage_within_reach(&p, from) {
            return Err(Refusal::TooFar { limit: REACH });
        }
        let [cx, cz] = p.chunk();
        if !self.guest_holds_chunk(gi, (cx, cz)) {
            return Err(Refusal::NearEndNotTracked);
        }
        let d = p.destination;
        if !self.guest_holds_chunk(gi, passage_chunk_of([d.x, d.y, d.z])) {
            return Err(Refusal::FarEndNotTracked);
        }
        Ok(p)
    }

    fn guest_holds_chunk(&self, gi: usize, chunk: (i32, i32)) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        let g = &host.hub.guests[gi];
        g.tracks_chunk(&g.dimension, chunk)
            && g.dimension == self.common.dim.id()
            && self.common.world.chunks.contains_key(&chunk)
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
