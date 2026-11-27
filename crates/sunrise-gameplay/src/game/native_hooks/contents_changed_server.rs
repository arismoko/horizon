use super::*;
use mod_api::__private::game::Fields;

struct ContentsChangedGuard<'a, 'game> {
    game: &'a mut crate::game::ServerGame<'game>,
}

impl Drop for ContentsChangedGuard<'_, '_> {
    fn drop(&mut self) {
        self.game.server.contents_changing.pop();
    }
}

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// The block's storage calls `setChanged` (`Container.setChanged` and
    /// `BlockEntity.setChanged`), after its contents were written and before
    /// dirty/comparator updates. A native transaction tells it once per backing
    /// block at the root commit, never for an abort or a simulation. If that commit
    /// happens inside `menu_quick_move`, the notice waits for source settlement and
    /// is delivered only if the same backing container remains in place.
    /// Explicit `set_changed` still tells it immediately, even if contents compare equal. Writes
    /// inside this notice still apply, but do not re-enter it for the same block, even during
    /// a quick move; a write to another backing block has its own notice.
    ///
    /// A server-only, non-cancellable BlockPart notice, not a global bus event.
    /// `level` and `pos` name the backing block, not an automation entity or its
    /// menu's placeholder coordinates. A double container tells each component;
    /// the comparator refresh of its other half is not another notice. Replaces
    /// Create's positional `contents_changed` injection for depots, packages and
    /// schematic inventories; NeoForge has no corresponding bus event.
    pub(in crate::game) fn native_contents_changed(
        &mut self,
        level: LevelRef,
        pos: BlockPos,
        state: BlockStateId,
        observer: Option<BlockEntityRef>,
    ) {
        if self.contents_notice_active(level, pos) {
            return;
        }
        let observers = self
            .common
            .world
            .block_entities
            .contents_watches
            .observers((pos.x, pos.y, pos.z));
        self.server.contents_changing.push((level, pos));
        let notice = ContentsChangedGuard { game: self };
        notice
            .game
            .fire_contents_changed(level, pos, state, observer);
        for observer in observers.iter() {
            let Some(entity) = notice.game.block_entity_identity(level, observer.position) else {
                continue;
            };
            if entity.fields().generation == observer.generation {
                notice
                    .game
                    .fire_contents_changed(level, pos, state, Some(entity));
            }
        }
    }
}

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn contents_notice_active(&self, level: LevelRef, pos: BlockPos) -> bool {
        self.server.contents_changing.contains(&(level, pos))
    }
}

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;
