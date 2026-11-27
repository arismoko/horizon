//! The player's own level past its view (revision 5 of `sunrise:levels`): a mod's area there
//! reaches a client that answered that revision even far from the player, so that a portal can
//! show a place of the same level thousands of blocks away. Its columns go in the player's
//! ordinary chunk queue, after the view's, in plain packets: `far` tells the client first to keep
//! them wherever its chunk cache centre goes. They are tracked as the view's are
//! (`Guest::tracks_chunk`), so their block changes, passages and entities reach the client too.
//! A column no area covers any more lingers [`LINGER`] ticks, then is forgotten unless the view
//! holds it, and released. A vanilla client, or an older Sunrise one, gets the area's ticket and
//! nothing past its view.
use super::{
    super::{chunk_watch, hub::Hub, hub::tracked},
    areas::ChunkArea,
    view::{LINGER, covers, distance},
};
use crate::fast_hash::{FxHashMap, FxHashSet};
use sunrise_protocol::{
    packets::play::{self, Clientbound as C},
    viewed_levels,
};

/// One player's own level past its view (`Viewed::own`).
#[derive(Default)]
pub struct Own {
    /// The columns its client keeps past its view: every one an area covers, as many as its cap
    /// allows, and those that linger.
    pub kept: FxHashSet<(i32, i32)>,
    /// Kept columns no area covers, with the hub tick they stopped being covered.
    linger: FxHashMap<(i32, i32), u64>,
    /// The areas' centres and radii, in chunks: the client also tracks the level's entities
    /// from them (`viewed_levels::entities::in_reach`).
    pub sources: Vec<((i32, i32), i32)>,
    /// The areas changed, or a kept column went, since `kept` was made.
    pub dirty: bool,
}

impl Own {
    /// Its client dropped its level (a Respawn): nothing is kept, and the areas are kept afresh.
    pub fn reset(&mut self) {
        *self = Self {
            dirty: true,
            ..Self::default()
        };
    }

    /// `kept` made again for `areas` as of hub tick `now`, at most `cap` columns: the ones it
    /// takes in, nearest to an area's centre first (as a viewed level's queue). Kept columns no
    /// area covers start to linger; those covered again stop.
    fn rekeep(&mut self, areas: &[&ChunkArea], now: u64, cap: usize) -> Vec<(i32, i32)> {
        self.sources = areas.iter().map(|a| (a.center, a.radius as i32)).collect();
        let sources = &self.sources;
        for &(x, z) in &self.kept {
            if covers(sources, x, z) {
                self.linger.remove(&(x, z));
            } else {
                self.linger.entry((x, z)).or_insert(now);
            }
        }
        let mut new = FxHashSet::default();
        for &((cx, cz), r) in sources {
            for z in cz - r..=cz + r {
                for x in cx - r..=cx + r {
                    if !self.kept.contains(&(x, z)) {
                        new.insert((x, z));
                    }
                }
            }
        }
        let mut new: Vec<_> = new.into_iter().collect();
        new.sort_unstable_by_key(|&(x, z)| (distance(sources, x, z), x, z));
        new.truncate(cap.saturating_sub(self.kept.len()));
        self.kept.extend(new.iter().copied());
        self.dirty = false;
        new
    }

    /// The lingering columns whose [`LINGER`] ticks are up at hub tick `now`, in order, kept no
    /// more.
    fn due(&mut self, now: u64) -> Vec<(i32, i32)> {
        let mut gone: Vec<_> = self
            .linger
            .iter()
            .filter(|&(_, &since)| now.saturating_sub(since) >= LINGER)
            .map(|(&key, _)| key)
            .collect();
        gone.sort_unstable();
        for key in &gone {
            self.linger.remove(key);
            self.kept.remove(key);
        }
        gone
    }
}

/// The columns an area keeps that are neither sent nor queued, put after `queue`, nearest to an
/// area's centre first. A column that only lingers is not sent.
pub(in crate::lan) fn queue_far(
    queue: &mut Vec<(i32, i32)>,
    sent: &FxHashSet<(i32, i32)>,
    own: &Own,
) {
    if own.kept.is_empty() {
        return;
    }
    let queued: FxHashSet<(i32, i32)> = queue.iter().copied().collect();
    let mut far: Vec<_> = (own.kept.iter().copied())
        .filter(|key| !own.linger.contains_key(key) && !sent.contains(key))
        .filter(|key| !queued.contains(key))
        .collect();
    far.sort_unstable_by_key(|&(x, z)| (distance(&own.sources, x, z), x, z));
    queue.extend(far);
}

