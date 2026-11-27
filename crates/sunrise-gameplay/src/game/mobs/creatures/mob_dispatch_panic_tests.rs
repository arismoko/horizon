use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_mod_runtime::attachments::AttachmentType;
use sunrise_sim::{
    join_seq::JoinSeq,
    join_seq::residency::{
        Owner,
        table::{EntityLookup, EntityTable, SourceEntity},
    },
};

pub const LEVEL_VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "mob-panic-test",
    name: "retained",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync_codec: None,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    First,
    Later,
    PendingDeath,
    UnloadedRow,
    Normal,
    NormalRemoval,
}

pub struct PanicOnDespawn {
    pub calls: Arc<AtomicUsize>,
    pub at: Option<usize>,
    pub force_allow: bool,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicOnDespawn {
    fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.force_allow {
            *event.result = mod_api::Despawn::Allow;
        }
        if self.at == Some(call) {
            // A failed answer must not affect this mob's despawn check.
            *event.result = mod_api::Despawn::Allow;
            panic!("actual mob despawn listener panicked");
        }
        if self.at.is_some_and(|at| call < at) {
            *event.result = mod_api::Despawn::Deny;
        }
    }
}

/// Resolve the real carrier through the journal consumer, not a parallel test index.
pub fn assert_resident_mobs(
    game: &crate::game::ServerGame<'_>,
    table: &mut EntityTable,
    owner: &Owner,
    expected: &[(u32, JoinSeq)],
) {
    for &identity in expected {
        let found = table.entity_lookup(&owner.dimension, identity.0, |at| {
            let mob = &game.common.mobs[game.common.mobs.locate(at.join)?];
            Some(SourceEntity {
                id: mob.id,
                value: (mob.id, mob.join),
            })
        });
        let EntityLookup::Found { location, value } = found else {
            panic!("restored mob did not resolve: {identity:?}");
        };
        assert_eq!(location.owner, *owner);
        assert_eq!(value, identity);
    }
    table
        .end_tick()
        .expect("mob pass left an abandoned operation");
}
