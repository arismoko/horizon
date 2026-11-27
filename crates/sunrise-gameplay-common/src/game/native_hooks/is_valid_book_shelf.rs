use super::*;

pub trait GameHooks<'g>: crate::game::SideGame<'g> {
    /// `EnchantingTableBlock.isValidBookShelf` for the position `offset` from the table at.
    fn native_is_valid_book_shelf(
        &self,
        level: mod_api::LevelRef,
        table_pos: BlockPos,
        offset: BlockPos,
        valid: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_is_valid_book_shelf(
            self, level, table_pos, offset, valid,
        );
    }
}

impl<'g, G: crate::game::SideGame<'g>> GameHooks<'g> for G {}