impl Hub {
    /// Guest `i`'s own level past its view, before its view moves (`update_chunks`): with its
    /// areas changed, or a lingering column's time up, the columns kept made again (a column
    /// left out at the cap gets the room a released one leaves), those released forgotten
    /// unless its view holds them, then `far` and the new columns queued. Nothing for a client
    /// that answered no revision 5, and one compare while it keeps nothing.
    pub(in crate::lan) fn update_far(&mut self, i: usize) {
        let g = &self.guests[i];
        let answer = g.sender.viewed_levels;
        if !self.viewed.active() || !answer.far_columns() {
            return;
        }
        if !g.viewed.own.dirty && g.viewed.own.linger.is_empty() {
            return;
        }
        let now = self.ticks;
        let cap = self.viewed.caps.for_client(answer).loaded_per_player as usize;
        let watches = &mut self.chunk_watches;
        let active = self.viewed.active.clone();
        self.guests.update(i, |g| {
            let released = g.viewed.own.due(now);
            if released.is_empty() && !g.viewed.own.dirty {
                return;
            }
            let areas: Vec<ChunkArea> = g.viewed.areas_in(&g.dimension).cloned().collect();
            let own = &mut g.viewed.own;
            let sources = std::mem::take(&mut own.sources);
            let kept = own.rekeep(&areas.iter().collect::<Vec<_>>(), now, cap);
            // Every pair of its entities is checked again from the new areas.
            g.moved |= own.sources != sources;
            let (center, r) = (g.center, g.view_distance);
            let in_view =
                |(x, z): (i32, i32)| center.is_some_and(|(cx, cz)| tracked(cx, cz, r, x, z));
            let gone: FxHashSet<_> = released.iter().copied().filter(|&k| !in_view(k)).collect();
            g.chunk_queue.retain(|key| !gone.contains(key));
            for &key in &released {
                if gone.contains(&key) && g.sent_chunks.remove(&key) {
                    chunk_watch::unwatch(watches, g, key);
                    let pos = [key.0, key.1];
                    g.send(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos }));
                }
            }
            if !kept.is_empty() || !released.is_empty() {
                g.sender.send(viewed_levels::far(&kept, &released));
            }
            queue_far(&mut g.chunk_queue, &g.sent_chunks, &g.viewed.own);
            g.viewed.recount(&active);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::viewed_levels::areas::ChunkAreaMode;

    fn area(center: (i32, i32), radius: u32) -> ChunkArea {
        ChunkArea {
            level: "minecraft:overworld".into(),
            center,
            radius,
            mode: ChunkAreaMode::Loaded,
        }
    }

    /// The square is kept centre first, as far as the cap; a withdrawn area's columns linger
    /// [`LINGER`] ticks, and those covered again stay. Only covered columns are queued.
    #[test]
    fn an_areas_columns_are_kept_centre_first_up_to_the_cap_then_linger() {
        let mut own = Own::default();
        let a = area((300, -40), 1);
        let kept = own.rekeep(&[&a], 0, 5);
        assert_eq!(kept.len(), 5, "cut at the cap");
        assert_eq!(kept[0], (300, -40), "the centre first");
        let mut queue = vec![(0, 0)];
        queue_far(&mut queue, &[(300, -40)].into_iter().collect(), &own);
        assert_eq!(queue.len(), 5, "after the view's, less the one sent");
        assert_eq!(queue[0], (0, 0));

        let b = area((299, -41), 0);
        own.rekeep(&[&b], 10, 9);
        assert!(own.due(10 + LINGER - 1).is_empty());
        let gone = own.due(10 + LINGER);
        assert_eq!(gone.len(), 4, "all but the column covered again");
        assert!(!gone.contains(&(299, -41)));
        assert_eq!(own.kept, [(299, -41)].into_iter().collect());
    }
}
