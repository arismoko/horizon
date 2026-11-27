//! The plugin channels a client announces (`minecraft:register`, `minecraft:unregister`):
//! NUL-separated identifiers, as Bukkit's `StandardMessenger` and Velocity read them.
//! The server keeps them per connection, in configuration and play, for mods to read.
use std::sync::{Arc, Mutex, PoisonError};

/// The channel lists' payload channels.
pub const REGISTER: &str = "minecraft:register";
pub const UNREGISTER: &str = "minecraft:unregister";
/// Channels kept for one connection at most (Bukkit's `CraftPlayer.addChannel` refuses past 128).
pub const MAX_CHANNELS: usize = 128;
/// One channel's length at most, in bytes (Bukkit's `Messenger.MAX_CHANNEL_SIZE`).
pub const MAX_CHANNEL_LEN: usize = 64;

/// One connection's announced channels, in announcement order.
#[derive(Debug, Default)]
pub struct AnnouncedChannels(Mutex<Arc<[String]>>);

/// Whether `channel` is one of the two list channels the engine consumes.
pub fn is_list_channel(channel: &str) -> bool {
    channel == REGISTER || channel == UNREGISTER
}

impl AnnouncedChannels {
    /// Applies a `minecraft:register` / `minecraft:unregister` body; another channel
    /// is not this list's (false). Malformed entries (not UTF-8, too long, not a
    /// namespaced identifier) are skipped, and a register past [`MAX_CHANNELS`]
    /// keeps the first ones: a client's bytes never panic or grow it unbounded.
    pub fn apply(&self, channel: &str, data: &[u8]) -> bool {
        let register = match channel {
            REGISTER => true,
            UNREGISTER => false,
            _ => return false,
        };
        let mut snapshot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut updated: Option<Vec<String>> = None;
        for entry in data.split(|&b| b == 0) {
            let Some(id) = valid(entry) else {
                continue;
            };
            let list = updated.as_deref().unwrap_or(snapshot.as_ref());
            let known = list.iter().position(|c| c == id);
            match (register, known) {
                (true, None) if list.len() < MAX_CHANNELS => {
                    updated
                        .get_or_insert_with(|| snapshot.to_vec())
                        .push(id.to_owned());
                }
                (false, Some(at)) => {
                    updated.get_or_insert_with(|| snapshot.to_vec()).remove(at);
                }
                _ => {}
            }
        }
        if let Some(updated) = updated {
            *snapshot = updated.into();
        }
        true
    }
    /// Shares the current announcement order. The snapshot needs no lock after this call.
    pub fn list(&self) -> Arc<[String]> {
        Arc::clone(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
    /// Transfers the configuration snapshot into the play connection's list.
    pub fn take_from(&self, other: &AnnouncedChannels) {
        let taken = std::mem::take(&mut *other.0.lock().unwrap_or_else(PoisonError::into_inner));
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = taken;
    }
}

fn valid(entry: &[u8]) -> Option<&str> {
    if entry.is_empty() || entry.len() > MAX_CHANNEL_LEN {
        return None;
    }
    let id = std::str::from_utf8(entry).ok()?;
    sunrise_core::identifier::Identifier::parse_namespaced(id).map(|_| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_list_channels_are_consumed() {
        assert!(is_list_channel("minecraft:register"));
        assert!(is_list_channel("minecraft:unregister"));
        for other in ["minecraft:brand", "voicechat:secret", "register"] {
            assert!(!is_list_channel(other), "{other}");
        }
    }

    #[test]
    fn register_and_unregister_keep_an_ordered_set() {
        let channels = AnnouncedChannels::default();
        assert!(channels.apply(
            REGISTER,
            b"voicechat:secret\0bungeecord:main\0voicechat:secret"
        ));
        let first = channels.list();
        assert_eq!(first.as_ref(), ["voicechat:secret", "bungeecord:main"]);
        assert!(channels.apply(UNREGISTER, b"voicechat:secret\0not:there"));
        let withdrawn = channels.list();
        assert_eq!(withdrawn.as_ref(), ["bungeecord:main"]);
        assert!(!channels.apply("minecraft:brand", b"vanilla"));
        assert!(channels.apply(REGISTER, b"bungeecord:main"));
        assert!(channels.apply(UNREGISTER, b"not:there"));
        assert!(Arc::ptr_eq(&withdrawn, &channels.list()));
        assert!(channels.apply(REGISTER, b"voicechat:secret\0bungeecord:main"));
        let reregistered = channels.list();
        assert_eq!(
            reregistered.as_ref(),
            ["bungeecord:main", "voicechat:secret"]
        );

        let play = AnnouncedChannels::default();
        assert!(play.apply(REGISTER, b"old:channel"));
        let replaced = play.list();
        play.take_from(&channels);
        assert!(channels.list().is_empty());
        assert!(Arc::ptr_eq(&reregistered, &play.list()));
        assert!(play.apply(UNREGISTER, b"bungeecord:main"));
        assert_eq!(play.list().as_ref(), ["voicechat:secret"]);
        assert_eq!(replaced.as_ref(), ["old:channel"]);
        assert_eq!(
            reregistered.as_ref(),
            ["bungeecord:main", "voicechat:secret"]
        );
        assert_eq!(withdrawn.as_ref(), ["bungeecord:main"]);
        assert_eq!(first.as_ref(), ["voicechat:secret", "bungeecord:main"]);
    }

    #[test]
    fn a_hostile_list_is_bounded_and_skips_malformed_entries() {
        let channels = AnnouncedChannels::default();
        let mut body = Vec::new();
        for entry in [
            &b"\0\0"[..],
            b"\xff\xfe:bad",
            b"no_namespace",
            b"UPPER:case",
            &[b'a'; MAX_CHANNEL_LEN + 1][..],
            b"ok:first",
        ] {
            body.extend_from_slice(entry);
            body.push(0);
        }
        for n in 0..MAX_CHANNELS * 4 {
            body.extend_from_slice(format!("flood:c{n}\0").as_bytes());
        }
        assert!(channels.apply(REGISTER, &body));
        let list = channels.list();
        assert_eq!(list.len(), MAX_CHANNELS);
        assert_eq!(list[0], "ok:first");
        assert!(channels.apply(REGISTER, &vec![0xff; 1 << 16]));
        assert!(Arc::ptr_eq(&list, &channels.list()));
    }
}
