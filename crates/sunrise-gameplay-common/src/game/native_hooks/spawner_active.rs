use super::*;

pub trait GameHooks<'g>: crate::game::SideGame<'g> {
    /// `BaseSpawner.isNearPlayer`: whether `spawner` in `dimension` is active this tick.
    fn native_spawner_active(&self, dimension: &str, spawner: mod_api::Spawner, active: &mut bool) {
        crate::game::native_hooks::fire::FireHooks::fire_spawner_active(
            self, dimension, spawner, active,
        );
    }

    /// Vanilla's `isNearPlayer` answer `near` for `spawner`, as a mod's `spawner_active` leaves.
    fn spawner_active(&self, spawner: impl FnOnce() -> mod_api::Spawner, near: bool) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::SpawnerActive) {
            return near;
        }
        let mut active = near;
        self.native_spawner_active(self.dim.id(), spawner(), &mut active);
        active
    }
}

impl<'g, G: crate::game::SideGame<'g>> GameHooks<'g> for G {}
