//! The entities of a viewed level (#3716, 9d): a client that views the level in place from
//! another tracks its entities as one more viewer of the level's tracking pass (`Entities::tick`),
//! with its view's own tracker and chunks, and is sent their packets wrapped in runs.
//!
//! It tracks an entity as Immersive Portals' chunk tracking does (`MixinTrackedEntity`): the
//! entity's chunk was sent to the view, and an area there holds it within the entity's tracking
//! range, `distanceToSource * 16 + 8 <= range`, with the distance in chunks from the area's
//! centre. A view tracks at most `viewed_levels::MAX_ENTITIES`. Its trackers' start and stop
//! events go to no mod: a mod hears only of the player's own level.
//!
//! An entity of a `Loaded` area is sent and stands still: nothing ticks there.
use super::{
    super::{guest::Guest, hub::Hub},
    send::Runs,
};
use sunrise_protocol::{
    buf::Reader,
    packets::play::{self, Clientbound as C},
    viewed_levels,
};

/// Whether an entity in chunk `at`, tracked `range` blocks (scaled), is within reach of one of a
/// view's areas, `(centre, radius)` in chunks.
pub(in crate::lan) fn in_reach(sources: &[((i32, i32), i32)], at: (i32, i32), range: i32) -> bool {
    sources.iter().any(|&((x, z), radius)| {
        let distance = (at.0 - x).abs().max((at.1 - z).abs());
        distance <= radius && distance * 16 + 8 <= range
    })
}

/// Whether `g`'s client views level `dimension` and holds it open: it tracks that level's
/// entities through its view.
pub(in crate::lan) fn tracks_view(g: &Guest, dimension: &str) -> bool {
    g.views_level(dimension) && g.viewed.view(dimension).is_some()
}

/// `g`'s client loses entity `id`, unless it is its own player (`ServerEntity.removePairing`, a
/// player respawned as a new entity): its level's tracker and each of its views' trackers forget
/// it, and each that tracked it sends `RemoveEntities`, a view's wrapped in a run of its level.
pub(in crate::lan) fn forget_entity(g: &mut Guest, id: i32) {
    if g.entity_id == id {
        return;
    }
    let gone = C::RemoveEntities(play::RemoveEntities { ids: vec![id] });
    if g.tracker.forget(id) {
        g.send(&gone);
    }
    for view in g.viewed.views.values_mut() {
        if view.tracker.forget(id) {
            let mut runs = Runs::new(view.level);
            runs.push(&gone.encode());
            g.sender.send_many(runs.finish());
        }
    }
}

/// Whether a packet as `Clientbound::encode` writes it may be wrapped: the tracker's bundle
/// delimiters, minecart track steps and projectile power are not level-scoped and stay out.
fn scoped(body: &[u8]) -> bool {
    Reader::new(body)
        .varint()
        .is_ok_and(|id| viewed_levels::LEVEL_SCOPED.contains(&id))
}

impl Hub {
    /// An empty output for each client that views the level in place, in the guests' order, for
    /// the tracking pass; none, without looking at a player, while nobody views a level.
    pub(in crate::lan) fn view_outs(&self) -> Vec<Vec<crate::lan::Body>> {
        if !self.viewed.active() {
            return Vec::new();
        }
        let dimension = &self.config.dimension;
        let viewers = self.guests.iter().filter(|g| tracks_view(g, dimension));
        viewers.map(|_| Vec::new()).collect()
    }

    /// What the tracking pass gave each viewing client (`view_outs`), wrapped in runs of its
    /// view's level, and its tracker's events dropped.
    pub(in crate::lan) fn send_view_entities(&mut self, outs: Vec<Vec<crate::lan::Body>>) {
        if outs.is_empty() {
            return;
        }
        let dimension = self.config.dimension.clone();
        let mut outs = outs.into_iter();
        for i in 0..self.guests.len() {
            if !tracks_view(&self.guests[i], &dimension) {
                continue;
            }
            let Some(out) = outs.next() else {
                return;
            };
            self.guests.update(i, |g| {
                let Some(view) = g.viewed.views.get_mut(&dimension) else {
                    return;
                };
                view.tracker.take_events();
                let mut runs = Runs::new(view.level);
                for body in out.iter().filter(|body| scoped(body)) {
                    runs.push(body);
                }
                g.sender.send_many(runs.finish());
            });
        }
    }

