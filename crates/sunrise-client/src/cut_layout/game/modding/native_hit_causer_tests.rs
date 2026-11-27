//! A hit's listener reads its causing entity (`DamageSource.getEntity`) at the damage sites
//! beyond melee: a projectile's shooter, an explosion's igniter, the wearer of Thorns, a
//! pufferfish stinging a mob inside its own tick, a firework's shooter, an evoker for its fangs,
//! a wither for its skull, a trident's thrower, a wind charge's owner, and a parked guest, read
//! from its hub record with its own attachments, also when it hurts the player in place and when
//! a lethal hit's Post waits for the death. A victim that caused its own hit is read as the
//! victim.
use super::native_entity_refs::tests::{bind_local_player, run_fresh_cases, with_game};
use mod_api::{
    AttachmentDescriptor, AttachmentKey, Flow, HitHost, Holder, Hooks, LivingDamagePostEvent,
    LivingIncomingDamageEvent, LivingView,
};
use std::sync::{Arc, Mutex};
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::server_side::native_living_refs::tests::summon_mob;
use sunrise_sim::mobs::{Damage, DamageKind, Who};
use sunrise_sim::motion::PlayerMotion;
use sunrise_sim::projectiles::{Hit, Projectile, ProjectileKind};

/// The probe's attachment, set to different values on the players.
const MARK: AttachmentDescriptor<i32> = AttachmentDescriptor::new("probe:mark", || -1);

/// What a listener read of a hit's causing entity: its id, health and the probe's attachment on
/// its holder.
#[derive(Clone, Debug, PartialEq)]
struct Causer {
    id: u32,
    health: Option<f32>,
    mark: Option<i32>,
}

/// One hook call: which, the hit entity, the causing id its source names, the causer it read,
/// and the victim's id.
#[derive(Clone, Debug)]
struct Seen {
    hook: &'static str,
    entity: i32,
    source_causing: i32,
    causing: Option<Causer>,
    victim: Option<u32>,
}

struct Probe {
    seen: Arc<Mutex<Vec<Seen>>>,
    mark: Arc<Mutex<Option<AttachmentKey<i32>>>>,
}

impl Probe {
    fn seen(&self, hook: &'static str, entity: i32, source_causing: i32, level: &mut dyn HitHost) {
        let mark = self
            .mark
            .lock()
            .unwrap()
            .clone()
            .expect("probe:mark registered");
        let read = |view: LivingView<'_>| (view.id(), view.health(), view.holder());
        let causing = level.causing().map(read);
        let victim = level.victim().map(|view| view.id());
        let causing = causing.map(|(id, health, holder)| Causer {
            id,
            health,
            mark: holder
                .and_then(|holder| {
                    mark.get_existing_in(level, holder, |value| value.copied())
                        .ok()
                })
                .flatten(),
        });
        self.seen.lock().unwrap().push(Seen {
            hook,
            entity,
            source_causing,
            causing,
            victim,
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        *self.mark.lock().unwrap() = Some(r.register_attachment_type(MARK).unwrap());
    }
    fn living_incoming_damage(
        &self,
        level: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        self.seen("incoming", event.entity, event.source.causing, level);
        Flow::Continue
    }
    fn living_damage_post(&self, level: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
        self.seen("post", event.entity, event.source.causing, level);
    }
}

/// The calls `hook` made on `victim`, taken out of the probe's record.
fn calls(seen: &Mutex<Vec<Seen>>, hook: &str, victim: u32) -> Vec<Seen> {
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    let calls: Vec<_> = seen
        .into_iter()
        .filter(|s| s.hook == hook && s.entity == victim as i32)
        .collect();
    assert!(!calls.is_empty(), "no {hook} on {victim}");
    calls
}

/// Each `living_incoming_damage` on `victim` names `causer` and read it with this health and mark.
fn assert_caused(
    seen: &Mutex<Vec<Seen>>,
    victim: u32,
    causer: u32,
    health: f32,
    mark: Option<i32>,
) {
    for hit in calls(seen, "incoming", victim) {
        assert_eq!(hit.source_causing, causer as i32, "{hit:?}");
        let expected = Causer {
            id: causer,
            health: Some(health),
            mark,
        };
        assert_eq!(hit.causing, Some(expected), "{hit:?}");
    }
}

fn with_probe(
    test: impl FnOnce(
        &mut sunrise_gameplay::game::ServerGame<'_>,
        &Mutex<Vec<Seen>>,
        &AttachmentKey<i32>,
    ),
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mark = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe {
            seen: seen.clone(),
            mark: mark.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mark = mark.lock().unwrap().clone().expect("probe:mark registered");
            game.player =
                Some(PlayerMotion::new(game.motion, Vec3::new(40.0, 70.0, 40.0)).unwrap());
            bind_local_player(game, 7, 7);
            game.set_game_mode(GameMode::Survival);
            test(game, &seen, &mark);
        })
    });
}

