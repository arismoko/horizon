//! The old `ServerPlayer` retained across `restoreFrom`, independently of its replacement.
use super::Dim;
use mod_api::{
    ActiveEffect, AttachmentError, AttachmentStore, Attachments, Holder, InventoryView, LevelRef,
    OriginalPlayerAccess, PlayerExperience,
};
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::Compound;
use sunrise_gameplay_common::game::player_state::{
    CloneScope, OriginalAttachmentSlot, ScopedPlayer,
};
use sunrise_items::equipment::Equipment;
use sunrise_items::items::ItemStack;
use sunrise_mod_runtime::attachments::{
    AttachmentBinding, AttachmentLoan, AttachmentStoreKey, AttachmentStoreWrite,
};

struct RetainedOriginalAttachments {
    slot: Arc<Mutex<OriginalAttachmentSlot>>,
}

impl RetainedOriginalAttachments {
    fn new(map: Attachments) -> Self {
        Self {
            slot: Arc::new(Mutex::new(OriginalAttachmentSlot::new(map))),
        }
    }
}

impl AttachmentStore for RetainedOriginalAttachments {
    fn holds(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<bool, AttachmentError> {
        mod_api::__private::validate_attachment_key(key)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        AttachmentStore::holds(&mut slot.map, holder, key)
    }

    fn lend_decoded(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
        read: AttachmentLoan<'_>,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_key(key)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        AttachmentStore::lend_decoded(&mut slot.map, holder, key, read)
    }

    fn binding(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<Option<AttachmentBinding>, AttachmentError> {
        mod_api::__private::validate_attachment_key(key)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        AttachmentStore::binding(&mut slot.map, holder, key)
    }

    fn store(
        &mut self,
        holder: Holder,
        write: AttachmentStoreWrite<'_>,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_key(write.key())?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.journal_current();
        AttachmentStore::store(&mut slot.map, holder, write)
    }

    fn discard(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_key(key)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.journal_current();
        AttachmentStore::discard(&mut slot.map, holder, key)
    }

    fn queue_sync(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_key(key)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.journal_current();
        AttachmentStore::queue_sync(&mut slot.map, holder, key)
    }
}

/// Pre-copy state of the original `ServerPlayer`, lent directly to `player_clone`.
pub struct RetainedOriginalPlayer {
    uuid: u128,
    position: [f64; 3],
    dimension: String,
    level: Option<LevelRef>,
    health: f32,
    experience: PlayerExperience,
    effects: Vec<ActiveEffect>,
    inventory: Vec<Option<ItemStack>>,
    armor: [Option<ItemStack>; 4],
    offhand: Option<ItemStack>,
    equipment: Equipment,
    persistent_data: Option<Compound>,
    attachments: RetainedOriginalAttachments,
}

impl OriginalPlayerAccess for RetainedOriginalPlayer {
    fn uuid(&self) -> u128 {
        self.uuid
    }

    fn position(&self) -> [f64; 3] {
        self.position
    }

    fn dimension(&self) -> &str {
        &self.dimension
    }

    fn level(&self) -> Option<LevelRef> {
        self.level
    }

    fn health(&self) -> f32 {
        self.health
    }

    fn experience(&self) -> PlayerExperience {
        self.experience
    }

    fn effects(&self) -> &[ActiveEffect] {
        &self.effects
    }

    fn with_inventory(&self, read: &mut dyn for<'a> FnMut(InventoryView<'a>)) {
        sunrise_gameplay_common::game::native_server::lend_inventory_view(
            &self.inventory,
            &self.armor,
            &self.offhand,
            &self.equipment,
            read,
        );
    }

    fn persistent_data(&self) -> Option<&Compound> {
        self.persistent_data.as_ref()
    }

    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        &mut self.attachments
    }
}

