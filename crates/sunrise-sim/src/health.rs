//! Stored player health and server damage for the resident scene.
//! World/audio/network consumers are queued explicitly; full death and tick
//! lifecycles (drops, respawn, effects) remain separate work.
use crate::{
    Result,
    items::ItemStack,
    motion::{MotionEvent, PlayerMotion},
};
pub use mod_api::entity::health::{HurtSource, ShieldBlock, Victim};
use std::{fs, io, path::Path};
use sunrise_world::{
    attributes::live::LiveAttributes,
    block_landing::LandingDamage,
    combat_rules::ArmorWorld,
    damage_immunity::{self, DamageRule, EntityImmunityWorld, ImmunityTag, ImmunityWorld},
    damage_reduction::Bypass,
    damage_source::{Catalog, DamageSource, DamageTypeId, Scaling},
    food::{Difficulty, FoodData},
    living_damage::{DamageStat, DamageState},
    living_hurt::{HurtState, HurtTag, HurtWorld, PendingHurt},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    player_hurt::{CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld},
    server_player_combat::{CombatEnvironment, ServerPlayerCombat},
    server_player_damage::{PlayerBodyRead, ServerDamageEnvironment},
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
pub(crate) type Attributes = LiveAttributes<Box<dyn sunrise_world::attributes::live::Identities>>;
pub(crate) type State = HurtState<DamageTypeId>;

pub use crate::world::HealthRules;
mod accepted_hit;
pub mod attack;
mod kill_credit;
use kill_credit::Credit;
pub use kill_credit::{KnockbackCredit, PlayerMemory, PlayerResolver};
mod hit_attempt;
pub use accepted_hit::AcceptedHit;
pub use attack::{damage_type, melee, mob_source, source_damage_type};
include!("health/hit.rs");

include!("health/damage.rs");

include!("health/player.rs");

include!("health/environment.rs");

#[cfg(test)]
mod datapack_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_hook_can_refuse_one_damage_type_and_allow_another() {
        use crate::mobs::{Damage, DamageKind, Mob};
        fn immune_to_generic(_: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
            match hurting {
                Hurting::Mob(_, hit) if matches!(hit.kind, DamageKind::Generic) => true,
                _ => vanilla,
            }
        }
        let hooks = DamageHooks {
            invulnerable: Some(immune_to_generic),
            ..DamageHooks::NONE
        };
        let mut test_level = TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        let mut mob = Mob::new(
            9201,
            crate::mobs::sensor_scan_tests::kind(),
            Default::default(),
            0.0,
        );
        assert!(!mob.damage(level, 2.0, Damage::of(DamageKind::Generic)));
        assert_eq!(mob.health, 10.0);
        assert!(mob.damage(level, 2.0, Damage::of(DamageKind::Fall)));
        assert_eq!(mob.health, 8.0);
    }

    #[test]
    fn the_weapon_is_the_attackers_main_hand_only_for_its_own_hit() {
        let sword = crate::items::ItemStack::new("minecraft:iron_sword", 1);
        let attack = Attack {
            attacker: 7,
            projectile: None,
            direct: None,
            player_uuid: None,
            attacker_uuid: None,
            held: [Some(sword.clone()), None],
            ..Default::default()
        };
        assert_eq!(attack.weapon_of(7), Some(&sword));
        // An arrow the attacker shot is the direct entity, not the attacker: no weapon.
        assert_eq!(attack.weapon_of(9), None);
        assert_eq!(attack.weapon_of(0), None);
        assert_eq!(NO_ATTACK.weapon_of(0), None);
    }
    #[test]
    fn magic_during_a_projectile_impact_names_no_direct_entity() {
        // DamageSources.magic() is entity-free. An ownerless wither skull and a
        // tipped arrow's instant harm both deal it while the projectile's attack
        // is still in scope; arrow, wither_skull and indirect_magic name it.
        let attack = Attack {
            attacker: 0,
            projectile: Some(12),
            direct: None,
            player_uuid: None,
            attacker_uuid: None,
            held: [None, None],
            ..Default::default()
        };
        assert_eq!(attack.projectile_direct("minecraft:arrow"), 12);
        assert_eq!(attack.projectile_direct("minecraft:wither_skull"), 12);
        assert_eq!(attack.projectile_direct("minecraft:spit"), 12);
        assert_eq!(attack.projectile_direct("minecraft:indirect_magic"), 12);
        assert_eq!(attack.projectile_direct("minecraft:magic"), 0);
        assert_eq!(attack.projectile_direct("minecraft:explosion"), 0);
        assert_eq!(NO_ATTACK.projectile_direct("minecraft:arrow"), 0);
    }
    #[test]
    fn a_knockback_hook_cannot_make_the_push_nonfinite() {
        fn hook(_: DamageLevel<'_>, _: Hurting<'_>, knock: &mut Knock) -> bool {
            knock.strength = f64::NAN;
            knock.x *= 2.0;
            false
        }
        let mut level = crate::health::TestLevel::with_hooks(
            0,
            DamageHooks {
                knockback: Some(hook),
                ..DamageHooks::NONE
            },
        );
        let source = HurtSource {
            damage_type: "minecraft:generic",
            causing: 0,
            direct: 0,
            position: None,
            direct_type: None,
        };
        let mut knock = Knock {
            strength: 0.4,
            x: 1.0,
            z: 0.5,
        };
        assert!(
            !level
                .level()
                .knockback_canceled(Hurting::Player(1, source), &mut knock)
        );
        // A value left that is not a number is put back; one that is finite stays changed.
        assert_eq!(
            knock,
            Knock {
                strength: 0.4,
                x: 2.0,
                z: 0.5
            }
        );
    }
    #[test]
    fn a_hit_in_the_hurt_cooldown_knocks_nobody_back() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO)
                .unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let mut hit = |amount| {
            player
                .hurt_blocking(level, health, rules, id, amount, None)
                .unwrap()
        };
        let first = hit(1.0);
        assert!(first.hurt && first.knockback);
        // A stronger hit inside the cooldown hurts by the difference only
        // (LivingEntity.hurtServer), with no dealDefaultKnockback.
        let second = hit(2.0);
        assert!(second.hurt && !second.knockback);
        // A weaker one doesn't hurt at all.
        let third = hit(1.5);
        assert!(!third.hurt && !third.knockback);
    }

    #[test]
    fn player_post_records_fresh_hit_decision() {
        struct Observed {
            should_cause_side_effects: bool,
            health_damage: f32,
            post_attack_invulnerability_ticks: i32,
            invulnerability_reduction: f32,
        }
        static SEEN: std::sync::Mutex<Vec<Observed>> = std::sync::Mutex::new(Vec::new());
        fn post(
            _: super::DamageLevel<'_>,
            _: super::Hurting<'_>,
            container: &super::DamageContainer,
        ) {
            SEEN.lock().unwrap().push(Observed {
                should_cause_side_effects: container.should_cause_side_effects(),
                health_damage: container.health_damage(),
                post_attack_invulnerability_ticks: container.post_attack_invulnerability_ticks(),
                invulnerability_reduction: container.reduction(super::Reduction::Invulnerability),
            });
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player = crate::motion::PlayerMotion::new(&resources, Default::default()).unwrap();
        let hooks = super::DamageHooks {
            post: Some(post),
            ..super::DamageHooks::NONE
        };
        let mut level = super::TestLevel::with_hooks(0, hooks);
        let health = &resources.health;
        let rules = super::HealthRules::default();
        let attack = health.damage.id("minecraft:mob_attack").unwrap();
        assert!(
            player
                .hurt(&mut level.level(), health, rules, attack, 2.0)
                .unwrap()
        );
        assert!(
            player
                .hurt(&mut level.level(), health, rules, attack, 4.0)
                .unwrap()
        );
        assert!(
            !player
                .hurt(&mut level.level(), health, rules, attack, 3.0)
                .unwrap()
        );
        let seen = SEEN.lock().unwrap();
        assert_eq!(seen.len(), 2, "the refused hit must not fire Post");
        let flags: Vec<_> = seen
            .iter()
            .map(|hit| hit.should_cause_side_effects)
            .collect();
        assert_eq!(flags, [true, false]);
        let invuln: Vec<_> = seen
            .iter()
            .map(|hit| hit.invulnerability_reduction)
            .collect();
        assert_eq!(invuln, [0.0, 2.0]);
        // Both first hits deal 2 and leave 20 ticks: neither value determines freshness.
        for hit in &seen[..2] {
            assert_eq!(hit.health_damage, 2.0);
            assert_eq!(hit.post_attack_invulnerability_ticks, 20);
        }
    }

    /// #2800: on Easy a mob's 4 is scaled to 3 before `applyItemBlocking`,
    /// so a shield that stops all 3 blocks fully (`damageBlocked >= damage`)
    /// and `blockedByItem` must not pull the player toward the mob.
    #[test]
    fn a_shield_fully_blocks_a_hit_scaled_down_by_easy() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        use sunrise_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO)
                .unwrap();
        let health = &resources.health;
        let rules = super::HealthRules {
            difficulty: sunrise_world::food::Difficulty::Easy,
            ..Default::default()
        };
        let shield = super::Blocking {
            rules: BlockingRules {
                delay_seconds: 0.25,
                disable_scale: 1.0,
                reductions: vec![DamageReduction {
                    horizontal_angle: 90.0,
                    types: None,
                    base: 0.0,
                    factor: 1.0,
                }],
                item_damage: ItemDamageFunction {
                    threshold: 3.0,
                    base: 1.0,
                    factor: 1.0,
                },
            },
            bypassed: false,
            pierce: false,
            angle: 0.0,
        };
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let o = player
            .hurt_caused_by(level, health, rules, id, 4.0, Some(shield), true, 0, false)
            .unwrap();
        assert_eq!(o.blocked, 3.0, "Easy scales 4 to 3 before the shield");
        assert!(o.fully_blocked && !o.knockback);
    }

    /// #7309: on Hard a mob's 4 is scaled to 6 (`Player.hurtServer`), on
    /// Normal it stays 4, and a non-mob source is never scaled.
    #[test]
    fn hard_scales_a_mob_hit_by_one_and_a_half() {
        use sunrise_world::food::Difficulty;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let health = &resources.health;
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let lost = |difficulty, living| {
            let mut player =
                crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO)
                    .unwrap();
            let rules = super::HealthRules {
                difficulty,
                ..Default::default()
            };
            let before = player.health.health();
            player
                .hurt_caused_by(
                    &mut super::TestLevel::new(0).level(),
                    health,
                    rules,
                    id,
                    4.0,
                    None,
                    living,
                    0,
                    false,
                )
                .unwrap();
            before - player.health.health()
        };
        assert_eq!(lost(Difficulty::Hard, true), 6.0);
        assert_eq!(lost(Difficulty::Normal, true), 4.0);
        assert_eq!(lost(Difficulty::Easy, true), 3.0);
        assert_eq!(lost(Difficulty::Hard, false), 4.0);
    }

    /// #2141: an arrow that takes the last of the player's health must kill
    /// them: `LivingEntity.hurt` leaves the health at zero and `Player.die`
    /// follows.
    #[test]
    fn an_arrow_kills_a_player_at_one_health() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO)
                .unwrap();
        player.set_health(1.0, false).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:arrow").unwrap();
        let hurt = player.hurt(level, health, rules, id, 5.0).unwrap();
        assert!(hurt, "the arrow must hurt");
        assert_eq!(player.health.health(), 0.0, "the health must be spent");
        assert!(player.health.dead_or_dying(), "the player must be dead");
    }

    /// What the hook of these tests saw: entity, damage type (a mob's kind)
    /// and causing entity, amount.
    static SEEN: std::sync::Mutex<Vec<(i32, String, f32)>> = std::sync::Mutex::new(Vec::new());
    /// Sets the native damage hook of these tests (global: it stays set for
    /// the process): for entities 9001 to 9004 and 9011 to 9013 it records the
    /// hit, cancels x1's, doubles x2's, and gives 9005's hit armor and absorption modifiers.
    fn set_hook() -> (super::DamageHooks, fn(i32) -> Vec<(String, f32)>) {
        use super::{DamageContainer, DamageLevel, Hurting, Victim};
        fn hook(
            _: DamageLevel<'_>,
            hurting: Hurting<'_>,
            _: Victim,
            container: &mut DamageContainer,
        ) -> bool {
            let (entity, seen) = match hurting {
                Hurting::Player(e, s)
                | Hurting::Dragon(e, s)
                | Hurting::ArmorStand(e, s)
                | Hurting::Entity(e, _, s) => (e, format!("{} {}", s.damage_type, s.causing)),
                Hurting::Mob(mob, source) => (mob.id as i32, format!("{:?}", source.kind)),
            };
            if !matches!(entity, 9001..=9005 | 9011..=9013) {
                return false;
            }
            let amount = container.new_damage();
            SEEN.lock().unwrap().push((entity, seen, amount));
            match entity {
                9002 | 9012 => container.record_new_damage(amount * 2.0),
                9005 => {
                    use sunrise_world::living_damage::Reduction;
                    container.add_reduction_modifier(
                        Reduction::Armor,
                        Box::new(|_: &DamageContainer, v: f32| v + 1.0),
                    );
                    container.add_reduction_modifier(
                        Reduction::Armor,
                        Box::new(|_: &DamageContainer, v: f32| v * 2.0),
                    );
                    container.add_reduction_modifier(
                        Reduction::Absorption,
                        Box::new(|_: &DamageContainer, v: f32| v * 0.5),
                    );
                }
                _ => {}
            }
            entity % 10 == 1
        }
        let hooks = super::DamageHooks {
            incoming: Some(hook),
            ..super::DamageHooks::NONE
        };
        (hooks, |entity| {
            let seen = SEEN.lock().unwrap();
            let hits = seen.iter().filter(|s| s.0 == entity);
            hits.map(|s| (s.1.clone(), s.2)).collect()
        })
    }

    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_mobs_hit() {
        let (hooks, seen) = set_hook();
        let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        use crate::mobs::{Damage, DamageKind, Mob};
        let mob = |id| {
            Mob::new(
                id,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            )
        };
        let hit = Damage {
            kind: DamageKind::PlayerAttack,
            attacker: None,
            from: Some([3.0, 0.0, 0.0]),
            direct: None,
            attacker_species: None,
        };
        // Canceled: no damage, cooldown, sound or knockback.
        let mut kept = mob(9001);
        assert!(!kept.damage(level, 2.0, hit));
        assert_eq!(
            (kept.health, kept.invulnerable, kept.hurt_time),
            (10.0, 0, 0)
        );
        assert!(kept.outbox.is_empty() && kept.velocity == Default::default());
        assert_eq!(seen(9001), [("PlayerAttack".to_owned(), 2.0)]);
        let mut doubled = mob(9002);
        assert!(doubled.damage(level, 2.0, hit));
        assert_eq!(doubled.health, 6.0);
        // 8 damage on a mob with 1.5 absorption hearts. Armor took none: +1 then x2 is 2, so 6
        // goes on. The hearts would take their 1.5; half of it is 0.75, so 5.25 reaches the
        // health (10 - 5.25) and 0.75 hearts are left.
        let mut modified = mob(9005);
        modified.add_effect("absorption", 600, 0);
        modified.absorption = 1.5;
        assert!(modified.damage(level, 8.0, hit));
        assert_eq!((modified.health, modified.absorption), (4.75, 0.75));
        // The same hit with no modifier: 8 less the 1.5 hearts.
        let mut plain = mob(9006);
        plain.add_effect("absorption", 600, 0);
        plain.absorption = 1.5;
        assert!(plain.damage(level, 8.0, hit));
        assert_eq!((plain.health, plain.absorption), (3.5, 0.0));
    }

    /// Needs the prepared data in `.local/sunrise-client/26.3`; skipped
    /// without it.
    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_players_hit() {
        let (hooks, seen) = set_hook();
        let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:arrow").unwrap();
        let player = |entity| {
            let mut p = crate::motion::PlayerMotion::new(&resources, Default::default()).unwrap();
            p.entity_id = entity;
            p
        };
        let mut kept = player(9011);
        assert!(!kept.hurt(level, health, rules, id, 5.0).unwrap());
        assert_eq!((kept.health.health(), kept.health.hurt.cooldown), (20.0, 0));
        let mut doubled = player(9012);
        assert!(doubled.hurt(level, health, rules, id, 5.0).unwrap());
        assert_eq!(doubled.health.health(), 10.0);
        // The causing mob when it is known, else 0.
        let hurt =
            player(9013).hurt_caused_by(level, health, rules, id, 5.0, None, false, 44, false);
        assert!(hurt.unwrap().hurt);
        assert_eq!(seen(9011), [("minecraft:arrow 0".to_owned(), 5.0)]);
        assert_eq!(seen(9013), [("minecraft:arrow 44".to_owned(), 5.0)]);
    }

    /// What the knockback hook of these tests saw: the entity, the hit that
    /// causes it (a mob's damage kind) and the push it was handed.
    static PUSHES: std::sync::Mutex<Vec<(i32, String, super::Knock)>> =
        std::sync::Mutex::new(Vec::new());
    /// Sets the native knockback hook of these tests for mobs 9101 to 9106 only: it records the push, cancels
    /// 9101's, doubles 9102's strength and turns its direction around, leaves
    /// 9103 non-finite numbers, and gives 9105 (whose resistance leaves
    /// nothing) a strength of 0.4.
    fn set_knockback_hook() -> (super::DamageHooks, fn(i32) -> Vec<(String, super::Knock)>) {
        use super::{DamageLevel, Hurting, Knock};
        fn hook(_: DamageLevel<'_>, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
            let Hurting::Mob(mob, source) = pushed else {
                return false;
            };
            let entity = mob.id as i32;
            if !(9101..=9106).contains(&entity) {
                return false;
            }
            let hit = format!("{:?}", source.kind);
            PUSHES.lock().unwrap().push((entity, hit, *knock));
            match entity {
                9101 => return true,
                9102 => {
                    knock.strength *= 2.0;
                    knock.x = -knock.x;
                }
                9103 => {
                    knock.strength = f64::NAN;
                    knock.x = f64::INFINITY;
                    knock.z = f64::NEG_INFINITY;
                }
                9105 => knock.strength = 0.4,
                _ => {}
            }
            false
        }
        let hooks = super::DamageHooks {
            knockback: Some(hook),
            ..super::DamageHooks::NONE
        };
        (hooks, |entity| {
            let seen = PUSHES.lock().unwrap();
            let pushes = seen.iter().filter(|p| p.0 == entity);
            pushes.map(|p| (p.1.clone(), p.2)).collect()
        })
    }

    /// A zombie in full netherite (0.1 a piece) is pushed 60 % as far as one
    /// with no armour: `knockback` scales by the attribute's value with the
    /// gear's modifiers, which the game hands the mob as `gear_knockback`.
    #[test]
    fn a_mobs_gear_resists_knockback() {
        use crate::mobs::{Damage, Mob};
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let hit = Damage::player([3.0, 0.0, 0.0]);
        let mut pushed = |gear: Option<f64>, own: f64| {
            let mut m = Mob::new(
                9200,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            );
            m.ground.on_ground = true;
            m.attrs.knockback_resistance = own;
            m.extra.gear_knockback = gear.map(|amount| {
                Box::new(crate::mobs::GearKnockback {
                    worn: Default::default(),
                    modifiers: vec![(amount, crate::mobs::finalize::Op::AddValue)],
                })
            });
            m.knockback(level, 0.4, 3.0, 0.0, &hit, 0.0);
            m.velocity.x
        };
        assert_eq!(pushed(None, 0.0), -0.4);
        assert_eq!(pushed(None, 0.5), -0.2);
        assert!((pushed(Some(0.4), 0.0) - -0.4 * 0.6).abs() < 1e-12);
    }
    /// `LivingEntity.knockback`: the hook runs before knockback resistance
    /// takes its share and before motion changes; a mod's strength or
    /// direction is what pushes, a cancel pushes nothing, and with no mod in
    /// the way the push is vanilla's. Expected values are worked out by hand
    /// from vanilla's formula: the horizontal speed halves and the push is
    /// subtracted along the normalized direction; on the ground the vertical
    /// speed becomes `min(0.4, vy / 2 + strength)`.
    #[test]
    fn the_native_knockback_hook_changes_or_cancels_a_mobs_push() {
        let (hooks, seen) = set_knockback_hook();
        let mut test_level = crate::health::TestLevel::with_hooks(0, hooks);
        let level = &mut test_level.level();
        use super::Knock;
        use crate::mobs::{Damage, Mob};
        let mob = |id, resistance| {
            let mut m = Mob::new(
                id,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            );
            m.ground.on_ground = true;
            m.attrs.knockback_resistance = resistance;
            m
        };
        let velocity = |m: &Mob| (m.velocity.x, m.velocity.y, m.velocity.z);
        let hit = Damage::player([3.0, 0.0, 0.0]);
        let mut push = |id, resistance| {
            let mut m = mob(id, resistance);
            m.knockback(level, 0.4, 3.0, 0.0, &hit, 0.0);
            m
        };
        // No mod in the way (the hook leaves 9100 alone): vanilla's push, away from x = 3.
        let plain = push(9100, 0.0);
        assert_eq!(velocity(&plain), (-0.4, 0.4, 0.0));
        assert!(plain.needs_sync);
        // Canceled: no motion, no resync. The hook saw the push the game made.
        let kept = push(9101, 0.0);
        assert_eq!(velocity(&kept), (0.0, 0.0, 0.0));
        assert!(!kept.needs_sync);
        let made = Knock {
            strength: 0.4,
            x: 3.0,
            z: 0.0,
        };
        assert_eq!(seen(9101), [("PlayerAttack".to_owned(), made)]);
        // A stronger push the other way: twice the strength, x turned around.
        let turned = push(9102, 0.0);
        assert_eq!(velocity(&turned), (0.8, 0.4, 0.0));
        assert_eq!(velocity(&push(9102, 0.5)), (0.4, 0.4, 0.0));
        // Numbers that are not finite are put back as the game made them.
        assert_eq!(velocity(&push(9103, 0.0)), (-0.4, 0.4, 0.0));
        // Before the resistance (`LivingKnockBackEvent`): the hook is handed the raw 0.4, and
        // the push that follows is halved.
        let resisted = push(9104, 0.5);
        assert_eq!(velocity(&resisted), (-0.2, 0.2, 0.0));
        assert_eq!(seen(9104)[0].1.strength, 0.4);
        // A full resistance takes all of what the hook leaves: no push.
        let stopped = push(9105, 1.0);
        assert_eq!(seen(9105)[0].1.strength, 0.4);
        assert_eq!(velocity(&stopped), (0.0, 0.0, 0.0));
        // The game's own hit on a mob reaches the hook with its damage.
        let mut hurt = mob(9106, 0.0);
        assert!(hurt.damage(level, 2.0, hit));
        // dealDefaultKnockback pushes with the float 0.4F.
        let strength = f64::from(0.4_f32);
        assert_eq!(velocity(&hurt), (-strength, 0.4, 0.0));
        let made = Knock { strength, ..made };
        assert_eq!(seen(9106), [("PlayerAttack".to_owned(), made)]);
    }

    /// #4301: `ServerPlayer.isInvulnerableTo`'s `!connection.hasClientLoaded`
    /// term spares a respawned player the void until the client has loaded the
    /// world; once it has, the void hurts again.
    #[test]
    fn the_void_spares_a_player_whose_client_has_not_loaded() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, sunrise_core::geometry::Vec3::ZERO)
                .unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:out_of_world").unwrap();
        player.client_loaded = false;
        assert!(
            !player.hurt(level, health, rules, id, 4.0).unwrap(),
            "an unloaded client must not take void damage"
        );
        assert_eq!(player.health.health(), 20.0);
        player.client_loaded = true;
        assert!(
            player.hurt(level, health, rules, id, 4.0).unwrap(),
            "a loaded client takes the void's 4.0"
        );
        assert_eq!(player.health.health(), 16.0);
    }

    #[test]
    fn hurt_sounds_follow_damage_effects() {
        // DamageEffects: HURT, THORNS, DROWNING, BURNING, POKING, FREEZING.
        for (effects, sound) in [
            ("hurt", "minecraft:entity.player.hurt"),
            ("thorns", "minecraft:entity.player.hurt"),
            ("drowning", "minecraft:entity.player.hurt_drown"),
            ("burning", "minecraft:entity.player.hurt_on_fire"),
            ("poking", "minecraft:entity.player.hurt_sweet_berry_bush"),
            ("freezing", "minecraft:entity.player.hurt_freeze"),
        ] {
            assert_eq!(super::hurt_sound(effects), sound, "{effects}");
        }
    }

    /// `LivingEntity.getDamageAfterArmorAbsorb` returns before
    /// `getArmorValue` and `getAttributeValue(ARMOR_TOUGHNESS)` for a source in
    /// `#minecraft:bypasses_armor`, and native `AttributeInstance.getValue` is
    /// what clears an instance's dirty flag. A hit must therefore leave both
    /// instances dirty with creation's zero cache when it bypasses the armour,
    /// and recompute them when it does not.
    #[test]
    fn a_bypassing_hit_leaves_the_armor_instances_dirty() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        // `setBaseValue` is what a command or a spawn uses; a base other than
        // the attribute's default is what makes the fresh instance's cache
        // stale, and it is the value a recompute would produce.
        let armoured = |resources: &crate::motion::MotionResources| {
            let mut player =
                crate::motion::PlayerMotion::new(resources, sunrise_core::geometry::Vec3::ZERO)
                    .unwrap();
            let mut handles = Vec::new();
            for name in ["minecraft:armor", "minecraft:armor_toughness"] {
                let key = player.attributes.key(name).unwrap();
                let handle = player
                    .attributes
                    .get_instance(Some(key))
                    .unwrap()
                    .expect("the player supplier has this attribute");
                player
                    .attributes
                    .edit(&handle, |a, o| a.set_base_value(7.0, o))
                    .unwrap();
                handles.push(handle);
            }
            (player, handles)
        };
        let states = |player: &crate::motion::PlayerMotion,
                      handles: &[sunrise_world::attributes::live::Handle]| {
            handles
                .iter()
                .map(|handle| {
                    let a = player.attributes.instance(handle).unwrap();
                    (a.base_value(), a.is_dirty(), a.cached_value())
                })
                .collect::<Vec<_>>()
        };
        // `minecraft:fall` is in the tag: neither instance is read.
        let (mut bypassing, bypassing_handles) = armoured(&resources);
        let fall = health.damage.id("minecraft:fall").unwrap();
        assert!(bypassing.hurt(level, health, rules, fall, 5.0).unwrap());
        assert_eq!(
            states(&bypassing, &bypassing_handles),
            [(7.0, true, 0.0), (7.0, true, 0.0)],
            "a bypassing hit must leave both instances dirty and uncached"
        );
        let record = bypassing.hurt_log.last().unwrap();
        assert_eq!(
            (record.armor, record.toughness),
            (0.0, 0.0),
            "the hit reports the cache it left"
        );
        // `minecraft:mob_attack` is not: `getDamageAfterAbsorb` asks for both
        // values, so the hit recomputes them and the record reads them back.
        let (mut meeting, meeting_handles) = armoured(&resources);
        let attack = health.damage.id("minecraft:mob_attack").unwrap();
        assert!(meeting.hurt(level, health, rules, attack, 5.0).unwrap());
        assert_eq!(
            states(&meeting, &meeting_handles),
            [(7.0, false, 7.0), (7.0, false, 7.0)],
            "a hit the armour meets must recompute both instances"
        );
        let record = meeting.hurt_log.last().unwrap();
        assert_eq!((record.armor, record.toughness), (7.0, 7.0));
    }

    #[test]
    fn resolve_damage_kind_maps_builtins_and_rejects_unknown_without_catalog() {
        use crate::mobs::DamageKind;
        assert_eq!(
            resolve_damage_kind(None, "minecraft:in_fire"),
            Some(DamageKind::InFire)
        );
        assert_eq!(
            resolve_damage_kind(None, "generic"),
            Some(DamageKind::Generic)
        );
        assert_eq!(resolve_damage_kind(None, "mod:unknown"), None);
    }
}
