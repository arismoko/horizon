#[path = "shared/screen_view.rs"]
pub mod screen_view;

/// What `decide_screen` says the hooks answered to a change of screen.
pub(in crate::game) enum ScreenDecision {
    /// The change goes ahead.
    Open,
    /// `Cancel`: the current screen stays.
    Keep,
    /// `ModScreen`: this mod's own screen opens instead.
    ModScreen(&'static str),
    /// `Vanilla`: this vanilla screen opens instead.
    Vanilla(ScreenTarget),
}

/// The title screen a `ScreenOpening::Vanilla` target is (its id), or None
/// for a world's screen.
pub(in crate::game) fn title_target_id(target: ScreenTarget) -> Option<&'static str> {
    match target {
        ScreenTarget::Title => Some("title"),
        ScreenTarget::WorldSelect => Some("world_select"),
        ScreenTarget::WorldCreate => Some("world_create"),
        ScreenTarget::Multiplayer => Some("multiplayer"),
        ScreenTarget::AddServer => Some("add_server"),
        ScreenTarget::DirectConnect => Some("direct_connect"),
        _ => None,
    }
}

pub fn observed_overlay_id(overlays: &[(bool, &'static str)]) -> Option<&'static str> {
    overlays.iter().find_map(|(open, id)| open.then_some(*id))
}

/// This client's window-owned mod screen and its local slot state.
pub struct OpenNativeScreen {
    /// Fixed for the opening; slot moves have their own revision.
    pub layout_revision: u64,
    pub slot_moves: SlotMoves,
    pub screen: mod_api::__private::OwnedMenuScreen,
    pub menu: OpenNativeMenu,
    /// Installed as the window's screen, not merely staged for ScreenOpening.
    pub shown: bool,
    /// The server closed the logical menu (`LocalPlayer.clientSideCloseContainer`),
    /// while the displayed screen stays whole through `screen_closing` or a veto
    /// until its removal runs (`AbstractContainerScreen` owns its own `menu`).
    pub detached: bool,
}

/// The menu the screen presents: where its slots are stored, the opener,
/// and the window's live data.
pub struct OpenNativeMenu {
    pub key: Pos,
    pub window: i32,
    pub generation: u64,
    /// The menu's type, which its screen factory is keyed by.
    pub name: String,
    /// The server title captured for this screen, kept if the logical menu closes first.
    pub title: Option<mod_api::Text>,
    /// The mod block whose matching container opened the menu, if any.
    pub block: Option<String>,
    pub pos: Option<[i32; 3]>,
    pub mod_type: Option<&'static sunrise_level::containers::ModMenuType>,
    pub data: Vec<i32>,
}

/// Where a mod moved the open screen's slots after it opened, over the
/// positions its `MenuScreenLayout` declared. Interior state, as the screen callbacks that
/// move slots hold `&self`. A move is pending until the layout build commits
/// it, so drawing and hit-testing read one position within a frame and both
/// take a move together from the next.
#[derive(Default)]
pub struct SlotMoves {
    /// Each moved slot's position, or None for a slot a mod hid.
    pub(in crate::game) committed:
        std::cell::RefCell<std::collections::HashMap<usize, Option<[f32; 2]>>>,
    pub(in crate::game) pending:
        std::cell::RefCell<std::collections::HashMap<usize, Option<[f32; 2]>>>,
    /// How many moves changed a position, for the layout build's cache key.
    pub(in crate::game) count: std::cell::Cell<u32>,
}

impl SlotMoves {
    /// Slot `slot`'s position this frame: where the last build committed it
    /// (None once hidden), else `declared`.
    pub(in crate::game) fn position(
        &self,
        slot: usize,
        declared: Option<[f32; 2]>,
    ) -> Option<[f32; 2]> {
        self.committed
            .borrow()
            .get(&slot)
            .copied()
            .unwrap_or(declared)
    }
    /// Where menu slot `slot` will be from the next build: the last move,
    /// pending or committed, else its declaration. `declared` is the screen's
    /// declaration (`OpenNativeScreen::declared_slot`), None for no such slot.
    pub fn latest(&self, slot: usize, declared: Option<Option<[f32; 2]>>) -> Option<[f32; 2]> {
        let declared = declared?;
        let pending = self.pending.borrow().get(&slot).copied();
        match pending {
            Some(pending) => pending,
            None => self.position(slot, declared),
        }
    }
    /// Moves or activates menu slot `slot` to `pos`, or hides it with None,
    /// from the next build (`Slot.isActive()`). `declared` is as in `latest`.
    /// A refusal or a repeat changes nothing, not even the move count.
    pub fn set(
        &self,
        slot: usize,
        declared: Option<Option<[f32; 2]>>,
        pos: Option<[f32; 2]>,
    ) -> Result<(), mod_api::ScreenSlotRefusal> {
        use mod_api::ScreenSlotRefusal;
        if declared.is_none() {
            return Err(ScreenSlotRefusal::NoSuchSlot);
        }
        if pos.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
            return Err(ScreenSlotRefusal::BadPosition);
        }
        if self.latest(slot, declared) != pos {
            self.pending.borrow_mut().insert(slot, pos);
            self.count.set(self.count.get().wrapping_add(1));
        }
        Ok(())
    }
    /// Makes the pending moves the positions drawing and hit-testing read.
    pub(in crate::game) fn commit(&self) {
        let mut pending = self.pending.borrow_mut();
        if !pending.is_empty() {
            self.committed.borrow_mut().extend(pending.drain());
        }
    }
}

const MAIN_INVENTORY_SLOTS: usize = 27;

impl OpenNativeScreen {
    pub fn layout(&self) -> &mod_api::MenuScreenLayout {
        self.screen.layout()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn layout_mut(&mut self) -> &mut mod_api::MenuScreenLayout {
        self.screen.layout_mut()
    }

    pub(in crate::game) fn frame(
        &self,
        batch: &HudBatch,
        (ox, oy): (f32, f32),
        (x, y): (f32, f32),
    ) -> mod_api::MenuScreenFrame<'_> {
        mod_api::MenuScreenFrame {
            origin: [ox, oy],
            pointer: [x as i32, y as i32],
            gui_size: [batch.width as i32, batch.height as i32],
            at: [x - ox, y - oy],
            data: &self.menu.data,
        }
    }

    /// How many slot moves changed a position, so a layout built before the
    /// last one is rebuilt.
    pub fn slot_move_count(&self) -> u32 {
        self.slot_moves.count.get()
    }
    /// Makes the slot moves a mod made since the last build the screen's
    /// positions; the layout build calls it before it reads them.
    pub fn commit_slot_moves(&self) {
        self.slot_moves.commit();
    }
    /// Data slot `index` of window `window`'s menu: false for another window.
    /// The caller resolves the shown opening before passing it a packet.
    pub(in crate::game) fn set_data(&mut self, window: i32, index: usize, value: i32) -> bool {
        if window != self.menu.window {
            return false;
        }
        if self.menu.mod_type.map_or(
            index < usize::from(sunrise_mod_package::content::MAX_SIGNED_DATA_SLOTS),
            |kind| index < kind.data_slots,
        ) {
            if self.menu.data.len() <= index {
                self.menu.data.resize(index + 1, 0);
            }
            self.menu.data[index] = value;
        }
        true
    }

    pub fn container_slot_base(&self) -> usize {
        if self.menu.mod_type.is_some() {
            sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE
        } else {
            300
        }
    }

    pub(in crate::game) fn player_slots(&self) -> usize {
        self.menu
            .mod_type
            .map_or(36, sunrise_level::containers::ModMenuType::player_slots)
    }

    /// Where the screen declared menu slot `slot` (container slots, then the
    /// player's main inventory and hotbar, as `AbstractContainerMenu.slots`
    /// holds them): Some(None) for a container slot declared inactive, None
    /// for no such slot.
    pub fn declared_slot(&self, slot: usize) -> Option<Option<[f32; 2]>> {
        let layout = self.layout();
        if let Some(declared) = layout.slots.get(slot) {
            return Some(declared.as_ref().map(|slot| slot.position));
        }
        let player = slot - layout.slots.len();
        if player >= self.player_slots() {
            return None;
        }
        let (column, row_y) = if player < MAIN_INVENTORY_SLOTS {
            (player % 9, layout.inventory_y + (player / 9) as f32 * 18.0)
        } else {
            (player - MAIN_INVENTORY_SLOTS, layout.inventory_y + 58.0)
        };
        Some(Some([layout.inventory_x + column as f32 * 18.0, row_y]))
    }

    /// Menu slot of the player's inventory slot `slot` (0..9 the hotbar, 9..36
    /// the main inventory), None when the menu has no such player slot.
    pub fn player_menu_slot(&self, slot: usize) -> Option<usize> {
        if slot >= 9 + MAIN_INVENTORY_SLOTS {
            return None;
        }
        let player = if slot >= 9 {
            slot - 9
        } else {
            MAIN_INVENTORY_SLOTS + slot
        };
        (player < self.player_slots()).then(|| self.layout().slots.len() + player)
    }

    /// Where this frame puts the player's inventory slot `slot`: where the
    /// last build committed it (None once hidden), else its declared place.
    pub fn player_slot_position(&self, slot: usize) -> Option<[f32; 2]> {
        let menu_slot = self.player_menu_slot(slot)?;
        self.slot_moves
            .position(menu_slot, self.declared_slot(menu_slot)?)
    }

    pub(in crate::game) fn visible_container_slots(&self) -> impl Iterator<Item = usize> + '_ {
        let base = self.container_slot_base();
        self.layout()
            .slots
            .iter()
            .enumerate()
            .filter_map(move |(i, slot)| {
                self.slot_moves
                    .position(i, slot.as_ref().map(|slot| slot.position))?;
                Some(base + i)
            })
    }

    pub fn visible_slots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.player_slots())
            .filter(move |&slot| self.player_slot_position(slot).is_some())
            .chain(self.visible_container_slots())
    }
}

