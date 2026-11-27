use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay_common::game::native_hooks::animate_tick::LentRandom;
use sunrise_gameplay_common::game::native_hooks::animate_tick::LevelRandom;

impl crate::game::ClientGame<'_> {
    /// Lend `ClientLevel.random` to a client Part, sharing the stream with any callbacks
    /// and host effects it calls before returning.
    pub(in crate::game) fn lend_client_level_random<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut dyn mod_api::RandomSource) -> R,
    ) -> R {
        if let Some(lent) = self.client.lent_level_random.0.clone() {
            return f(self, &mut LevelRandom(lent));
        }
        let lent: LentRandom = Rc::new(RefCell::new(mod_api::LegacyRandomSource::from(<(
            u64,
            Option<f64>,
        )>::from(
            &*self.client.particles.0.level_random(),
        ))));
        self.client.lent_level_random.0 = Some(lent.clone());
        let out = f(self, &mut LevelRandom(lent.clone()));
        self.client.lent_level_random.0 = None;
        *self.client.particles.0.level_random() =
            LegacyRandom::from(<(u64, Option<f64>)>::from(&*lent.borrow()));
        out
    }
    /// Runs `f`, which draws from the level random (breaking-particle position, `add_particle`'s limiter or
    /// the poofs from `create_fireworks`), on the stream a client Part
    /// holds as `event.random`: the engine's random is that stream for `f`, and
    /// `f`'s draws stay in it.
    pub(crate) fn with_lent_level_random<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(lent) = self.client.lent_level_random.0.clone() else {
            return f(self);
        };
        *self.client.particles.0.level_random() =
            LegacyRandom::from(<(u64, Option<f64>)>::from(&*lent.borrow()));
        let out = f(self);
        *lent.borrow_mut() = mod_api::LegacyRandomSource::from(<(u64, Option<f64>)>::from(
            &*self.client.particles.0.level_random(),
        ));
        out
    }
}