fn with_guest_probe(
    test: impl FnOnce(
        &mut sunrise_gameplay::game::ServerGame<'_>,
        &Mutex<Vec<Seen>>,
        &AttachmentKey<i32>,
    ),
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mark = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe {
            seen: seen.clone(),
            mark: mark.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_guest_turn_tests::with_guests(|game| {
            let mark = mark.lock().unwrap().clone().expect("probe:mark registered");
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                })
                .unwrap();
            test(game, &seen, &mark)
        })
    });
}

/// A blast centred on `at`, whose causing entity is `owner` (a player when `player`).
fn blast(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    at: [f64; 3],
    owner: u32,
    player: bool,
    guest: Option<u32>,
) {
    game.server.combat.0.blast_owner =
        Some((owner, sunrise_game_data::text::Text::literal("owner")));
    game.server.combat.0.blast_owner_player = player;
    game.server.combat.0.blast_guest = guest;
    game.server_explode(
        at,
        3.0,
        sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
        false,
    )
    .unwrap();
    game.server.combat.0.blast_owner = None;
    game.server.combat.0.blast_owner_player = false;
    game.server.combat.0.blast_guest = None;
}

/// A projectile of `kind` owned by `owner`, at the origin.
fn shot(kind: ProjectileKind, owner: Who) -> Projectile {
    let mut p = Projectile::new(kind, Vec3::new(0.5, 70.0, 0.5), Some(owner));
    p.id = 912;
    p
}

fn mob_at(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Vec3 {
    game.mobs.iter().find(|m| m.id == id).unwrap().position
}

fn move_mob(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, to: Vec3) {
    game.mobs.iter_mut().find(|m| m.id == id).unwrap().position = to;
}

fn projectile(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let shooter = summon_mob(game, "minecraft:skeleton", "{NoAI:1b}");
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    let arrow = shot(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Who::Mob(shooter),
    );
    let mut damage = Damage::of(DamageKind::Arrow);
    damage.attacker = Some(Who::Mob(shooter));
    damage.from = Some([0.5, 70.0, 0.5]);
    assert!(game.damage_projectile_with_weapon(&arrow, pig, 2.0, damage, None));
    assert_caused(seen, pig, shooter, 20.0, None);
}

fn explosion(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let creeper = summon_mob(game, "minecraft:creeper", "{NoAI:1b}");
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    // The creeper stands out of the blast; the pig in it.
    move_mob(game, creeper, Vec3::new(30.5, 70.0, 0.5));
    blast(game, [1.5, 70.0, 0.5], creeper, false, None);
    assert_caused(seen, pig, creeper, 20.0, None);
}

fn thorns(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    seen: &Mutex<Vec<Seen>>,
    mark: &AttachmentKey<i32>,
) {
    let zombie = summon_mob(game, "minecraft:zombie", "{NoAI:1b}");
    let mut chest = sunrise_items::items::ItemStack::new("minecraft:diamond_chestplate", 1);
    chest.data_mut().enchantments = vec![("minecraft:thorns".into(), 255)];
    game.armor[1] = Some(chest);
    mark.set(game, Holder::Player(7), 9).unwrap();
    game.thorns(zombie);
    assert_caused(seen, zombie, 7, 20.0, Some(9));
}

fn sting(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    // A server level whose chunks around the origin are loaded, so the mob tick runs there.
    game.options.dedicated = Some(Vec::new());
    for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
        let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
        chunk.lit = true;
        game.world.chunks.insert((x, z), Arc::new(chunk));
    }
    game.player.as_mut().unwrap().position = Vec3::new(3.5, 70.0, 3.5);
    let fish = summon_mob(game, "minecraft:pufferfish", "{NoAI:1b}");
    let cow = summon_mob(game, "minecraft:cow", "{NoAI:1b}");
    game.mobs
        .iter_mut()
        .find(|m| m.id == fish)
        .unwrap()
        .extra
        .size = 2;
    game.world.game_time += 1;
    game.tick_mobs().unwrap();
    assert_caused(seen, cow, fish, 3.0, None);
}

