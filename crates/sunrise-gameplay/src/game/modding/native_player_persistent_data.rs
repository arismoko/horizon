//! Scoped access to the compound mods keep on a server player (NeoForge's
//! `Entity.getPersistentData`).
use mod_api::{Compound, JavaString, PlayerPersistentDataError, PlayerRef, Tag};
use sunrise_core::nbt::{Limits, clone_tag};
use sunrise_gameplay_common::game::native_server::PlayerRead;

/// The key NeoForge writes the mods' compound under in the player file.
const PERSISTENT_DATA: &str = "NeoForgeData";
/// The entry a death respawn and the first End exit carry over.
const PLAYER_PERSISTED: &str = "PlayerPersisted";

/// Check one new subtree at its real depth in the player file, including the compound wrappers
/// that NBT emits for mixed lists and single empty-key compounds inside compound lists.
fn check_saved_value(tag: &Tag, depth: usize) -> Result<(), PlayerPersistentDataError> {
    use PlayerPersistentDataError::{InvalidNbt, TooDeep};
    let limit = Limits::saved().max_depth;
    if depth >= limit && matches!(tag, Tag::Compound(_) | Tag::List(_)) {
        return Err(TooDeep);
    }
    match tag {
        Tag::End => Err(InvalidNbt),
        Tag::String(s) if s.modified_utf8_len() > usize::from(u16::MAX) => Err(InvalidNbt),
        Tag::ByteArray(values) if i32::try_from(values.len()).is_err() => Err(InvalidNbt),
        Tag::IntArray(values) if i32::try_from(values.len()).is_err() => Err(InvalidNbt),
        Tag::LongArray(values) if i32::try_from(values.len()).is_err() => Err(InvalidNbt),
        Tag::Compound(data) => {
            for (key, value) in data {
                if key.modified_utf8_len() > usize::from(u16::MAX) {
                    return Err(InvalidNbt);
                }
                check_saved_value(value, depth + 1)?;
            }
            Ok(())
        }
        Tag::List(values) => {
            if i32::try_from(values.len()).is_err() {
                return Err(InvalidNbt);
            }
            let compound_list = values.first().is_some_and(|first| {
                first.id() == 10 || values.iter().skip(1).any(|value| value.id() != first.id())
            });
            for value in values {
                let wrapped = compound_list
                    && match value {
                        Tag::Compound(data) => {
                            data.len() == 1
                                && data.keys().next().is_some_and(|key| key.0.is_empty())
                        }
                        _ => true,
                    };
                if wrapped && depth + 1 >= limit {
                    return Err(TooDeep);
                }
                check_saved_value(value, depth + 1 + usize::from(wrapped))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Borrow the `NeoForgeData` persistent-data compound from a saved player root tag, if present.
pub(in crate::game) fn persistent_data_compound(saved: Option<&Tag>) -> Option<&Compound> {
    match saved?.get(PERSISTENT_DATA)? {
        Tag::Compound(data) => Some(data),
        _ => None,
    }
}

impl crate::game::ServerGame<'_> {
    /// The mods' compound on a player: the acting player's saved data is the level's for their
    /// turn, another's is their record's.
    pub(in crate::game) fn player_persistent_data(&self, player: PlayerRef) -> Option<&Compound> {
        if !self.common.is_logical_server() {
            return None;
        }
        let saved = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.common.level.player_extra.as_ref(),
            PlayerRead::Guest(g) => g.saved.as_ref(),
        };
        persistent_data_compound(saved)
    }

    /// Edits one checked key directly on the acting player's or guest's saved compound, so
    /// living and dead online players both accept edits (NeoForge's `Entity.getPersistentData`).
    /// Only the submitted value needs to be copied.
    pub(in crate::game) fn player_set_persistent_data_key(
        &mut self,
        player: PlayerRef,
        key: &JavaString,
        value: Option<&Tag>,
    ) -> Result<(), PlayerPersistentDataError> {
        if !self.common.is_logical_server() {
            return Err(PlayerPersistentDataError::Client);
        }
        let target = match self.player_handle_read(player) {
            Some(PlayerRead::Acting { .. }) => None,
            Some(PlayerRead::Guest(g)) => Some(g.conn),
            None => return Err(PlayerPersistentDataError::Stale),
        };
        if let Some(value) = value {
            if key.modified_utf8_len() > usize::from(u16::MAX) {
                return Err(PlayerPersistentDataError::InvalidNbt);
            }
            // The root is at depth 0, NeoForgeData at depth 1, and this entry at depth 2.
            check_saved_value(value, 2)?;
        }
        let has_key = |saved: Option<&Tag>| {
            matches!(
                saved.and_then(|tag| tag.get(PERSISTENT_DATA)),
                Some(Tag::Compound(data)) if data.contains_key(key)
            )
        };
        let edit = |saved: &mut Option<Tag>| {
            if let Some(Tag::Compound(data)) =
                saved.as_mut().and_then(|tag| tag.get_mut(PERSISTENT_DATA))
            {
                match value {
                    Some(value) => {
                        let value = clone_tag(value);
                        if let Some(entry) = data.get_mut(key) {
                            *entry = value;
                        } else {
                            data.insert(key.clone(), value);
                        }
                    }
                    None => {
                        data.remove(key);
                    }
                }
            } else if let Some(value) = value {
                let data = Tag::Compound(Compound::from([(key.clone(), clone_tag(value))]));
                if let Some(Tag::Compound(root)) = saved {
                    root.insert(PERSISTENT_DATA.into(), data);
                } else {
                    *saved = Some(Tag::compound([(PERSISTENT_DATA, data)]));
                }
            }
        };
        match target {
            None => {
                if value.is_none() && !has_key(self.common.level.player_extra.as_ref()) {
                    return Ok(());
                }
                self.journal_player_extra(player.uuid());
                edit(&mut self.common.level.player_extra);
                Ok(())
            }
            Some(conn) => {
                let Some(index) = self
                    .server
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guests.connection_index(conn))
                else {
                    return Err(PlayerPersistentDataError::Stale);
                };
                if value.is_none()
                    && !has_key(
                        self.server.lan.as_ref().expect("resolved guest").hub.guests[index]
                            .saved
                            .as_ref(),
                    )
                {
                    return Ok(());
                }
                self.journal_parked_player_extra(index);
                self.server
                    .lan
                    .as_mut()
                    .expect("resolved guest")
                    .hub
                    .guests
                    .update_connection(conn, |g| edit(&mut g.saved))
                    .ok_or(PlayerPersistentDataError::Stale)
            }
        }
    }

    /// `PlayerList.respawn` after a death and after a first exit from the End: of the saved
    /// compound only `PlayerPersisted` goes to the new player (NeoForge's
    /// `PlayerEvent.Clone` keeps that sub-compound). Called before the respawn's clone event.
    pub(in crate::game::server_side) fn keep_only_player_persisted(&mut self, conn: u32) {
        if let Some(host) = self.server.lan.as_mut() {
            host.hub
                .guests
                .update_connection(conn, |g| keep_player_persisted(&mut g.saved));
        }
    }
}

/// Carries only `PlayerPersisted`, whatever its type. With no data to carry, the new
/// player has no stored compound.
fn keep_player_persisted(saved: &mut Option<Tag>) {
    let Some(Tag::Compound(root)) = saved else {
        return;
    };
    let data_key = PERSISTENT_DATA.into();
    let persisted_key = PLAYER_PERSISTED.into();
    if let Some(Tag::Compound(data)) = root.get_mut(&data_key)
        && let Some(value) = data.remove(&persisted_key)
    {
        *data = Compound::from([(persisted_key, value)]);
    } else {
        root.remove(&data_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(entries: Vec<(&str, Tag)>) -> Option<Tag> {
        let mut data = Compound::new();
        for (key, tag) in entries {
            data.insert(key.into(), tag);
        }
        let mut saved = Compound::new();
        saved.insert("Other".into(), Tag::Int(1));
        saved.insert(PERSISTENT_DATA.into(), Tag::Compound(data));
        Some(Tag::Compound(saved))
    }

    #[test]
    fn saved_depth_counts_mixed_list_wrappers() {
        fn mixed_at_depth(depth: usize) -> Tag {
            // The sole empty-key compound has to be wrapped, as does the scalar.
            let mut tag = Tag::List(vec![Tag::compound([("", Tag::Int(1))]), Tag::Int(2)]);
            for _ in 2..depth {
                tag = Tag::compound([("next", tag)]);
            }
            tag
        }
        // An entry starts at 2. A list at 509 can hold the wrapper at 510 and
        // its compound at 511. Pushing the list one deeper puts that compound at 512.
        assert_eq!(check_saved_value(&mixed_at_depth(509), 2), Ok(()));
        assert_eq!(
            check_saved_value(&mixed_at_depth(510), 2),
            Err(PlayerPersistentDataError::TooDeep)
        );
    }

    /// Only `PlayerPersisted` is kept, of any type; the player file's other keys stay.
    #[test]
    fn only_player_persisted_is_kept_whatever_its_type() {
        let mut compound = saved(vec![
            ("lost", Tag::Int(1)),
            (PLAYER_PERSISTED, Tag::String("kept".into())),
        ]);
        keep_player_persisted(&mut compound);
        assert_eq!(
            compound,
            saved(vec![(PLAYER_PERSISTED, Tag::String("kept".into()))])
        );
        let mut none = saved(vec![("lost", Tag::Int(1))]);
        keep_player_persisted(&mut none);
        let Some(Tag::Compound(root)) = &none else {
            panic!("the player file stays")
        };
        assert!(
            root.get(&PERSISTENT_DATA.into()).is_none(),
            "nothing kept, no key"
        );
        assert_eq!(root.get(&"Other".into()), Some(&Tag::Int(1)));
    }
}
