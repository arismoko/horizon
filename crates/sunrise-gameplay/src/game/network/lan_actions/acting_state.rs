use crate::game::ServerGame;
use sunrise_gameplay_common::mount_rules::has_mount_menu;
use sunrise_level::containers::BlockEntity;

/// Authoritative per-turn input flags for the active server actor (guest or host-over-pipe).
/// `attack` and `use_item` are CONTINUATION flags (`play.digging.is_some()` / `self.using.is_some()`),
/// NOT initial-action triggers; `sneak` and `sprint` come from `Guest.{sneaking,sprinting}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::game) struct ServerActingInput {
    pub sneak: bool,
    pub attack: bool,
    pub use_item: bool,
    pub sprint: bool,
}

/// Authoritative sign-editor open request recorded during a server guest turn.
/// Allowed-editor UUID is assigned in `lan_open_sign_editor` at turn drain (not here),
/// so a superseded request never leaves an un-sent sign reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) struct PendingSignEditor {
    pub pos: (i32, i32, i32),
    pub front: bool,
}

/// Authoritative UI open requests recorded during one server guest turn,
/// drained to clientbound packets on turn exit (`turns_server.rs:876-890`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::game) struct ServerPendingGuestUi {
    pub sign_editor: Option<PendingSignEditor>,
    pub lectern: Option<(i32, i32, i32)>,
    pub command_block: Option<(i32, i32, i32)>,
    pub mount: Option<u32>,
}

