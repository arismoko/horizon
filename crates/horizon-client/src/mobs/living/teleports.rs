//! A mob's position writes inside its tick that are not its walk: `Entity.teleportTo`
//! (`snapTo`: an enderman's, a pet's to its owner, a cat's morning gift, a shulker's, a dismount)
//! and `setPos` (a villager laid in its bed or standing up from it, a size change's nudge). Each
//! is one position update of its own (`setPosRaw`, then `EntityInLevelCallback.onMove`), which
//! `entity_entering_section` tells, so a mob that teleports twice and then walks has made three.
//! Mob code moves the mob with [`snap_to`] or [`set_pos`]; while a [`Record`] is open on this
//! thread (the game opens one around its mobs' ticks while the hook is listened to), each write
//! is kept there, in order, for the game to file after that mob's tick.

use super::super::Mob;
use horizon_core::geometry::Vec3;
use std::cell::RefCell;

thread_local! {
    /// The open record's position writes: the mob and where it was put.
    static OPEN: RefCell<Option<Vec<(u32, Vec3)>>> = const { RefCell::new(None) };
}

/// `snapTo`: the position and the old position (`xo`) move together, and the motion stays. Kept
/// in the open [`Record`], if any.
pub fn snap_to(m: &mut Mob, at: Vec3) {
    m.previous = at;
    set_pos(m, at);
}

/// `setPos`: the position moves, the old position (`xo`) stays. Kept in the open [`Record`], if
/// any.
pub fn set_pos(m: &mut Mob, at: Vec3) {
    m.position = at;
    OPEN.with_borrow_mut(|open| {
        if let Some(kept) = open {
            kept.push((m.id, at));
        }
    });
}

/// The position writes made on this thread while it is open, in order. One is open at a time;
/// dropping it closes it.
#[derive(Debug)]
pub struct Record(());

impl Record {
    /// Opens a record.
    pub fn open() -> Self {
        OPEN.with_borrow_mut(|open| {
            debug_assert!(open.is_none(), "one position record at a time");
            *open = Some(Vec::new());
        });
        Self(())
    }

    /// Moves the writes kept so far onto `out`, in order.
    pub fn take_into(&self, out: &mut Vec<(u32, Vec3)>) {
        OPEN.with_borrow_mut(|open| {
            if let Some(kept) = open {
                out.append(kept);
            }
        });
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        OPEN.with_borrow_mut(|open| *open = None);
    }
}