    /// `ChunkMap.sendToTrackingPlayers` for the clients that view the level in place: those whose
    /// view tracks entity `id` get `packet` wrapped (its events and animations).
    pub(in crate::lan) fn send_view_trackers(&self, id: i32, packet: &C) {
        if self.viewed.active() && viewed_levels::LEVEL_SCOPED.contains(&packet.id()) {
            self.send_to_views(|view| view.tracker.is_tracking(id), packet);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::lan::entity::{Entities, EntityState, Riding, Tracker, Viewer};
    use sunrise_protocol::viewed_levels::MAX_ENTITIES;

    fn zombie(id: i32, chunk_x: i32) -> EntityState {
        EntityState {
            id,
            kind: "minecraft:zombie".into(),
            position: [f64::from(chunk_x) * 16.0 + 8.0, 64.0, 8.0],
            ..Default::default()
        }
    }

    /// One tracking pass for a client viewing the level from an area at chunk (0, 0) of radius
    /// 10, every chunk of it sent: what it tracks.
    fn track(level: &mut Entities, tracker: &mut Tracker, entities: &[EntityState]) {
        let chunks = (-10..=10)
            .flat_map(|x| (-10..=10).map(move |z| (x, z)))
            .collect();
        let mut out = Vec::new();
        let entities: Vec<&EntityState> = entities.iter().collect();
        let mut viewer = [Viewer {
            entity_id: -1,
            spectator: false,
            permission: 0,
            sunrise: true,
            general_entity: false,
            attachment_sender: None,
            moved: true,
            position: [1.0e6, 64.0, 1.0e6],
            view_distance: 2,
            chunks: &chunks,
            sources: Some(&[((0, 0), 10)]),
            far: &[],
            tracker,
            out: &mut out,
        }];
        level.tick(&entities, &Riding::default(), &mut viewer, None, |_| {
            Some(1)
        });
    }

    /// Immersive Portals' rule, from the area and not the player's place: a zombie (tracked 8
    /// chunks) is tracked 7 chunks from the area's centre (7 * 16 + 8 <= 128), not 8.
    #[test]
    fn a_view_tracks_an_entity_within_its_range_of_an_area_centre() {
        let (mut level, mut tracker) = (Entities::default(), Tracker::default());
        track(
            &mut level,
            &mut tracker,
            &[zombie(1, 7), zombie(2, 8), zombie(3, -7)],
        );
        assert!(tracker.is_tracking(1) && tracker.is_tracking(3));
        assert!(!tracker.is_tracking(2));
    }

    /// A player in the level whose client keeps an area at chunk (300, 0) past its view
    /// (`viewed_levels::own`) tracks from it too: a zombie 7 chunks from its centre, 300 from the
    /// player, is tracked, one 8 chunks from it is not, and one beside the player still is.
    #[test]
    fn a_player_tracks_from_its_own_levels_area_past_its_view() {
        let (mut level, mut tracker) = (Entities::default(), Tracker::default());
        let chunks = (0..=310).map(|x| (x, 0)).collect();
        let entities = [zombie(1, 307), zombie(2, 308), zombie(3, 0)];
        let entities: Vec<&EntityState> = entities.iter().collect();
        let mut out = Vec::new();
        let mut viewer = [Viewer {
            entity_id: -1,
            spectator: false,
            permission: 0,
            sunrise: true,
            general_entity: false,
            attachment_sender: None,
            moved: true,
            position: [0.5, 64.0, 8.0],
            view_distance: 2,
            chunks: &chunks,
            sources: None,
            far: &[((300, 0), 10)],
            tracker: &mut tracker,
            out: &mut out,
        }];
        level.tick(&entities, &Riding::default(), &mut viewer, None, |_| {
            Some(1)
        });
        assert!(tracker.is_tracking(1) && tracker.is_tracking(3));
        assert!(!tracker.is_tracking(2));
    }

    /// A view starts tracking no more than its client's world holds: past 8192 entities the rest
    /// wait until one leaves.
    #[test]
    fn a_view_tracks_at_most_8192_entities() {
        let (mut level, mut tracker) = (Entities::default(), Tracker::default());
        let mut many: Vec<EntityState> = (1..=8200).map(|id| zombie(id, 0)).collect();
        track(&mut level, &mut tracker, &many);
        assert_eq!((tracker.len(), MAX_ENTITIES), (8192, 8192));
        let first_left = (1..=8200).find(|&id| !tracker.is_tracking(id)).unwrap();
        many.retain(|e| e.id != 1);
        track(&mut level, &mut tracker, &many);
        assert_eq!(tracker.len(), 8192);
        assert!(!tracker.is_tracking(1) && tracker.is_tracking(first_left));
    }
}
