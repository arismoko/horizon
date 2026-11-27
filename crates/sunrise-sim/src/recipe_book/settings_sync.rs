//! Connection-scoped authoritative custom book synchronization.
use super::{BookFlags, CustomBooks, placement_sync::ResourceId};
use std::collections::BTreeSet;

pub use sunrise_protocol::custom_book::CHANNEL;

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Hello,
    Ready,
    Offer(Vec<ResourceId>),
    Snapshot {
        epoch: u64,
        revision: u64,
        entries: Vec<(ResourceId, BookFlags)>,
    },
    Request {
        epoch: u64,
        number: u64,
        key: ResourceId,
        flags: BookFlags,
    },
    Update {
        epoch: u64,
        revision: u64,
        key: ResourceId,
        flags: BookFlags,
    },
    Placement(super::placement_sync::Frame),
}

/// A new value is required for every connection, including reconnects.
pub struct Authority {
    pub epoch: u64,
    pub revision: u64,
    keys: BTreeSet<ResourceId>,
    last_request: u64,
}

impl Authority {
    /// Book membership was negotiated once for this connection, not inferred from a request.
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
    pub fn negotiate(
        epoch: u64,
        offered: Vec<ResourceId>,
        registered: &BTreeSet<ResourceId>,
    ) -> Self {
        Self {
            epoch,
            revision: 0,
            keys: offered
                .into_iter()
                .filter(|id| registered.contains(id))
                .collect(),
            last_request: 0,
        }
    }
    pub fn server_update(&mut self, key: &str, flags: BookFlags) -> Option<Message> {
        let key = self.keys.get(key)?.clone();
        self.revision = self.revision.checked_add(1)?;
        Some(Message::Update {
            epoch: self.epoch,
            revision: self.revision,
            key,
            flags,
        })
    }
    pub fn snapshot(&self, books: &CustomBooks) -> Message {
        Message::Snapshot {
            epoch: self.epoch,
            revision: self.revision,
            entries: self
                .keys
                .iter()
                .map(|id| (id.clone(), books.get(id.as_str())))
                .collect(),
        }
    }
    /// Rejections return an authoritative snapshot, never mutate saved state.
    pub fn request(
        &mut self,
        books: &mut CustomBooks,
        epoch: u64,
        number: u64,
        key: &ResourceId,
        flags: BookFlags,
    ) -> (Message, bool) {
        if epoch != self.epoch || number <= self.last_request || !self.keys.contains(key) {
            log::warn!(
                "[recipe book] refused stale or unnegotiated settings request for {} (epoch {epoch}, number {number})",
                key.as_str()
            );
            return (self.snapshot(books), false);
        }
        self.last_request = number;
        if self.revision == u64::MAX || !books.set(key, flags) {
            return (self.snapshot(books), false);
        }
        self.revision += 1;
        (
            Message::Update {
                epoch: self.epoch,
                revision: self.revision,
                key: key.clone(),
                flags,
            },
            true,
        )
    }
}

/// Pending intent is deliberately absent: only snapshots and ordered updates
/// enter this view, and callers cannot save it as the server's player record.
#[derive(Default)]
pub struct ClientView {
    pub epoch: Option<u64>,
    pub revision: u64,
    pub ready: bool,
    pub books: CustomBooks,
    keys: BTreeSet<ResourceId>,
}
impl ClientView {
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
    pub fn receive(&mut self, message: Message, registered: &BTreeSet<ResourceId>) -> bool {
        match message {
            Message::Snapshot {
                epoch,
                revision,
                entries,
            } => {
                if self.epoch.is_some_and(|current| current != epoch)
                    || (self.epoch == Some(epoch) && revision < self.revision)
                {
                    return false;
                }
                let mut books = CustomBooks::default();
                let mut keys = BTreeSet::new();
                for (key, flags) in entries {
                    if !registered.contains(&key) {
                        return false;
                    }
                    books.set(&key, flags);
                    keys.insert(key);
                }
                self.epoch = Some(epoch);
                self.revision = revision;
                self.books = books;
                self.keys = keys;
                self.ready = true;
                true
            }
            Message::Update {
                epoch,
                revision,
                key,
                flags,
            } => {
                if !self.ready
                    || self.epoch != Some(epoch)
                    || self.revision.checked_add(1) != Some(revision)
                    || !self.keys.contains(&key)
                {
                    return false;
                }
                self.books.set(&key, flags);
                self.revision = revision;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "settings_sync_tests.rs"]
mod tests;
