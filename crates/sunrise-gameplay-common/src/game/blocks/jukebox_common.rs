//! Common jukebox helpers shared by server simulation and client playback/prediction.
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::{BlockEntity, Jukebox};
use sunrise_sim::mobs::Species;

/// The stack's `jukebox_playable` component (`JukeboxPlayable`), else the item's default one.
pub fn jukebox_playable(
    record: &ItemStack,
    items: &sunrise_items::items::Items,
) -> Option<Option<String>> {
    let data = record.data();
    if data
        .and_then(|d| d.other("!minecraft:jukebox_playable"))
        .is_some()
    {
        return None;
    }
    match data.and_then(|d| d.other("minecraft:jukebox_playable")) {
        Some(sunrise_core::nbt::Tag::String(song)) => Some(Some(song.to_string_lossy())),
        Some(_) => Some(None),
        None => Some(Some(items.get(&record.item)?.jukebox_song.clone()?)),
    }
}

/// `JukeboxSong.fromStack`: the song the disc names, by its id.
pub fn song_of_record(record: &ItemStack, items: &sunrise_items::items::Items) -> Option<String> {
    jukebox_playable(record, items).flatten()
}

/// The jukebox's redstone signal (15 while a song plays) and comparator output.
pub fn jukebox_signals(
    world: &sunrise_level::world::World,
    items: &sunrise_items::items::Items,
    pos: BlockPos,
) -> Option<(i32, i32)> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Jukebox(j)) => {
            let song = j.record().and_then(|r| song_of_record(r, items));
            let comparator = song
                .as_deref()
                .and_then(|s| items.songs.get(s).map(|song| song.comparator))
                .unwrap_or(0);
            let stale = j.loaded_clock
                && song
                    .as_deref()
                    .and_then(|s| items.songs.get(s))
                    .is_some_and(|s| s.finished(j.ticks));
            Some((if j.playing && !stale { 15 } else { 0 }, comparator))
        }
        _ => None,
    }
}

impl crate::game::CommonGame<'_> {
    pub fn jukebox(&self, pos: [i32; 3]) -> Option<&Jukebox> {
        match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
            Some(BlockEntity::Jukebox(j)) => Some(j),
            _ => None,
        }
    }

    /// `JukeboxBlock.HAS_RECORD`, the state both clicks are decided by.
    pub(in crate::game) fn jukebox_has_record(&self, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("has_record"))
            .is_some_and(|v| v == "true")
    }

    /// `JukeboxBlock.useItemOn`: whether the jukebox at `pos` takes the click.
    pub fn jukebox_takes(&self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:jukebox" {
            return false;
        }
        self.jukebox_has_record(pos)
            || held.is_some_and(|stack| jukebox_playable(stack, self.res.items()).is_some())
    }

    /// `LevelEventHandler.notifyNearbyEntities`: parrots within three blocks dance or stop.
    pub fn notify_record_playing(&mut self, pos: [i32; 3], playing: bool) {
        let min = Vec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
        let area = Aabb::new(min, min.add_vector(Vec3::new(1.0, 1.0, 1.0)))
            .inflate(Vec3::new(3.0, 3.0, 3.0));
        for m in self
            .mobs
            .iter_mut()
            .filter(|m| m.species() == Species::Parrot && m.bounds().intersects(area))
        {
            m.extra.party_jukebox = playing.then_some(pos);
        }
    }
}