impl crate::game::ServerGame<'_> {
    /// Make the callback's player addressable without publishing it in a level's player list
    /// or entity table. This also covers construction and the post-clone set-spawn event.
    pub fn with_cloning_player<R>(
        &mut self,
        conn: u32,
        scope: CloneScope,
        run: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let guest = self.command_guest(conn)?;
        let identity = ScopedPlayer {
            uuid: guest.uuid,
            conn,
            entity: matches!(scope, CloneScope::LendBody).then_some(guest.entity_id as u32),
        };
        self.common.player_state.cloning_players.push(identity);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self)));
        self.common.player_state.cloning_players.pop();
        match result {
            Ok(result) => Some(result),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// Common `ServerPlayer.restoreFrom` boundary for death respawn (`was_death = true`)
    /// and End credits return (`was_death = false`).
    ///
    /// Expects old-player removal/menu-close work to have completed against the old state
    /// and the destination level to be open (`self.enter_level`). Copies eligible
    /// attachments into a fresh replacement map unconditionally, retains the old player
    /// when `player_clone` is listened, assigns the replacement's destination dimension
    /// without placing it, reinitializes the replacement `ServerPlayer`, and dispatches
    /// `player_clone` with both holders journaled to `host.transaction`.
    pub(in crate::game::server_side) fn restore_cloned_player(
        &mut self,
        conn: u32,
        destination: Dim,
        was_death: bool,
        restore_replacement: impl FnOnce(&mut sunrise_sim::lan::guest::Guest),
        after_clone: impl FnOnce(&mut Self),
    ) -> bool {
        let Some((uuid, name)) = self.command_guest(conn).map(|g| (g.uuid, g.name.clone())) else {
            return false;
        };
        debug_assert!(
            self.transactions.is_empty(),
            "respawn must not replace an open journal's player"
        );
        let Some(map) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.attachments_mut(conn))
        else {
            return false;
        };
        let previous = std::mem::take(&mut **map);
        let _ = self.with_cloning_player(conn, CloneScope::Constructing, |game| {
            game.reinitialize_server_player(uuid, conn);
        });
        let Some(map) = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.attachments_mut(conn))
        else {
            return false;
        };
        map.copy_clone_attachments_from(
            &previous,
            was_death,
            mod_api::__private::registered_clone_attachment_policy,
        );
        let mut original = self.retain_original_player(conn, previous);
        // NeoForge's clone keeps only the `PlayerPersisted` part of a mod's saved compound.
        self.keep_only_player_persisted(conn);
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.update_guest(conn, |g| {
                g.dimension = destination.id().to_owned();
                restore_replacement(g);
            });
        }
        self.with_cloning_player(conn, CloneScope::LendBody, |game| {
            if let Some(original) = &mut original {
                game.transactions
                    .push_retained_original(Arc::clone(&original.attachments.slot));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.native_player_clone(
                        uuid,
                        &name,
                        was_death,
                        mod_api::OriginalPlayer::new(original),
                    );
                }));
                game.transactions.pop_retained_original();
                if let Err(panic) = result {
                    std::panic::resume_unwind(panic);
                }
            }
            after_clone(game);
        })
        .is_some()
    }

    /// `ServerPlayer.restoreFrom`: retain the old player's state before replacing any data.
    pub fn retain_original_player(
        &self,
        conn: u32,
        detached_attachments: Attachments,
    ) -> Option<RetainedOriginalPlayer> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerClone) {
            return None;
        }
        let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
        Some(RetainedOriginalPlayer {
            uuid: guest.uuid,
            position: guest.position,
            dimension: guest.dimension.clone(),
            level: self.level(&guest.dimension),
            health: guest.health,
            experience: PlayerExperience::from(mod_api::__private::built::PlayerExperience {
                level: guest.xp_level,
                progress: guest.xp_progress,
                total: guest.xp_total,
            }),
            effects: guest
                .effects
                .list
                .iter()
                .map(sunrise_gameplay_common::game::native_living_body::active_effect)
                .collect(),
            inventory: guest.inventory.clone(),
            armor: guest.armor.clone(),
            offhand: guest.offhand.clone(),
            equipment: guest.equipment.clone(),
            persistent_data: super::native_player_persistent_data::persistent_data_compound(
                guest.saved.as_ref(),
            )
            .cloned(),
            attachments: RetainedOriginalAttachments::new(detached_attachments),
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
