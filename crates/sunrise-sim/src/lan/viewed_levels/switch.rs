//! A travel into a viewed level (#3716, 9e): `ServerPlayer.teleport(TeleportTransition)`'s
//! Respawn (keep 3) into a level the player's client views swaps the two on both sides, as
//! Immersive Portals moves the client's world reference with no loading screen. The chunks of the
//! level entered that its client was sent stay sent, now as its own; the level left becomes a
//! viewed level that keeps every chunk sent, lingering until an area covers it again. The client
//! keeps both worlds, but their entities are re-sent and the promoted world's weather is synced
//! from what the view last received.
//!
//! It is negotiated: only a client that answered revision 2 of `sunrise:levels`
//! (`viewed_levels::REVISION_2`) swaps. Such a client swaps on that Respawn exactly when it holds
//! the level entered as a viewed world, which is when this server has its view open. A travel of
//! a revision-1 client or into a level the client does not view, and a death's or an End exit's
//! Respawn (`PlayerList.respawn`), drops every view instead, and the client every viewed world.
//! So does a travel out of a level opened after the client's Login when the client is older than
//! revision 4, which cannot name that level. From revision 4 the server sends `swap` right after
//! the Respawn: the slot of the level entered and the next epoch, which the level left is bound
//! to. A client that did not swap refuses it, so the two sides never disagree in silence. With
//! the epochs spent, the view of the level entered is closed before the travel
//! (`Hub::before_travel`), which then swaps nothing (`sunrise_protocol::viewed_levels`).
use super::super::{
    chunk_watch::ChunkWatchKind,
    hub::{AttachmentSnapshot, Hub, tracked},
    weather,
};
use super::view::View;
use sunrise_mod_runtime::attachments::Holder;
use sunrise_protocol::viewed_levels::{self, Level};

impl Hub {
    /// Name the view of the level left, if the client will swap worlds. With the epochs spent,
    /// close the destination view before Respawn so the client takes it as an ordinary travel.
    pub(in crate::lan) fn before_travel(&mut self, i: usize) -> Option<Level> {
        let entered = &self.config.dimension;
        self.guests.update(i, |g| {
            let answer = g.sender.viewed_levels;
            if answer.bindings()
                && g.viewed.epochs_spent()
                && let Some(view) = g.viewed.views.remove(entered)
            {
                g.sender.send(viewed_levels::close(view.level));
            }
            if !answer.switches() {
                return None;
            }
            let destination = g.viewed.view(entered)?.level;
            g.viewed.name_left(&g.dimension, destination, answer)
        })
    }

    /// Exchange the views and chunks after the player moves, or drop all views when the client
    /// didn't swap worlds. Either way the player leaves `left`'s queues and entities.
    /// Returns the last weather sent to the promoted view, if any.
    pub(in crate::lan) fn leave_level(
        &mut self,
        i: usize,
        left: &str,
        swap: Option<Level>,
    ) -> Option<weather::State> {
        let entered = self.config.dimension.clone();
        let watching = self.chunk_watches.recording();
        let (kept, weather) = self.guests.update(i, |g| {
            let own = std::mem::take(&mut g.sent_chunks);
            g.chunk_queue.clear();
            // Its client keeps nothing of the level entered past its view yet.
            g.viewed.own.reset();
            // The client drops both levels' entities: the server sends them again.
            g.tracker.clear();
            let Some(level) = swap else {
                g.viewed.forget_views();
                return (Vec::new(), None);
            };
            let view = g
                .viewed
                .views
                .remove(&entered)
                .expect("planned destination view");
            // The level left keeps what its client holds; the next pass starts its linger.
            let mut old = View::new(level);
            old.sent = own;
            g.viewed.views.insert(left.to_owned(), old);
            // What the view sent is the player's own now: `update_view` forgets what is past the
            // player's view and queues the rest.
            g.sent_chunks = view.sent;
            let mut kept: Vec<_> = g.sent_chunks.iter().copied().collect();
            kept.sort_unstable();
            if watching {
                let ((cx, cz), r) = (g.chunk(), g.view_distance);
                for &key in kept.iter().filter(|&&(x, z)| tracked(cx, cz, r, x, z)) {
                    if g.watched.insert(key) {
                        self.chunk_watches.push(g, ChunkWatchKind::Watch, key);
                    }
                }
            }
            (kept, Some(view.weather))
        });
        // A viewed level's chunks went without their synced attachments.
        if self.sync_attachments {
            let player = self.guests[i].uuid;
            self.attachment_snapshots
                .extend(kept.into_iter().map(|(x, z)| AttachmentSnapshot {
                    player,
                    holder: Holder::Chunk([x, z]),
                    paired_sync_from: None,
                }));
        }
        weather
    }
}
