//! What a mod's blast takes beyond vanilla's short `Level.explode` (#4009): its
//! damage type, its damage calculator as data (`SimpleExplosionDamageCalculator`)
//! and what players see and hear of it. `level_explode` resolves them once; the
//! combat state holds them while the blast goes off, and the rays, the entity
//! walk and the effects read them here. Any other blast has none of them and
//! goes off as before.
use crate::game::ServerGame;
use std::collections::HashSet;
use sunrise_sim::mobs::{DamageKind, NamedDamage};

/// What `SimpleExplosionDamageCalculator` gives a block of its immune set.
const IMMUNE_RESISTANCE: f32 = 3_600_000.0;

/// A mod blast's own choices; each `None` is vanilla's.
#[derive(Default)]
pub(in crate::game) struct BlastSettings {
    /// The damage type, in place of the one the blast derives.
    pub(in crate::game) damage: Option<NamedDamage>,
    pub(in crate::game) calculator: Option<BlastCalculator>,
    /// The calculator the blast's source brings, used only when the mod
    /// passes none (`ServerExplosion.makeDamageCalculator`).
    pub(in crate::game) source_calculator: SourceCalculator,
    pub(in crate::game) effects: Option<Box<BlastEffects>>,
}

/// The resistance rule a mod blast's source brings with it.
#[derive(Default)]
pub(in crate::game) enum SourceCalculator {
    /// The default calculator.
    #[default]
    Default,
    /// `PrimedTnt.USED_PORTAL_DAMAGE_CALCULATOR`, a primed TNT's that went
    /// through a portal: a nether portal neither resists nor breaks.
    PortalProof,
    /// A wind charge's `SimpleExplosionDamageCalculator`: the blocks of
    /// `#blocks_wind_charge_explosions` resist 3,600,000 and every other
    /// cell, fluids included, nothing; entities are pushed but never
    /// damaged, by 1.22 for a player's wind charge (`WindCharge`) and 1 for
    /// a breeze's (`BreezeWindCharge`).
    WindCharge(BlastCalculator),
}

/// `SimpleExplosionDamageCalculator(explodesBlocks, damagesEntities,
/// knockbackMultiplier, immuneBlocks)`.
pub(in crate::game) struct BlastCalculator {
    pub(in crate::game) explodes_blocks: bool,
    pub(in crate::game) damages_entities: bool,
    pub(in crate::game) knockback: f32,
    /// The immune tag's blocks: when given, the only resistance there is.
    pub(in crate::game) immune: Option<HashSet<String>>,
}

pub(in crate::game) use sunrise_sim::lan::server_effects::BlastEffects;

impl ServerGame<'_> {
    /// The damage type the blast deals: the mod's, else `DamageSources.explosion`'s
    /// (`minecraft:player_explosion` for a caused blast).
    pub(in crate::game::server_side) fn blast_damage_type(&self) -> &'static str {
        match self.server.combat.0.blast_settings.damage {
            Some(damage) => damage.name,
            None => sunrise_gameplay_common::game::creatures::damage_type(
                DamageKind::Explosion,
                self.blast_attacker().is_some(),
            ),
        }
    }
    /// The mobs' side of `blast_damage_type`: the mod's type, else the
    /// explosion kind, which tells a caused blast by its attacker.
    pub(in crate::game::server_side) fn blast_damage_kind(&self) -> DamageKind {
        let damage = self.server.combat.0.blast_settings.damage;
        damage.map_or(DamageKind::Explosion, DamageKind::Named)
    }
    /// What a ray's step through block `name` takes and whether the block is
    /// spared, when the blast's calculator overrides the resistance rule: the
    /// mod's calculator when it passes one, else its source's
    /// (`SourceCalculator`). An immune set (the mod's `immune_blocks`, a wind
    /// charge's tag) resists 3,600,000 in its blocks and nothing in every
    /// other cell, fluids included, and no part is asked; a portal-proof
    /// blast passes through a nether portal and spares it. None for the
    /// usual rule. The rays and every resistance read take it from here.
    pub(in crate::game::server_side) fn blast_calculator_step(
        &self,
        name: &str,
    ) -> Option<(Option<f32>, bool)> {
        let settings = &self.server.combat.0.blast_settings;
        if let (None, SourceCalculator::PortalProof) =
            (&settings.calculator, &settings.source_calculator)
        {
            return (name == "minecraft:nether_portal").then_some((None, true));
        }
        let immune = self.blast_simple_calculator()?.immune.as_ref()?;
        let taken = if immune.contains(name) {
            Some(IMMUNE_RESISTANCE)
        } else {
            None
        };
        Some((taken, false))
    }
    /// The blast's `SimpleExplosionDamageCalculator`: the mod's when it
    /// passes one, which replaces every rule of the source's, else a wind
    /// charge's own. None for the default and portal-proof calculators.
    fn blast_simple_calculator(&self) -> Option<&BlastCalculator> {
        let settings = &self.server.combat.0.blast_settings;
        match (&settings.calculator, &settings.source_calculator) {
            (Some(c), _) | (None, SourceCalculator::WindCharge(c)) => Some(c),
            _ => None,
        }
    }
    /// `shouldBlockExplode`: false when the calculator explodes no blocks.
    pub(in crate::game::server_side) fn blast_explodes_blocks(&self) -> bool {
        let calculator = self.blast_simple_calculator();
        calculator.is_none_or(|c| c.explodes_blocks)
    }
    /// `shouldDamageEntity`: false for a wind charge's calculator.
    pub(in crate::game::server_side) fn blast_damages_entities(&self) -> bool {
        let calculator = self.blast_simple_calculator();
        calculator.is_none_or(|c| c.damages_entities)
    }
    /// `getKnockbackMultiplier`: 1 by default; a simple calculator's (a wind
    /// charge's 1.22 or 1), or 0 for a flying player (`Abilities.flying`).
    pub(in crate::game::server_side) fn blast_knockback(&self, flying_player: bool) -> f32 {
        match self.blast_simple_calculator() {
            Some(_) if flying_player => 0.0,
            Some(c) => c.knockback,
            None => 1.0,
        }
    }
    /// What players see and hear of the blast.
    pub(in crate::game::server_side) fn blast_effects(&self) -> BlastEffects {
        match &self.server.combat.0.blast_settings.effects {
            Some(effects) => (**effects).clone(),
            None => BlastEffects::default(),
        }
    }
}
