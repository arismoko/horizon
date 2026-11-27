use super::*;

/// The hook points. Each does nothing in Sunrise.
impl HitLevel<'_> {
    /// `Block.entityInside`: an entity's sweep crossed a block with a part for it
    /// (`precise` is `Entity.checkInsideBlocks`' flag), told mid-sweep, on the level
    /// host a hit or a landing uses. The part draws the level's random in vanilla's
    /// order and queues effects and owned callbacks on `event.apply`, `event.run_before`,
    /// and `event.run_after`, which flush at the end of the sweep via
    /// `InsideEffects::apply_and_clear` with live entity and level access.
    pub(in crate::game) fn native_entity_inside(
        &mut self,
        call: super::fire::EntityInsideCall<'_>,
    ) {
        self.fire_entity_inside(call);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod sweep_tests {
    use super::*;

    /// The `entity_inside` part the player's sweep tests install: it draws the
    /// level's random once a contact (vanilla's order), queues a push and a wither
    /// effect around the freeze, and can kill or panic on later contacts.
    #[derive(Clone, Default)]
    pub struct InsidePart {
        pub contacts: std::sync::Arc<std::sync::Mutex<Vec<[i32; 3]>>>,
        pub applied_order: std::sync::Arc<std::sync::Mutex<Vec<(&'static str, i32)>>>,
        pub wither: std::sync::Arc<std::sync::Mutex<Option<mod_api::EffectKey>>>,
        pub speed: std::sync::Arc<std::sync::Mutex<Option<mod_api::EffectKey>>>,
        pub hurt_after: Option<(usize, f32)>,
        pub generic: std::sync::Arc<std::sync::Mutex<Option<mod_api::DamageTypeKey>>>,
        pub panic_after: Option<usize>,
    }
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for InsidePart {
        fn entity_inside(
            &self,
            host: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::EntityInsideEvent<'_>,
        ) {
            let mut contacts = self.contacts.lock().unwrap();
            contacts.push([event.pos.x, event.pos.y, event.pos.z]);
            let seen = contacts.len();
            drop(contacts);
            // A level draw, in vanilla's order: between the vanilla contacts.
            let _ = host.with_random(|random| random.next_long());
            if self.panic_after.is_some_and(|after| seen > after) {
                // Unlike an owned closure, this effect would not be skipped by owner poisoning
                // if a failed listener accidentally published its staged collector.
                event.apply(mod_api::InsideEffect::LavaIgnite);
                panic!("an entity_inside part panics");
            }
            if let Some((after, amount)) = self.hurt_after
                && seen > after
            {
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    view.hurt("minecraft:generic", amount);
                });
            }
            event.run_after(mod_api::InsideEffect::Freeze, |view| {
                view.push([0.5, 0.0, 0.0]);
            });
            if let Some(wither) = *self.wither.lock().unwrap() {
                let order = self.applied_order.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    if view.add_effect(wither, 40, 0) {
                        order.lock().unwrap().push(("minecraft:wither", 40));
                    }
                });
            }
            if let Some(speed) = *self.speed.lock().unwrap() {
                let order = self.applied_order.clone();
                event.run_after(mod_api::InsideEffect::Extinguish, move |view| {
                    if view.add_effect(speed, 20, 0) {
                        order.lock().unwrap().push(("minecraft:speed", 20));
                    }
                });
            }
        }
    }
    pub(in crate::game) struct InsideMod(pub(in crate::game) InsidePart);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for InsideMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            *self.0.generic.lock().unwrap() = Some(r.damage_type_key("minecraft:generic"));
            *self.0.wither.lock().unwrap() = Some(r.effect_key("minecraft:wither"));
            *self.0.speed.lock().unwrap() = Some(r.effect_key("minecraft:speed"));
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, self.0.clone());
            if self.0.hurt_after.is_some() {
                let grass = r.block_key("minecraft:short_grass");
                r.block_part(grass, self.0.clone());
            }
        }
    }

    /// The part's mods, installed with their parts as a game's open does, before
    /// `with_game`: the game that opens then publishes the block index itself (the
    /// #824 parts-resolution order).
    pub fn install_inside_mod(name: &'static str, part: &InsidePart) -> mod_api::Mods {
        let entry =
            |part: &InsidePart| mod_api::ModEntry::new(name, Box::new(InsideMod(part.clone())));
        crate::game::api_values::install_parts(&[entry(part)]).unwrap();
        vec![entry(part)].into()
    }

    /// An arrow for the sweep tests: an arrow state, its move recorded.
    pub fn arrow_shot(
        id: u32,
        from: (f64, f64, f64),
        to: (f64, f64, f64),
    ) -> sunrise_sim::projectiles::Projectile {
        let mut shot = sunrise_sim::projectiles::Projectile::new(
            sunrise_sim::projectiles::ProjectileKind::Arrow {
                base_damage: 1.0,
                critical: false,
                effect: None,
            },
            sunrise_core::geometry::Vec3::new(to.0, to.1, to.2),
            None,
        );
        shot.id = id;
        shot.previous = sunrise_core::geometry::Vec3::new(from.0, from.1, from.2);
        shot
    }
}