impl ServerGame<'_> {
    #[inline(always)]
    pub(in crate::game) fn server_acting_sneak(&self) -> bool {
        assert!(
            self.is_logical_server(),
            "server_acting_sneak called on Client role"
        );
        self.server.acting_input.sneak
    }

    #[inline(always)]
    pub(in crate::game) fn server_acting_attack(&self) -> bool {
        assert!(
            self.is_logical_server(),
            "server_acting_attack called on Client role"
        );
        self.server.acting_input.attack
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn server_acting_use(&self) -> bool {
        assert!(
            self.is_logical_server(),
            "server_acting_use called on Client role"
        );
        self.server.acting_input.use_item
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn server_acting_sprint(&self) -> bool {
        assert!(
            self.is_logical_server(),
            "server_acting_sprint called on Client role"
        );
        self.server.acting_input.sprint
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn server_set_acting_input(
        &mut self,
        input: ServerActingInput,
    ) {
        assert!(
            self.is_logical_server(),
            "server_set_acting_input called on Client role"
        );
        self.server.acting_input = input;
    }

    #[inline(always)]
    pub fn server_set_acting_sneak(&mut self, held: bool) {
        assert!(
            self.is_logical_server(),
            "server_set_acting_sneak called on Client role"
        );
        self.server.acting_input.sneak = held;
    }

    #[inline(always)]
    pub(in crate::game) fn server_set_acting_attack(&mut self, held: bool) {
        assert!(
            self.is_logical_server(),
            "server_set_acting_attack called on Client role"
        );
        self.server.acting_input.attack = held;
    }

    #[inline(always)]
    pub fn server_set_acting_use(&mut self, held: bool) {
        assert!(
            self.is_logical_server(),
            "server_set_acting_use called on Client role"
        );
        self.server.acting_input.use_item = held;
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn server_set_acting_sprint(&mut self, held: bool) {
        assert!(
            self.is_logical_server(),
            "server_set_acting_sprint called on Client role"
        );
        self.server.acting_input.sprint = held;
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn server_clear_acting_input(&mut self) {
        assert!(
            self.is_logical_server(),
            "server_clear_acting_input called on Client role"
        );
        self.server.acting_input = ServerActingInput::default();
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn server_replace_acting_input(
        &mut self,
        next: ServerActingInput,
    ) -> ServerActingInput {
        assert!(
            self.is_logical_server(),
            "server_replace_acting_input called on Client role"
        );
        std::mem::replace(&mut self.server.acting_input, next)
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn server_take_pending_guest_ui(&mut self) -> ServerPendingGuestUi {
        assert!(
            self.is_logical_server(),
            "server_take_pending_guest_ui called on Client role"
        );
        std::mem::take(&mut self.server.pending_guest_ui)
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn server_replace_pending_guest_ui(
        &mut self,
        next: ServerPendingGuestUi,
    ) -> ServerPendingGuestUi {
        assert!(
            self.is_logical_server(),
            "server_replace_pending_guest_ui called on Client role"
        );
        std::mem::replace(&mut self.server.pending_guest_ui, next)
    }

    /// `SignBlock.openTextEdit` on a server-role game: checks fake-player admission,
    /// records the pending editor open (last-write-wins), clears active turn input,
    /// and notifies `PlayerOpenSign`. Authoritative `allowed_editor` UUID is assigned
    /// in `lan_open_sign_editor` at turn drain right before sending `OpenSignEditor`.
    pub(in crate::game) fn server_request_sign_editor(
        &mut self,
        pos: (i32, i32, i32),
        front: bool,
    ) {
        assert!(
            self.is_logical_server(),
            "server_request_sign_editor called on Client role"
        );
        if self.fake_blocks(mod_api::FakeEffect::Menu) {
            return;
        }
        self.server.pending_guest_ui.sign_editor = Some(PendingSignEditor { pos, front });
        self.server.acting_input = ServerActingInput::default();
        self.open_sign_editor_server(pos, front);
    }

    /// `LecternBlock.openScreen` on a server-role game: validates lectern has a book,
    /// awards `interact_with_lectern`, records pending open, and clears turn input.
    pub(in crate::game) fn server_request_lectern(&mut self, pos: [i32; 3]) -> bool {
        assert!(
            self.is_logical_server(),
            "server_request_lectern called on Client role"
        );
        let key = (pos[0], pos[1], pos[2]);
        let has_book = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Lectern(l)) if l.book.is_some()
        );
        if !has_book {
            return false;
        }
        self.award_custom_stat("interact_with_lectern", 1);
        self.server.pending_guest_ui.lectern = Some(key);
        self.server.acting_input = ServerActingInput::default();
        true
    }

    /// `Player.openCommandBlock` on a server-role game: validates block entity and
    /// authoritative world block state directly via `pack.assets.registry` (never calls
    /// client-private `block_state_at`), records pending open, and clears turn input.
    pub(in crate::game) fn server_request_command_block(&mut self, pos: (i32, i32, i32)) {
        assert!(
            self.is_logical_server(),
            "server_request_command_block called on Client role"
        );
        if self.common.command_block(pos).is_none() {
            return;
        }
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        if self.common.pack.assets.registry.state(state).is_none() {
            return;
        }
        self.server.pending_guest_ui.command_block = Some(pos);
        self.server.acting_input = ServerActingInput::default();
    }

    /// `AbstractHorse.openCustomInventoryScreen` on a server-role game.
    pub(in crate::game::server_side) fn server_request_mount_menu(&mut self, index: usize) {
        assert!(
            self.is_logical_server(),
            "server_request_mount_menu called on Client role"
        );
        let Some(m) = self.common.mobs.get(index) else {
            return;
        };
        if !has_mount_menu(m.species()) {
            return;
        }
        let mob_id = m.id;
        self.common.station = None;
        self.common.container = None;
        self.common.craft_size = 2;
        self.server.acting_input.use_item = false;
        self.server.pending_guest_ui.mount = Some(mob_id);
    }

    /// `ServerboundPlayerCommandPacket::OPEN_INVENTORY` while riding a mount.
    /// Uses `m.extra.tame && m.alive()` so an invalid mount returns `false` and
    /// preserves the chest-boat fallback in `packets_server.rs:606-607`.
    pub(in crate::game::server_side) fn server_open_ridden_mount_menu(&mut self) -> bool {
        assert!(
            self.is_logical_server(),
            "server_open_ridden_mount_menu called on Client role"
        );
        let Some(id) = self.common.riding else {
            return false;
        };
        let Some(index) = self.common.mobs.iter().position(|m| m.id == id) else {
            return false;
        };
        let m = &self.common.mobs[index];
        if !has_mount_menu(m.species()) || !m.extra.tame || !m.alive() {
            return false;
        }
        self.server_request_mount_menu(index);
        true
    }
}
