//! `ServerGamePacketListenerImpl.handleAttack`'s checks between the target's lookup and
//! `Player.attack`: the world border, the attack range, and the targets no player may attack.
use sunrise_protocol::packets::configuration::UpdateTags;

/// `AbstractArrow.isAttackable`: the arrow's type (`type_id`) is in
/// `#minecraft:redirectable_projectile`, as the server sent its tags to the players, a world's
/// data packs included. A type or a tag missing from the data counts as attackable, so no
/// player is kicked over data the server does not have.
pub fn arrow_attackable(tags: &UpdateTags, type_id: Option<i32>) -> bool {
    let Some(type_id) = type_id else {
        return true;
    };
    let tag = tags
        .registries
        .iter()
        .find(|r| r.registry == "minecraft:entity_type")
        .and_then(|r| {
            r.tags
                .iter()
                .find(|t| t.name == "minecraft:redirectable_projectile")
        });
    tag.is_none_or(|t| t.ids.contains(&type_id))
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    use std::sync::mpsc;
    #[cfg(test)]
    use sunrise_protocol::packets::configuration::{RegistryTags, TagEntry, UpdateTags};
    use sunrise_sim::lan::Out;

    #[cfg(test)]
    pub(in crate::game) fn tags(redirectable: &[i32]) -> UpdateTags {
        UpdateTags {
            registries: vec![RegistryTags {
                registry: "minecraft:entity_type".into(),
                tags: vec![TagEntry {
                    name: "minecraft:redirectable_projectile".into(),
                    ids: redirectable.to_vec(),
                }],
            }],
        }
    }

    #[cfg(test)]
    /// `AbstractArrow.isAttackable` reads the tag the players were sent: a data pack that
    /// puts arrows in `#minecraft:redirectable_projectile` makes them attackable, and a
    /// vanilla client then picks and attacks them, which must not kick.
    #[test]
    fn an_arrow_is_attackable_only_in_the_sent_redirectable_tag() {
        assert!(!arrow_attackable(&tags(&[30, 40]), Some(5)));
        assert!(arrow_attackable(&tags(&[5, 30, 40]), Some(5)));
        assert!(arrow_attackable(&tags(&[30]), None), "an unknown type");
        assert!(
            arrow_attackable(&UpdateTags { registries: vec![] }, Some(5)),
            "no entity type tags"
        );
    }

    pub fn disconnects(rx: &mpsc::Receiver<Out>) -> Vec<sunrise_core::nbt::Tag> {
        rx.try_iter()
            .filter_map(|out| match out {
                Out::Disconnect(reason) => Some(reason),
                _ => None,
            })
            .collect()
    }
}
