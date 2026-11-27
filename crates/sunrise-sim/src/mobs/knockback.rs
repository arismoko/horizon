use super::*;

impl Mob {
    /// Short `LivingEntity.knockback` entry, delegating to the full entry with
    /// `comes_from_effect = false`. Both entries are independent native internals points.
    /// API access refuses the `Mob` receiver and the unmirrored `DamageLevel`/`Damage`
    /// context.
    /// When the push reaches the living implementation, the hook runs once before resistance,
    /// motion changes and random draws.
    #[sunrise_mod_api_macros::replaceable]
    pub fn knockback(
        &mut self,
        level: &mut DamageLevel<'_>,
        power: f64,
        xd: f64,
        zd: f64,
        source: &Damage,
        damage: f32,
    ) -> bool {
        self.knockback_full(level, power, xd, zd, source, damage, false)
    }
    /// `getAttributeValue(KNOCKBACK_RESISTANCE)` at the time of the push: the mob's attribute as
    /// it stands now, with the modifiers its gear granted at the last equipment check
    /// (`LivingEntity.collectEquipmentChanges`, once a tick).
    pub fn knockback_resistance(&self) -> f64 {
        match self.extra.gear_knockback.as_deref() {
            Some(gear) => finalize::value_with(
                self,
                finalize::Attr::KnockbackResistance,
                gear.modifiers.iter().copied(),
            ),
            None => self.attrs.knockback_resistance,
        }
    }
    /// Full knockback entry, including whether this is an extra effect push.
    /// The short entry delegates here once; direct full calls bypass the short entry.
    /// `SulfurCube.knockback` handles a block-carrying cube pushed by a causing entity without
    /// calling the living implementation. An effect push changes only the launch multiplier.
    /// API access has the same receiver/context refusal as the short entry.
    #[sunrise_mod_api_macros::replaceable]
    pub fn knockback_full(
        &mut self,
        level: &mut DamageLevel<'_>,
        power: f64,
        xd: f64,
        zd: f64,
        source: &Damage,
        damage: f32,
        comes_from_effect: bool,
    ) -> bool {
        if self.species() == Species::SulfurCube
            && sulfur_cube::has_body(self)
            && source.attacker.is_some()
        {
            sulfur_cube::knockback(
                self,
                (xd, zd),
                source.from,
                damage,
                comes_from_effect.then_some(power),
            );
            return true;
        }
        if creaking::is_creaking(self) && !creaking::can_move(self) {
            return false;
        }
        // The native hook runs first, on the raw strength, then the resistance scales what it left.
        let mut knock = crate::health::Knock {
            strength: power,
            x: xd,
            z: zd,
        };
        let pushed = crate::health::Hurting::Mob(self, source);
        if level.knockback_canceled(pushed, &mut knock) {
            return false;
        }
        let resistance = self.knockback_resistance();
        if !knock.apply(
            &mut self.velocity,
            resistance,
            self.ground.on_ground,
            &mut self.random,
        ) {
            return false;
        }
        self.needs_sync = true;
        true
    }
}
