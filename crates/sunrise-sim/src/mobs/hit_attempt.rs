//! Observe base/species invulnerability once per living hit attempt, before equipment.
use super::*;

impl Mob {
    pub(super) fn remember_accepted_hit(
        &mut self,
        level: &DamageLevel<'_>,
        source: &Damage,
        accepted: bool,
    ) {
        if accepted {
            self.death_canceled = false;
            self.accepted_hit = level.accepted_hit(crate::health::Hurting::Mob(self, source));
        }
    }
    /// WitherBoss refuses its friends after asking isInvulnerableTo, not at the caller.
    pub(super) fn attacker_allows_damage(&self, source: &Damage) -> bool {
        !source
            .attacker_species
            .is_some_and(|attacker| wither::shielded_from(self.species(), attacker))
    }

    /// Observe the complete attempt once; an explicit allow overrides an initial refusal.
    pub(super) fn invulnerable_to_attempt(
        &self,
        level: &mut DamageLevel<'_>,
        source: &Damage,
    ) -> (bool, bool) {
        let vanilla = self.base_damage_immune(level, source);
        let immune =
            level.invulnerability_check(crate::health::Hurting::Mob(self, source), vanilla);
        (immune, vanilla && !immune)
    }

    /// `Entity.isInvulnerableToBase`, with the species' short-circuit rules.
    pub(super) fn base_damage_immune(&self, level: &DamageLevel<'_>, source: &Damage) -> bool {
        use sunrise_world::damage_immunity::ImmunityTag;

        let bypasses = source.kind.bypasses_invulnerability();
        let reflected = self.reflected_fireball(source, level.attack());
        let items = level.tags();
        let digging =
            self.species() == Species::Warden && warden::digging_or_emerging(self) && !bypasses;
        // Ghast.isInvulnerableTo can short-circuit the shared living predicate.
        let flagged_ghast = self.species() == Species::Ghast && self.is_invulnerable_to(source);
        digging
            || if reflected || flagged_ghast {
                self.removed || flagged_ghast
            } else {
                crate::health::BodyImmunity {
                    removed: self.removed,
                    invulnerable: self.is_invulnerable(),
                    creative_source: self.creative_attacker,
                    fire_immune: self.fire_immune(),
                    fall_immune: items.is_some_and(|items| {
                        items.entity_type_tags.has_bit(
                            self.kind.flags.entity_tag_slot,
                            crate::items::EntityTypeTags::FALL_DAMAGE_IMMUNE,
                        )
                    }),
                    source_has_tag: |tag| match tag {
                        ImmunityTag::BypassesInvulnerability => bypasses,
                        ImmunityTag::Fire => source.is_fire(),
                        ImmunityTag::Fall => match source.kind {
                            DamageKind::Named(named) => named.has(NamedDamage::IS_FALL),
                            kind => matches!(kind, DamageKind::Fall | DamageKind::Stalagmite),
                        },
                        _ => false,
                    },
                }
                .vanilla()
            }
    }

    /// `EnchantmentHelper.isImmuneToDamage`: evaluate active enchantments in worn slots.
    pub(super) fn enchantment_immune(&self, level: &mut DamageLevel<'_>, source: &Damage) -> bool {
        use sunrise_items::equipment::{HEAD, SLOT_NAMES};

        let Some(items) = level.tags() else {
            return false;
        };
        let attack = level.attack();
        let source = crate::health::mob_source(source, attack);
        let context = attack.enchantment_context(source, &self.kind.id);
        let gear = gear::gear(self);
        let equipment = SLOT_NAMES
            .into_iter()
            .enumerate()
            .filter_map(|(slot, name)| {
                let stack = self
                    .extra
                    .slot_stack(slot)
                    .filter(|stack| stack.count != 0)?;
                let levels = gear
                    .filter(|_| slot <= HEAD)
                    .map(|gear| gear.slots[finalize::gear_slot(slot)].as_slice())
                    .filter(|levels| !levels.is_empty())
                    .unwrap_or_else(|| stack.enchantments());
                Some((levels, name))
            });
        let enchantments = &items.enchantments;
        enchantments.is_damage_immune(
            enchantments.damage_immunities(equipment),
            &context,
            level.random(),
        )
    }
}
