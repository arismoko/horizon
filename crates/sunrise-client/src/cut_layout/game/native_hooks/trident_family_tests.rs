//! `WitherBoss.hurtServer` and `Shulker.hurtServer` inspect the direct AbstractArrow,
//! not the damage type: ThrownTrident extends AbstractArrow but /damage does not.
use mod_api::{
    Flow, HitHost, Hooks, HurtSource, LivingDeathEvent, LivingIncomingDamageEvent, ModEntry,
    ServerHost,
};
use std::sync::{Arc, Mutex};
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    mobs::{Mob, Who},
    projectiles::{Projectile, ProjectileKind},
};

#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) struct Hit {
    pub(in crate::game) entity: u32,
    pub(in crate::game) damage_type: String,
    pub(in crate::game) causing: i32,
    pub(in crate::game) direct: i32,
    pub(in crate::game) position: Option<[f64; 3]>,
    pub(in crate::game) weapon: Option<mod_api::ItemStack>,
}
impl Hit {
    pub(crate) fn new(
        entity: u32,
        source: &HurtSource<'_>,
        weapon: Option<&mod_api::ItemStack>,
    ) -> Self {
        Self {
            entity,
            damage_type: source.damage_type.into(),
            causing: source.causing,
            direct: source.direct,
            position: source.position,
            weapon: weapon.cloned(),
        }
    }
}
#[derive(Default)]
pub(crate) struct Seen {
    pub(in crate::game) incoming: Vec<Hit>,
    pub(in crate::game) deaths: Vec<Hit>,
}
pub(crate) struct Probe(pub(crate) Arc<Mutex<Seen>>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().incoming.push(Hit::new(
            event.entity as u32,
            event.source,
            event
                .weapon
                .and_then(mod_api::ItemStackView::project)
                .as_ref(),
        ));
        Flow::Continue
    }
    fn living_death(&self, _: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        self.0
            .lock()
            .unwrap()
            .deaths
            .push(Hit::new(event.entity, event.source, event.weapon));
        Flow::Continue
    }
}
#[allow(unused_imports)]
pub(crate) use crate::game::server_side::native_hooks_server::trident_family_tests::fixture;
pub(crate) fn victim(game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str) -> f32 {
    let kind = game.res.mob_types().get(name).unwrap().clone();
    let mut mob = Mob::new(41, kind, Vec3::new(6.0, 80.0, 4.5), 0.0);
    mob.persistent = true;
    mob.no_ai = true;
    mob.ground.on_ground = true;
    mob.extra.counter = 0;
    if name == "minecraft:wither" {
        mob.health = mob.attrs.max_health / 2.0;
    }
    let health = mob.health;
    game.mobs.push(mob);
    health
}
pub(crate) fn weapon() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:trident", 1);
    stack.damage = 7;
    stack.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::compound([("trident_family_marker", Tag::Int(3414))])),
    );
    stack.data_mut().set_other(
        "minecraft:custom_name",
        Some(Tag::String("Thrown weapon".into())),
    );
    stack
}
pub(crate) fn throw_player(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (u32, ItemStack) {
    let selected = game.selected;
    game.inventory[selected] = Some(weapon());
    game.release_trident(10);
    assert_eq!(
        game.tridents.len(),
        1,
        "TridentItem.releaseUsing spawned the projectile"
    );
    let shot = &mut game.tridents[0];
    shot.position = Vec3::new(4.5, 80.5, 4.5);
    shot.previous = shot.position;
    shot.velocity = Vec3::new(4.0, 0.0, 0.0);
    (shot.id, shot.stack.clone())
}
pub(crate) fn throw_mob(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (u32, ItemStack) {
    // A non-undead owner ensures WitherBoss's separate WITHER_FRIENDS filter
    // cannot hide a missing direct-projectile immunity.
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    let mut owner = Mob::new(42, kind, Vec3::new(1.0, 80.0, 1.0), 0.0);
    owner.no_ai = true;
    owner.persistent = true;
    game.mobs.push(owner);
    let stack = weapon();
    let mut shot = Projectile::new(
        ProjectileKind::Trident,
        Vec3::new(4.5, 80.5, 4.5),
        Some(Who::Mob(42)),
    );
    shot.id = game.take_entity_id();
    shot.velocity = Vec3::new(4.0, 0.0, 0.0);
    shot.arrow.as_mut().unwrap().pickup_item = stack.clone();
    let id = shot.id;
    game.creatures.projectiles.push(shot);
    (id, stack)
}
pub(crate) fn step(game: &mut sunrise_gameplay::game::ServerGame<'_>, player: bool) {
    if player {
        game.tick_tridents();
    } else {
        game.tick_projectiles().unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn player_thrown_trident_is_refused_by_powered_wither_and_closed_shulker() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::player_thrown_trident_is_refused_by_powered_wither_and_closed_shulker",
        || {
            for kind in ["minecraft:wither", "minecraft:shulker"] {
                fixture(|game, seen| {
                    let health = victim(game, kind);
                    throw_player(game);
                    step(game, true);
                    assert_eq!(
                        game.mobs.iter().find(|m| m.id == 41).unwrap().health,
                        health,
                        "{kind}"
                    );
                    assert!(
                        game.tridents[0].velocity.x < 0.0,
                        "a real collision bounced the trident"
                    );
                    let seen = seen.lock().unwrap();
                    assert!(
                        seen.incoming.is_empty(),
                        "species immunity precedes incoming damage"
                    );
                    assert!(seen.deaths.is_empty());
                });
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn mob_thrown_trident_is_refused_by_powered_wither_and_closed_shulker() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::mob_thrown_trident_is_refused_by_powered_wither_and_closed_shulker",
        || {
            for kind in ["minecraft:wither", "minecraft:shulker"] {
                fixture(|game, seen| {
                    let health = victim(game, kind);
                    throw_mob(game);
                    step(game, false);
                    assert_eq!(
                        game.mobs.iter().find(|m| m.id == 41).unwrap().health,
                        health,
                        "{kind}"
                    );
                    assert!(
                        game.creatures.projectiles[0].velocity.x < 0.0,
                        "a real collision bounced the trident"
                    );
                    let seen = seen.lock().unwrap();
                    assert!(seen.incoming.is_empty());
                    assert!(seen.deaths.is_empty());
                });
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn accepted_player_and_mob_tridents_keep_type_entities_position_and_full_weapon() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::accepted_player_and_mob_tridents_keep_type_entities_position_and_full_weapon",
        || {
            for player in [true, false] {
                fixture(|game, seen| {
                    victim(game, "minecraft:cow");
                    game.mobs[0].health = 1.0;
                    let (id, stack) = if player {
                        throw_player(game)
                    } else {
                        throw_mob(game)
                    };
                    let expected = stack.try_into_api().unwrap();
                    let selected = game.selected;
                    game.inventory[selected] = Some(ItemStack::new("minecraft:stick", 1));
                    step(game, player);
                    assert!(game.mobs.iter().find(|m| m.id == 41).unwrap().health <= 0.0);
                    game.tick_creature_entities().unwrap();
                    let seen = seen.lock().unwrap();
                    assert_eq!(seen.incoming.len(), 1);
                    assert_eq!(seen.deaths, seen.incoming);
                    let hit = &seen.deaths[0];
                    assert_eq!(hit.entity, 41);
                    assert_eq!(hit.damage_type, "minecraft:trident");
                    assert_eq!(hit.causing, if player { 11 } else { 42 });
                    assert_eq!(hit.direct, id as i32);
                    assert!(hit.position.is_some());
                    assert_eq!(hit.weapon.as_ref(), Some(&expected));
                    assert!(expected.components().is_some());
                });
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn type_only_damage_trident_and_arrow_are_not_direct_abstract_arrows() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::type_only_damage_trident_and_arrow_are_not_direct_abstract_arrows",
        || {
            for kind in ["minecraft:wither", "minecraft:shulker"] {
                for source in ["minecraft:trident", "minecraft:arrow"] {
                    fixture(|game, seen| {
                        let health = victim(game, kind);
                        assert!(
                            game.run_command(&format!("damage @e[type={kind},limit=1] 2 {source}"))
                        );
                        assert!(
                            game.mobs[0].health < health,
                            "{kind} accepts type-only {source}"
                        );
                        let seen = seen.lock().unwrap();
                        assert_eq!(seen.incoming.len(), 1);
                        let hit = &seen.incoming[0];
                        assert_eq!(hit.damage_type, source);
                        assert_eq!((hit.causing, hit.direct), (0, 0));
                        assert_eq!(hit.position, None);
                        assert_eq!(hit.weapon, None);
                    });
                }
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn actual_arrows_still_bounce_off_powered_wither_and_closed_shulker() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::actual_arrows_still_bounce_off_powered_wither_and_closed_shulker",
        || {
            for kind in ["minecraft:wither", "minecraft:shulker"] {
                fixture(|game, seen| {
                    let health = victim(game, kind);
                    let bow = ItemStack::new("minecraft:bow", 1);
                    let selected = game.selected;
                    game.inventory[selected] = Some(bow.clone());
                    game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 8));
                    game.release_bow(&bow, 20);
                    game.creatures.projectiles.update(0, |shot| {
                        shot.position = Vec3::new(4.5, 80.5, 4.5);
                        shot.previous = shot.position;
                        shot.velocity = Vec3::new(4.0, 0.0, 0.0);
                    });
                    game.tick_projectiles().unwrap();
                    assert_eq!(game.mobs[0].health, health, "{kind}");
                    assert!(
                        game.creatures.projectiles[0].velocity.x < 0.0,
                        "the actual arrow bounced"
                    );
                    assert!(seen.lock().unwrap().incoming.is_empty());
                });
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn unlistened_projectile_factory_keeps_family_without_copying_weapon() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::unlistened_projectile_factory_keeps_family_without_copying_weapon",
        || {
            mod_platform::testing::with_mods(Vec::<ModEntry>::new().into(), || {
                with_game(|game| {
                    game.build_registries();
                    for kind in [
                        ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        },
                        ProjectileKind::Trident,
                        ProjectileKind::Snowball,
                    ] {
                        let is_arrow =
                            matches!(kind, ProjectileKind::Arrow { .. } | ProjectileKind::Trident);
                        let mut shot = Projectile::new(kind, Vec3::ZERO, None);
                        shot.id = 71;
                        if let Some(arrow) = shot.arrow.as_mut() {
                            arrow.pickup_item = weapon();
                            arrow.fired_from_weapon = Some(ItemStack::new("minecraft:bow", 1));
                        }
                        let attack = game.projectile_attack(&shot);
                        assert_eq!(attack.projectile, Some(71));
                        assert_eq!(attack.projectile_kind.is_arrow(), is_arrow);
                        assert!(attack.projectile_weapon.is_none());
                        assert!(attack.held.iter().all(Option::is_none));
                        assert_eq!(
                            attack.direct_is_arrow(&sunrise_sim::mobs::Damage::of(
                                game.projectile_damage_kind(&shot),
                            )),
                            is_arrow
                        );
                    }
                })
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn explicit_damage_by_actual_arrow_family_is_refused_even_with_generic_type() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::explicit_damage_by_actual_arrow_family_is_refused_even_with_generic_type",
        || {
            for kind in ["minecraft:wither", "minecraft:shulker"] {
                for trident in [true, false] {
                    fixture(|game, seen| {
                        let health = victim(game, kind);
                        let direct = if trident {
                            throw_player(game);
                            "minecraft:trident"
                        } else {
                            let bow = ItemStack::new("minecraft:bow", 1);
                            let selected = game.selected;
                            game.inventory[selected] = Some(bow.clone());
                            game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 8));
                            game.release_bow(&bow, 20);
                            "minecraft:arrow"
                        };
                        assert!(!game.run_command(&format!(
                            "damage @e[type={kind},limit=1] 2 minecraft:generic by @e[type={direct},limit=1] from @p",
                        )));
                        assert_eq!(game.mobs[0].health, health);
                        assert!(seen.lock().unwrap().incoming.is_empty());
                    });
                }
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn explicit_damage_by_trident_retains_its_weapon_with_independent_causing_entity() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::trident_family_tests::explicit_damage_by_trident_retains_its_weapon_with_independent_causing_entity",
        || {
            fixture(|game, seen| {
                victim(game, "minecraft:cow");
                game.mobs[0].health = 1.0;
                let (id, stack) = throw_player(game);
                let expected = stack.try_into_api().unwrap();
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:stick", 1));
                assert!(game.run_command(
                "damage @e[type=cow,limit=1] 2 minecraft:generic by @e[type=trident,limit=1] from @p",
            ));
                game.tick_creature_entities().unwrap();
                let seen = seen.lock().unwrap();
                assert_eq!(seen.incoming.len(), 1);
                assert_eq!(seen.deaths, seen.incoming);
                let hit = &seen.deaths[0];
                assert_eq!(hit.damage_type, "minecraft:generic");
                assert_eq!((hit.causing, hit.direct), (11, id as i32));
                assert!(hit.position.is_some());
                assert_eq!(hit.weapon.as_ref(), Some(&expected));
            })
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