/// Whether F3's debug screen, open when `debug`, covers `corner`
/// (`DebugScreenOverlay.extractRenderState`): its left and right columns
/// the top corners, and its charts the bottom corners `charts` (bottom left,
/// bottom right) says they are drawn in.
pub(in crate::game) fn debug_takes(corner: Corner, debug: bool, charts: [bool; 2]) -> bool {
    debug
        && match corner {
            Corner::TopLeft | Corner::TopRight => true,
            Corner::BottomLeft => charts[0],
            Corner::BottomRight => charts[1],
        }
}

/// What `mod_screen_open(mod_id)` does before running a screen transition
/// (`unavailable`: no renderer, acting for a guest, neither a player nor a
/// title screen, or a world/connection is loading; `owner`: the mod whose
/// `Screen::Mod` is currently shown on top, if any): `Some(false)` refuses
/// without changing state, `Some(true)` keeps `mod_id`'s already open screen,
/// and `None` proceeds through the normal screen-transition path.
pub(in crate::game) fn mod_screen_opens(
    screen: Screen,
    unavailable: bool,
    owner: Option<&str>,
    mod_id: &str,
) -> Option<bool> {
    if unavailable {
        return Some(false);
    }
    if screen == Screen::Mod && owner == Some(mod_id) {
        return Some(true);
    }
    None
}
