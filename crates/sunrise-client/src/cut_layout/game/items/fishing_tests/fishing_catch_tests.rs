use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::fishing_common::Hook;
use sunrise_gameplay_common::game::fishing_common::HookLerp;
use sunrise_gameplay_common::game::fishing_common::State;
use sunrise_items::items::ItemStack;
// Included as fishing's child so fixtures can construct its private hook state.
use std::sync::Arc;
use sunrise_sim::motion::PlayerMotion;

struct Catch(u8);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Catch {
    fn item_fished(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemFishedEvent<'_, '_>,
    ) -> mod_api::Flow {
        match self.0 {
            1 => {
                event.drops.clear();
                event
                    .drops
                    .push(
                        mod_api::ItemStack::new(
                            host.item_key(
                                &mod_api::Identifier::parse("minecraft:diamond").unwrap(),
                            )
                            .unwrap(),
                            2,
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            2 => {
                event.rod_damage = 5;
                return mod_api::Flow::Cancel;
            }
            3 => event.drops.clear(),
            _ => {}
        }
        mod_api::Flow::Continue
    }
}
struct InvalidCatch;
#[mod_api::__subscriptions]
impl mod_api::Hooks for InvalidCatch {
    #[listener(receive_cancelled)]
    fn item_fished(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ItemFishedEvent<'_, '_>,
    ) -> mod_api::Flow {
        assert!(event.cancelled);
        assert_eq!(event.rod_damage, 5);
        let key = host
            .item_key(&mod_api::Identifier::parse("minecraft:diamond").unwrap())
            .unwrap();
        let mut stack = mod_api::ItemStack::new(key, 1).unwrap();
        stack.set_count(65).unwrap();
        assert_eq!(
            event.drops.push(stack),
            Err(mod_api::StackWriteError::CountExceeded { maximum: 64 })
        );
        mod_api::Flow::Continue
    }
}

fn run(mode: Option<u8>) -> (Vec<(String, u32)>, usize, i32, u32, i64, bool) {
    let mut result = None;
    mod_platform::testing::with_mods(Default::default(), || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().into();
            game.server.save.0 = Arc::new(sunrise_sim::save::SaveDir::new(
                scratch.path().join("save"),
                sunrise_sim::save::LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.server.lan_acting_uuid = Some(7);
            game.server.next_entity = 1000;
            game.level.seed = 12345;
            *game.world.random = LegacyRandom::new(12345);
            let advancement = scratch.path().join("test/advancement/catch.json");
            std::fs::create_dir_all(advancement.parent().unwrap()).unwrap();
            std::fs::write(
                &advancement,
                r#"{"criteria":{"catch":{"trigger":"minecraft:fishing_rod_hooked"}}}"#,
            )
            .unwrap();
            game.adv.tree = Arc::new(sunrise_sim::advancements::Tree::load(scratch.path()));
            game.adv.loaded = true;
            let loot = sunrise_world::loot::LootData::load(
                &game.options.resources,
                &Default::default(),
                game.res.item_defaults(),
            )
            .unwrap();
            game.server.server_loot.0.set(Some(Arc::new(loot)));
            let mut hook = Hook {
                uuid: [0; 4],
                attachment_lifetime: Default::default(),
                join: sunrise_sim::join_seq::JoinSeq::UNJOINED,
                caster: None,
                caster_generation: sunrise_sim::join_seq::JoinSeq::UNJOINED,
                id: 999,
                position: Vec3::new(2.0, 64.0, 2.0),
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                state: State::Bobbing,
                hooked_in: None,
                left_owner: false,
                yaw: 0.0,
                pitch: 0.0,
                previous_yaw: 0.0,
                previous_pitch: 0.0,
                last_deflected_by: None,
                needs_sync: false,
                tick_count: 0,
                life: 0,
                ground: Default::default(),
                nibble: 1,
                time_until_lured: 0,
                time_until_hooked: 0,
                fish_angle: 0.0,
                out_of_water: 0,
                open_water: true,
                luck: 0,
                lure: 0,
                medium: None,
                lerp: HookLerp::default(),
                random: LegacyRandom::new(42),
                base: Default::default(),
            };
            let mods = mode
                .map(|m| {
                    let mut mods = vec![mod_api::ModEntry::new(
                        "catch",
                        Box::new(Catch(if m == 4 { 2 } else { m })),
                    )];
                    if m == 4 {
                        mods.push(mod_api::ModEntry::new(
                            "invalid_catch",
                            Box::new(InvalidCatch),
                        ));
                    }
                    mods
                })
                .unwrap_or_default()
                .into();
            let before_items = game.dropped.len();
            let before_orbs = game.orbs.len();
            let stat = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.stats
                    .values
                    .get("minecraft:custom")
                    .and_then(|v| v.get("minecraft:fish_caught"))
                    .copied()
                    .unwrap_or(0)
            };
            let before_stat = stat(game);
            let damage = mod_platform::testing::with_mods(mods, || {
                game.retrieve_hook_extracted(&ItemStack::new("minecraft:fishing_rod", 1), &mut hook)
            });
            result = Some((
                game.dropped
                    .iter()
                    .skip(before_items)
                    .map(|item| (item.stack.item.to_string(), item.stack.count))
                    .collect(),
                game.orbs.len() - before_orbs,
                stat(game) - before_stat,
                damage,
                hook.random.next_long(),
                game.adv.progress.has("test:catch", "catch"),
            ));
        });
    });
    result.unwrap()
}
#[test]
#[ignore = "daily"]
fn fishing_catch_replaces_rolled_stack() {
    let (drops, orbs, stat, damage, _, triggered) = run(Some(1));
    assert_eq!(drops, vec![("minecraft:diamond".into(), 2)]);
    assert_eq!((orbs, stat, damage), (1, 0, 1));
    assert!(triggered);
}
#[test]
#[ignore = "daily"]
fn fishing_catch_cancel_keeps_damage() {
    let (drops, orbs, stat, damage, _, triggered) = run(Some(2));
    assert!(drops.is_empty());
    assert_eq!((orbs, stat, damage), (0, 0, 5));
    assert!(!triggered);
}
#[test]
#[ignore = "daily"]
fn fishing_catch_empty_keeps_damage() {
    let (drops, orbs, stat, damage, _, triggered) = run(Some(3));
    assert!(drops.is_empty());
    assert_eq!((orbs, stat, damage), (0, 0, 1));
    assert!(triggered);
}
#[test]
#[ignore = "daily"]
fn fishing_catch_noop_listener_preserves_randomness() {
    let absent = run(None);
    assert!(!absent.0.is_empty());
    assert_eq!(absent, run(Some(0)));
}

struct Capture(std::sync::Mutex<Vec<String>>);
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.level() == log::Level::Warn {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}
static LOG: Capture = Capture(std::sync::Mutex::new(Vec::new()));

#[test]
#[ignore = "daily"]
fn fishing_catch_invalid_drops_keep_the_earlier_listeners_outputs() {
    // Isolate the process-global logger, as the structure-transform tests do.
    if std::env::var_os("SUNRISE_FISHING_REFUSAL_LOG_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cut_layout::game::server_side::fishing::fishing_catch_tests::fishing_catch_invalid_drops_keep_the_earlier_listeners_outputs", "--include-ignored", "--nocapture"])
            .env("SUNRISE_FISHING_REFUSAL_LOG_CHILD", "1")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    // A refused append doesn't alter the catch or clear an earlier listener's cancellation.
    // The catch ends as that cancelling listener alone leaves it.
    let cancelled = run(Some(2));
    LOG.0.lock().unwrap().clear();
    let refused = run(Some(4));
    assert_eq!(refused, cancelled);
    assert!(refused.0.is_empty());
    assert_eq!(refused.3, 5);
    assert!(!refused.5);
    let warnings: Vec<_> = LOG
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("refused"))
        .cloned()
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].ends_with(
            "mod invalid_catch hook item_fished output drops refused: CountExceeded { maximum: 64 }"
        ),
        "{warnings:?}"
    );
}
