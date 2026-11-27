use mod_api::{__private::TransactionFrame, Attachments};

/// A player record temporarily lent during clone/respawn lifecycle hooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopedPlayer {
    pub uuid: u128,
    pub conn: u32,
    /// No whole body is lent while the replacement's attachments are being constructed.
    pub entity: Option<u32>,
}

/// Which phase of player cloning is lending the replacement record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneScope {
    Constructing,
    LendBody,
}

/// Per-player attachment state and active replacement player UUIDs during clone hooks.
#[derive(Default)]
pub struct PlayerState {
    pub attachments: sunrise_mod_runtime::attachments::ResidentAttachments,
    pub cloning_players: Vec<ScopedPlayer>,
}

/// Retained original-player attachment storage tied to `Game::transactions` so mutations,
/// default insertions, removals and decode caching roll back with an aborted transaction.
pub struct OriginalAttachmentSlot {
    pub map: Attachments,
    active_frames: Vec<TransactionFrame>,
    journal: Vec<(TransactionFrame, Attachments)>,
}

impl OriginalAttachmentSlot {
    pub fn new(map: Attachments) -> Self {
        Self {
            map,
            active_frames: Vec::new(),
            journal: Vec::new(),
        }
    }

    pub(in crate::game) fn open_frame(&mut self, token: TransactionFrame) {
        self.active_frames.push(token);
    }

    pub(in crate::game) fn close_frame(&mut self, token: TransactionFrame, commit: bool) {
        if let Some(pos) = self.active_frames.iter().rposition(|&t| t == token) {
            self.active_frames.remove(pos);
        }
        let kept = self
            .journal
            .iter()
            .rposition(|(t, _)| *t == token)
            .map(|pos| self.journal.remove(pos).1);
        let Some(kept) = kept else {
            return;
        };
        if commit {
            if let Some(&outer) = self.active_frames.last()
                && !self.journal.iter().any(|(t, _)| *t == outer)
            {
                self.journal.push((outer, kept));
            }
        } else {
            self.map = kept;
        }
    }

    pub fn journal_current(&mut self) {
        if let Some(&token) = self.active_frames.last()
            && !self.journal.iter().any(|(t, _)| *t == token)
        {
            self.journal.push((token, self.map.clone()));
        }
    }
}
