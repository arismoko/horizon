//! The cartography table's role-neutral map session.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};
use sunrise_sim::menu::Maps;

/// The level's maps as a menu sees them (`ContainerLevelAccess`): the
/// states of the maps the click can reach, and the copies it makes.
#[derive(Clone)]
pub struct LevelMaps {
    states: RefCell<HashMap<i32, (bool, u8)>>,
    next_id: Cell<i32>,
    /// (new id, source id, scaled) for each copy made.
    made: RefCell<Vec<(i32, i32, bool)>>,
}

impl LevelMaps {
    pub fn new(states: HashMap<i32, (bool, u8)>, next_id: i32) -> Self {
        Self {
            states: RefCell::new(states),
            next_id: Cell::new(next_id),
            made: RefCell::new(Vec::new()),
        }
    }

    pub fn into_made(self) -> Vec<(i32, i32, bool)> {
        self.made.into_inner()
    }
}

impl Maps for LevelMaps {
    fn with_staged(&self, edit: &mut dyn FnMut(&dyn Maps) -> bool) -> bool {
        let staged = self.clone();
        if !edit(&staged) {
            return false;
        }
        self.states.replace(staged.states.into_inner());
        self.next_id.set(staged.next_id.get());
        self.made.replace(staged.made.into_inner());
        true
    }
    fn state(&self, id: i32) -> Option<(bool, u8)> {
        self.states.borrow().get(&id).copied()
    }

    fn derive(&self, id: i32, scale: bool) -> Option<i32> {
        let (_, s) = self.state(id)?;
        let new = self.next_id.get();
        self.next_id.set(new + 1);
        let made = if scale {
            (false, (s + 1).min(4))
        } else {
            (true, s)
        };
        self.states.borrow_mut().insert(new, made);
        self.made.borrow_mut().push((new, id, scale));
        Some(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_maps_restore_id_reservations_and_pending_copies_on_refusal() {
        for accepted in [false, true] {
            let maps = LevelMaps {
                states: RefCell::new(HashMap::from([(7, (false, 0))])),
                next_id: Cell::new(42),
                made: RefCell::new(Vec::new()),
            };
            assert_eq!(
                maps.with_staged(&mut |staged| {
                    assert_eq!(staged.derive(7, true), Some(42));
                    assert_eq!(staged.state(42), Some((false, 1)));
                    accepted
                }),
                accepted
            );
            assert_eq!(maps.next_id.get(), if accepted { 43 } else { 42 });
            assert_eq!(maps.state(42), accepted.then_some((false, 1)));
            assert_eq!(
                maps.made.borrow().as_slice(),
                if accepted { &[(42, 7, true)][..] } else { &[] }
            );
        }
    }
}
