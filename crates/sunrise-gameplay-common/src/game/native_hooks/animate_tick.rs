//! `Block.animateTick` for mod blocks: the parts of the blocks `ClientLevel.animateTick` picks,
//! called inside the pick.
use super::*;
use std::cell::RefCell;
use std::rc::Rc;

/// The level's random (`ClientLevel.random`) while a client Part holds it as
/// `event.random`: one stream for the part's draws and the game's own draws during the call
/// (`add_particle`'s limiter roll, `Game::with_lent_level_random`), in call order.
pub type LentRandom = Rc<RefCell<mod_api::LegacyRandomSource>>;

/// `event.random`: draws from the lent level random.
pub struct LevelRandom(pub LentRandom);

impl mod_api::RandomSource for LevelRandom {
    fn next_int(&mut self) -> i32 {
        self.0.borrow_mut().next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.borrow_mut().next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.borrow_mut().next_long()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.borrow_mut().next_boolean()
    }
    fn next_float(&mut self) -> f32 {
        self.0.borrow_mut().next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.borrow_mut().next_double()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.borrow_mut().next_gaussian()
    }
}
