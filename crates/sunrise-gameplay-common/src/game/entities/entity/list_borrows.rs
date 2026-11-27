//! Split entity-list borrows so level inventory reads can mutate carts.
#[doc(hidden)]
#[macro_export]
macro_rules! borrow_level_list {
    (Cart, $game:expr, $($list:ident).+) => { &[] };
    ($kind:ident, $game:expr, $($list:ident).+) => { &$game$(.$list)+ };
}
pub use crate::borrow_level_list;

#[doc(hidden)]
#[macro_export]
macro_rules! borrow_lists {
    ((without_carts $game:expr) $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $crate::game::entity::EntityLists {
            $($field: $crate::borrow_level_list!($kind, $game, $($list).+),)*
        }
    };
    (($game:expr) $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $crate::game::entity::EntityLists {
            $($field: &$game$(.$list)+,)*
        }
    };
}
pub use crate::borrow_lists;

/// `$game`'s [`super::EntityLists`]; a macro so the borrow stays field by field.
#[doc(hidden)]
#[macro_export]
macro_rules! entity_lists_of {
    ($game:expr, without_carts) => {
        $crate::entity_lists!($crate::borrow_lists, without_carts $game)
    };
    ($game:expr) => {
        $crate::entity_lists!($crate::borrow_lists, $game)
    };
}
pub use crate::entity_lists_of;
