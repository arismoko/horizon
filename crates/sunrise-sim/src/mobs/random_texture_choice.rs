//! Portable OptiFine random-texture per-mob sighting and pick state.
use std::sync::atomic::{AtomicBool, Ordering};

/// Weather at the mob's first sighting, matching `sunrise_optifine::random::Weather`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomTextureWeather {
    Clear,
    Rain,
    Thunder,
}

/// Where the mob was when the client first saw it.
#[derive(Clone, Debug)]
pub struct RandomTextureSeen {
    /// `minecraft:plains`; `None` in an unloaded chunk.
    pub biome: Option<String>,
    pub y: i32,
    /// The block it stood on and the block it stood in: ids and state properties.
    pub blocks: Vec<(String, Vec<(String, String)>)>,
    pub moon_phase: i32,
    pub day_time: i32,
    pub weather: RandomTextureWeather,
}

/// A mob's random-texture variants.
#[derive(Debug, Default)]
pub struct RandomTextureChoice {
    pub seen: Option<RandomTextureSeen>,
    pub seed: i32,
    /// The `RandomTextures::generation` the picks are from: another (a pack reload) makes
    /// them stale.
    pub generation: u64,
    /// The variant each rule file chose, by its index in `RandomTextures::rules`.
    pub picks: Vec<(usize, u32)>,
    /// The body texture's variant.
    pub primary: Option<u32>,
    /// Ticks until the next check of the facts that change.
    pub countdown: u8,
    /// Drawing found a texture with rules and no pick: choose on the next tick, unless a miss
    /// was answered since the last regular check.
    pub missed: AtomicBool,
    /// A miss was answered since the last regular check: one per check, so a texture that
    /// drawing keeps missing (one `drawn_textures` doesn't list) can't make the mob choose
    /// every tick.
    pub answered: bool,
}

impl Clone for RandomTextureChoice {
    fn clone(&self) -> Self {
        Self {
            seen: self.seen.clone(),
            seed: self.seed,
            generation: self.generation,
            picks: self.picks.clone(),
            primary: self.primary,
            countdown: self.countdown,
            missed: AtomicBool::new(self.missed.load(Ordering::Relaxed)),
            answered: self.answered,
        }
    }
}

/// Ticks between checks of the facts that change.
const RECHECK: u8 = 20;

impl RandomTextureChoice {
    /// Whether the mob's first sighting is recorded.
    pub fn has_seen(&self) -> bool {
        self.seen.is_some()
    }

    /// Called once a tick: whether `random_textures::choose` should run now.
    pub fn due_generation(&mut self, generation: u64, stagger: u32) -> bool {
        let fresh = !self.has_seen() || self.generation != generation;
        if fresh || self.countdown == 0 {
            self.countdown = if fresh {
                (stagger % u32::from(RECHECK)) as u8
            } else {
                RECHECK - 1
            };
            self.answered = false;
            self.missed.store(false, Ordering::Relaxed);
            return true;
        }
        self.countdown -= 1;
        if self.missed.swap(false, Ordering::Relaxed) && !self.answered {
            self.answered = true;
            return true;
        }
        false
    }
}
