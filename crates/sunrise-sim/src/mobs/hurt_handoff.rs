//! A hit releases its level loan at steps that need the owning server.
use super::*;
use crate::health::Attack;

#[derive(Debug)]
pub(super) struct MobHurt {
    pub source: Damage,
    pub reflected: bool,
    pub crackiness: Option<Option<&'static str>>,
    pub forced_allow: bool,
    pub container: DamageContainer,
}

#[derive(Debug)]
pub(super) struct ReducedHurt {
    pub amount: f32,
    pub blocked: f32,
    pub blocked_hit: bool,
    pub incoming: f32,
    pub cooldown: bool,
    pub absorbed: bool,
    pub actually_hurt: bool,
    pub armor_hit: Option<f32>,
    pub local_magic: f32,
}

#[derive(Debug)]
pub(super) enum HurtStage {
    Wake,
    AfterResistance(ReducedHurt),
}

/// An owning-level action, before the corresponding continuation is allowed to run.
#[derive(Clone, Copy, Debug)]
pub enum MobHurtAction {
    Wake,
    AwardResistance { player: u32, tenths: i32 },
}

/// One suspended hit, held by the owning thread (not by a clone of the victim).
#[derive(Debug)]
pub struct PausedMobHurt {
    pub victim: u32,
    pub join: crate::join_seq::JoinSeq,
    pub attack: Attack,
    pub lent_attacker: Option<i32>,
    pub action: MobHurtAction,
    pub(super) direct_tail: Option<(&'static str, bool)>,
    aim: Option<Aim>,
    projectile_motion: Option<[f64; 2]>,
    creative_attacker: bool,
    hit: MobHurt,
    stage: HurtStage,
}

impl PausedMobHurt {
    pub fn attacker(&self) -> Option<Who> {
        self.hit.source.attacker
    }

    pub fn source_kind(&self) -> DamageKind {
        self.hit.source.kind
    }
}

impl Mob {
    pub(super) fn pause_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        hit: MobHurt,
        stage: HurtStage,
        action: MobHurtAction,
    ) {
        self.hurt_paused = true;
        level.pause_mob_hurt(PausedMobHurt {
            victim: self.id,
            join: self.join,
            attack: level.attack().clone(),
            lent_attacker: level.lent_attacker(),
            action,
            direct_tail: None,
            aim: self.attacker_aim,
            projectile_motion: self.projectile_motion,
            creative_attacker: self.creative_attacker,
            hit,
            stage,
        });
    }

    /// The owner extracted the continuation; nested damage can start on this body.
    pub fn release_hurt_pause(&mut self) {
        self.hurt_paused = false;
    }

    /// Continue after the owner has performed the wake or the stat award. Neither
    /// entry guards nor reduction modifiers that ran before the yield are replayed.
    pub fn resume_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        paused: PausedMobHurt,
        armor_effectiveness: Option<&mut dyn FnMut(f32, &mut LegacyRandom) -> f32>,
    ) -> bool {
        let PausedMobHurt {
            hit,
            stage,
            aim,
            projectile_motion,
            creative_attacker,
            direct_tail,
            ..
        } = paused;
        let (old_aim, old_motion, old_creative) = (
            self.attacker_aim,
            self.projectile_motion,
            self.creative_attacker,
        );
        self.attacker_aim = aim;
        self.projectile_motion = projectile_motion;
        self.creative_attacker = creative_attacker;
        self.hurt_paused = false;
        let source = hit.source;
        let hurt = match stage {
            HurtStage::Wake => self.hurt_after_wake(level, hit, armor_effectiveness),
            HurtStage::AfterResistance(reduced) => self.hurt_after_resistance(level, hit, reduced),
        };
        if self.hurt_paused {
            level.paused_mob_hurt().direct_tail = direct_tail;
        } else {
            self.finish_species_hurt(level, &source, true);
            if let Some((direct, cloud)) = direct_tail {
                self.finish_damage_by(hurt, &source, direct, cloud);
            }
        }
        self.attacker_aim = old_aim;
        self.projectile_motion = old_motion;
        self.creative_attacker = old_creative;
        hurt
    }
}
