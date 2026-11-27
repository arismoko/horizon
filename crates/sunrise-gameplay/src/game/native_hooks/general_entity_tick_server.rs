use super::*;
use mod_api::{
    EntityRef, EntityServerAccess, EntityServerScope, EntitySubjectView, ResidentGeneralEntity,
    ServerHost,
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::{
    commands_common::{Target, entity_pitch},
    general::GeneralRow,
    transactions::OpenTransactions,
};
use sunrise_sim::lan::playerdata::ints_uuid;

pub(in crate::game) struct ResidentGeneralRow<'a> {
    pub(in crate::game) row: &'a mut GeneralRow,
    pub(in crate::game) journal: &'a mut OpenTransactions,
    pub(in crate::game) kind: &'a str,
}

pub struct GeneralTickAccess<'g, 'w, 'k> {
    pub game: &'g mut crate::game::ServerGame<'w>,
    pub subject: EntityRef,
    pub kind: &'k str,
}

/// The authoritative tick of one general entity row of type `kind`
/// (`mymod:robot`) (#792 stage 4): `ServerLevel.tickNonPassenger`'s
/// `entity.tick()` for a row the type registry owns, its rider subtree
/// ticked after it. The scope lends the live row sequentially with `ServerHost`.
/// Vanilla families never reach this call. Server-side.
pub(in crate::game) fn native_general_entity_tick(
    call: mod_api::__private::GeneralEntityTickCall<'_>,
) {
    crate::game::native_hooks::fire::fire_general_entity_tick(call);
}

impl EntityServerAccess for GeneralTickAccess<'_, '_, '_> {
    fn with_view(&mut self, call: &mut dyn FnMut(EntitySubjectView<'_>)) {
        if !self.game.is_logical_server()
            || self.game.native_lifecycle.epoch() != Some(self.subject.epoch())
            || self
                .game
                .persist
                .removals
                .contains(&Target::General(self.subject.id()))
        {
            return;
        }
        let common = &mut self.game.common;
        let Some(row) = common.resident_entities.general.row_mut(self.subject.id()) else {
            return;
        };
        let mut data = ResidentGeneralRow {
            row,
            journal: &mut common.transactions,
            kind: self.kind,
        };
        call(EntitySubjectView::new(self.subject, &mut data));
    }

    fn host(&mut self) -> &mut dyn ServerHost {
        self.game
    }

    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn mod_api::EntityServerAccess,
            &'loan mod_api::TransactionContext,
        ),
    ) -> Result<(), mod_api::TransactionUnavailable> {
        let frame = self
            .game
            .current_storage_frame()
            .ok_or(mod_api::TransactionUnavailable)?;
        frame.lend(|tx| callback(self, tx));
        Ok(())
    }

    fn with_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn mod_api::EntityServerAccess,
            &'loan mod_api::TransactionContext,
        ) -> bool,
    ) -> Result<bool, mod_api::TransactionScopeError> {
        let (subject, kind) = (self.subject, self.kind);
        self.game
            .scoped_storage(|game, tx| {
                let mut child = GeneralTickAccess {
                    game,
                    subject,
                    kind,
                };
                callback(&mut child, tx).then_some(())
            })
            .map(|settled| settled.is_some())
    }
}

impl ResidentGeneralEntity for ResidentGeneralRow<'_> {
    fn kind(&self) -> &str {
        self.kind
    }
    fn position(&self) -> [f64; 3] {
        [
            self.row.position.x,
            self.row.position.y,
            self.row.position.z,
        ]
    }
    fn velocity(&self) -> [f64; 3] {
        [
            self.row.velocity.x,
            self.row.velocity.y,
            self.row.velocity.z,
        ]
    }
    fn rotation(&self) -> [f32; 2] {
        self.row.rotation
    }
    fn uuid(&self) -> u128 {
        ints_uuid(self.row.uuid)
    }
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        if let Some(kept) = self.journal.journal_general_entity(self.row.join) {
            kept.velocity.get_or_insert(self.row.velocity);
        }
        self.row.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }
    fn set_rotation(&mut self, yaw: f32, pitch: f32) -> bool {
        if !yaw.is_finite() || !pitch.is_finite() {
            return false;
        }
        if let Some(kept) = self.journal.journal_general_entity(self.row.join) {
            kept.rotation.get_or_insert(self.row.rotation);
        }
        self.row.rotation = [yaw, entity_pitch(pitch)];
        true
    }
    fn reset_fall_distance(&mut self) {
        if let Some(kept) = self.journal.journal_general_entity(self.row.join) {
            kept.fall_distance.get_or_insert(self.row.fall_distance);
        }
        self.row.fall_distance = 0.0;
    }
}

impl crate::game::ServerGame<'_> {
    /// Dispatch the authoritative tick of one resident general entity row of type `kind`,
    /// lending its live row sequentially with `ServerHost`.
    pub(crate) fn tick_general_row_scoped(&mut self, kind: &str, subject: mod_api::EntityRef) {
        let mut access = GeneralTickAccess {
            game: self,
            subject,
            kind,
        };
        native_general_entity_tick(mod_api::__private::GeneralEntityTickCall {
            kind,
            subject: EntityServerScope::new(&mut access),
        });
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