fn firework(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let shooter = summon_mob(game, "minecraft:skeleton", "{NoAI:1b}");
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    game.rocket_hurt_mobs(
        &[(pig, 5.0)],
        950,
        Some(Who::Mob(shooter)),
        [0.5, 70.0, 0.5],
    );
    assert_caused(seen, pig, shooter, 20.0, None);
}

fn fangs(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let evoker = summon_mob(game, "minecraft:evoker", "{NoAI:1b}");
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    move_mob(game, evoker, Vec3::new(30.5, 70.0, 0.5));
    let at = mob_at(game, pig);
    game.insert_fangs(960, [at.x, at.y, at.z], 0.0, 0, evoker);
    for _ in 0..30 {
        game.tick_fangs();
        if !seen.lock().unwrap().is_empty() {
            break;
        }
    }
    assert_caused(seen, pig, evoker, 24.0, None);
}

fn skull(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let wither = summon_mob(game, "minecraft:wither", "{NoAI:1b}");
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    move_mob(game, wither, Vec3::new(30.5, 70.0, 0.5));
    let at = mob_at(game, pig);
    let p = shot(
        ProjectileKind::WitherSkull { dangerous: false },
        Who::Mob(wither),
    );
    let hit = Hit::Entity {
        who: Who::Mob(pig),
        at,
    };
    game.wither_skull_hit(&p, hit, [at.x, at.y, at.z]).unwrap();
    assert_caused(seen, pig, wither, 300.0, None);
}

fn wind(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let owner = summon_mob(game, "minecraft:zombie", "{NoAI:1b}");
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    move_mob(game, owner, Vec3::new(30.5, 70.0, 0.5));
    let at = mob_at(game, pig);
    let p = shot(ProjectileKind::WindCharge, Who::Mob(owner));
    game.wind_charge_hit(
        &p,
        Hit::Entity {
            who: Who::Mob(pig),
            at,
        },
    )
    .unwrap();
    assert_caused(seen, pig, owner, 20.0, None);
}

fn trident(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    seen: &Mutex<Vec<Seen>>,
    mark: &AttachmentKey<i32>,
) {
    use sunrise_level::chunk::Chunk;
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
        .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command("forceload add 0 0"));
    game.player.as_mut().unwrap().position = Vec3::new(0.5, 64.0, 8.5);
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs.push(sunrise_sim::mobs::Mob::new(
        72,
        kind,
        Vec3::new(3.0, 64.0, 0.5),
        0.0,
    ));
    let mut t =
        sunrise_gameplay::trident::ThrownTrident::remote(71, Vec3::new(0.5, 64.5, 0.5), 0.0, 0.0);
    t.velocity = Vec3::new(4.0, 0.0, 0.0);
    t.owner = 7;
    t.owner_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(7));
    t.left_owner = true;
    game.tridents.push(t);
    mark.set(game, Holder::Player(7), 9).unwrap();
    game.tick_tridents();
    assert_caused(seen, 72, 7, 20.0, Some(9));
}

