//! Observe an ordinary player move at its position write while the world is borrowed.

use super::PlayerMotion;
use std::{cell::RefCell, rc::Rc};
use sunrise_core::geometry::Vec3;

pub trait PositionObserver {
    fn position_written(&self, player: &PlayerMotion, from: Vec3);
}

thread_local! {
    static OBSERVER: RefCell<Option<Rc<dyn PositionObserver>>> = const { RefCell::new(None) };
}

/// Installed only around a live player move, not speculative or replay motion.
#[must_use]
pub struct PositionObserverScope(Option<Rc<dyn PositionObserver>>);

impl PositionObserverScope {
    pub fn install(observer: Rc<dyn PositionObserver>) -> Self {
        Self(OBSERVER.replace(Some(observer)))
    }
}

impl Drop for PositionObserverScope {
    fn drop(&mut self) {
        OBSERVER.replace(self.0.take());
    }
}

pub(super) fn position_written(player: &PlayerMotion, from: Vec3) {
    let observer = OBSERVER.with_borrow(Clone::clone);
    if let Some(observer) = observer {
        observer.position_written(player, from);
    }
}