/// A guest's blast going off while the guest is parked: its record is the causer, as the
/// `LivingRef` reads answer for it then, with its own attachment map.
fn guest(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    seen: &Mutex<Vec<Seen>>,
    mark: &AttachmentKey<i32>,
) {
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    assert!(game.server.lan_acting != Some(12));
    mark.set(game, Holder::Player(12), 5).unwrap();
    let living = game.living(game.entity(12).unwrap()).unwrap();
    assert_eq!(living.health(game), Some(20.0));
    blast(game, [1.5, 70.0, 0.5], 12, true, Some(12));
    assert_caused(seen, pig, 12, 20.0, Some(5));
}

/// A parked guest's blast hurting the player in place (another guest, in its turn): the causer
/// is the parked guest's record, read with its own map, not the player in place's.
fn guest_player(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    seen: &Mutex<Vec<Seen>>,
    mark: &AttachmentKey<i32>,
) {
    mark.set(game, Holder::Player(12), 5).unwrap();
    game.as_guest(13, |g| {
        g.set_game_mode(GameMode::Creative);
        g.set_game_mode(GameMode::Survival);
        mark.set(g, Holder::Player(13), 9).unwrap();
        let at = g.player.as_ref().unwrap().position;
        blast(g, [at.x, at.y, at.z], 12, true, Some(12));
    })
    .unwrap();
    assert_caused(seen, 13, 12, 20.0, Some(5));
}

/// A parked guest's blast killing a pig: the Post waits for the death, and lends the guest
/// again from its record.
fn guest_post(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    seen: &Mutex<Vec<Seen>>,
    mark: &AttachmentKey<i32>,
) {
    let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
    game.mobs.iter_mut().find(|m| m.id == pig).unwrap().health = 0.5;
    mark.set(game, Holder::Player(12), 5).unwrap();
    blast(game, [1.5, 70.0, 0.5], 12, true, Some(12));
    let posts = calls(seen, "post", pig);
    assert_eq!(posts.len(), 1, "{posts:?}");
    let expected = Causer {
        id: 12,
        health: Some(20.0),
        mark: Some(5),
    };
    assert_eq!(posts[0].source_causing, 12);
    assert_eq!(posts[0].causing, Some(expected));
}

/// The player's own blast hurting the player: the source names the victim, which the listener
/// reads through `victim()`; `causing()` lends nothing more.
fn self_caused(game: &mut sunrise_gameplay::game::ServerGame<'_>, seen: &Mutex<Vec<Seen>>) {
    let at = game.player.as_ref().unwrap().position;
    blast(game, [at.x, at.y, at.z], 7, true, None);
    for hit in calls(seen, "incoming", 7) {
        assert_eq!(hit.source_causing, 7, "{hit:?}");
        assert_eq!(hit.causing, None, "{hit:?}");
        assert_eq!(hit.victim, Some(7), "{hit:?}");
    }
}

#[test]
#[ignore = "daily"]
fn a_hit_listener_reads_the_causer_at_each_damage_site() {
    const CASE: &str = "SUNRISE_TEST_HIT_CAUSERS";
    if run_fresh_cases(
        "cut_layout::game::server_side::native_hit_causer_tests::a_hit_listener_reads_the_causer_at_each_damage_site",
        CASE,
        &[
            "projectile",
            "explosion",
            "thorns",
            "sting",
            "firework",
            "fangs",
            "skull",
            "trident",
            "wind",
            "guest",
            "guest_player",
            "guest_post",
            "self",
        ],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    match case.as_str() {
        "guest" => with_guest_probe(guest),
        "guest_player" => with_guest_probe(guest_player),
        "guest_post" => with_guest_probe(guest_post),
        _ => with_probe(|game, seen, mark| match case.as_str() {
            "projectile" => projectile(game, seen),
            "explosion" => explosion(game, seen),
            "thorns" => thorns(game, seen, mark),
            "sting" => sting(game, seen),
            "firework" => firework(game, seen),
            "fangs" => fangs(game, seen),
            "skull" => skull(game, seen),
            "trident" => trident(game, seen, mark),
            "wind" => wind(game, seen),
            "self" => self_caused(game, seen),
            other => panic!("unknown case {other}"),
        }),
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
