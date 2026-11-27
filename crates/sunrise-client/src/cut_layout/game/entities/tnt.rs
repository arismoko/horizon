#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Who;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2;
    use sunrise_gameplay_common::game::creatures::ModGameExt1;
    use sunrise_gameplay_common::game::server_side::tnt::tests::on_big_stack;

    use sunrise_gameplay_common::game::server_side::tnt::tests::write_silence_output;

    use sunrise_gameplay::game::server_side::tnt::tests::particle_count;

    use sunrise_gameplay::game::server_side::tnt::tests::set_silent;

    use sunrise_gameplay::game::server_side::tnt::tests::sound_count;

    use super::*;

    /// A server's game on the prepared resources with the chunk at the origin
    /// loaded, as `native_machines`' tests make one; None without them.
    pub(crate) fn server_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> Option<Box<sunrise_gameplay::game::ServerGame<'a>>> {
        use {crate::game::*, sunrise_gameplay_common::game::*};
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return None;
        }
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let res = resources.insert(loaded.resources);
        // Keep nested fixture helpers from copying the large Game value
        // through several test-thread stack frames.
        let mut game = Box::new(
            sunrise_gameplay::game::ServerGame::new(
                res,
                loaded.pack,
                options,
                settings.sim_settings(),
                None,
                crate::game::profile_resolver,
            )
            .unwrap(),
        );
        game.options.dedicated = Some(Vec::new());
        // Entity ticking waits for every chunk within 2 columns
        // (`ChunkMap.prepareEntityTickingChunk`).
        for z in -2..=2 {
            for x in -2..=2 {
                game.world.chunks.insert(
                    (x, z),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(x, z)),
                );
            }
        }
        game.server.streaming.forced.insert((0, 0));
        Some(game)
    }
    #[test]
    #[ignore = "daily"]
    fn portal_transfer_tnt_state_follows_memory_but_not_saved_nbt() {
        run_portal_transfer_tnt_state_follows_memory_but_not_saved_nbt();
    }
    pub(crate) fn run_portal_transfer_tnt_state_follows_memory_but_not_saved_nbt() {
        use sunrise_gameplay_common::game::commands_common::Target;
        let mut resources = None;
        let mut game = server_game(&mut resources).expect("set SUNRISE_TEST_RESOURCES");
        game.add_primed_tnt(BlockPos::new(8, 64, 8), 0.0, 80, None, None);
        assert!(!game.tnts[0].used_portal);
        let id = game.tnts[0].id;
        game.teleport_to_in_level(Target::Tnt(id), [8.5, 64.0, 8.5], None, [false; 3], false);
        assert!(game.tnts[0].used_portal);
        let saved = game.tnt_nbt(0);
        game.tnts.clear();
        game.load_tnt(saved, None);
        assert!(!game.tnts[0].used_portal);
        let mut transfer = game.tnt_nbt(0);
        sunrise_gameplay::game::server_side::persist::mark_fresh(&mut transfer);
        game.tnts.clear();
        game.world.loaded_entities.push((
            (0, 0),
            vec![sunrise_level::world::PendingEntity::after_tnt_portal(
                transfer,
            )],
        ));
        game.spawn_loaded_entities();
        assert!(game.tnts[0].used_portal);
        assert!(game.tnt_nbt(0).get("sunrise:tnt_used_portal").is_none());
    }
    #[test]
    #[ignore = "daily"]
    fn portal_transfer_ordinary_nbt_cannot_supply_runtime_immunity() {
        run_portal_transfer_ordinary_nbt_cannot_supply_runtime_immunity();
    }
    pub(crate) fn run_portal_transfer_ordinary_nbt_cannot_supply_runtime_immunity() {
        use sunrise_core::nbt::Tag;
        let mut resources = None;
        let mut game = server_game(&mut resources).expect("set SUNRISE_TEST_RESOURCES");
        for marker in [0, 1] {
            game.add_primed_tnt(BlockPos::new(8, 64, 8), 0.0, 80, None, None);
            let mut saved = game.tnt_nbt(0);
            if let Tag::Compound(fields) = &mut saved {
                fields.insert("sunrise:tnt_used_portal".into(), Tag::Byte(marker));
            }
            game.tnts.clear();
            game.world.queue_entity_nbt((0, 0), vec![saved]);
            game.spawn_loaded_entities();
            assert_eq!(game.tnts.len(), 1);
            assert!(!game.tnts[0].used_portal, "ordinary NBT marker {marker}");
            game.tnts.clear();
        }
    }
    #[test]
    #[ignore = "daily"]
    fn portal_blast_sulfur_cube_uses_only_positive_cooldown() {
        run_portal_blast_sulfur_cube_uses_only_positive_cooldown();
    }
    pub(crate) fn source_silence_game(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> Option<Box<sunrise_gameplay::game::ServerGame<'_>>> {
        let mut game = server_game(resources)?;
        // Observe the server's real sound, level-event, and explosion broadcasts without a client.
        game.start_audio_capture();
        game.server.lan_acting = Some(0);
        Some(game)
    }
    pub(crate) fn run_portal_blast_sulfur_cube_uses_only_positive_cooldown() {
        use sunrise_sim::mobs::Mob;
        use sunrise_sim::nether_portal::PortalState;
        let mut resources = None;
        let mut game = server_game(&mut resources).expect("set SUNRISE_TEST_RESOURCES");
        game.start_audio_capture();
        let portal = game
            .pack
            .assets
            .registry
            .default_state("minecraft:nether_portal")
            .unwrap();
        for (cooldown, silent) in [
            (1, false),
            (1, true),
            (0, false),
            (0, true),
            (-1, false),
            (-1, true),
        ] {
            game.world.set(8, 64, 8, portal).unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let mut cube = Mob::new(9123, kind, Vec3::new(8.5, 64.0, 8.5), 0.0);
            cube.silent = silent;
            cube.extra.set_body(Some("minecraft:tnt".into()));
            // Fuse 1: explodes on this tick.
            cube.extra.sulfur_fuse = sunrise_sim::mobs::sulfur_cube::Fuse::set(1);
            game.mobs.push(cube);
            game.server.portals.entities.insert(
                9123,
                PortalState {
                    cooldown,
                    process: None,
                },
            );
            // The actual level tick orders the portal pass before the mob pass.
            game.server.pending_sounds.clear();
            game.handle_entity_portals();
            game.tick_creatures().unwrap();
            assert_eq!(
                game.world.block(8, 64, 8) == portal,
                cooldown > 0,
                "cooldown {cooldown}, Silent={silent}"
            );
            assert_eq!(
                sound_count(&game, "minecraft:entity.generic.explode"),
                u32::from(!silent),
                "cooldown {cooldown}, Silent={silent}"
            );
        }
        // An unrelated native/sourceless blast never inherits the cube's calculator.
        game.world.set(8, 64, 8, portal).unwrap();
        game.server_explode(
            [8.5, 64.0625, 8.5],
            4.0,
            sunrise_gameplay::game::server_side::explosion::BlockInteraction::Destroy,
            false,
        )
        .unwrap();
        assert_ne!(game.world.block(8, 64, 8), portal);
    }
    #[test]
    #[ignore = "daily"]
    fn loaded_tnt_fuse_uses_numeric_short_and_saves_without_raw() {
        use sunrise_core::nbt::Tag;
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        let cases = [
            (None, 80),
            (Some(Tag::Short(80)), 80),
            (Some(Tag::Short(2)), 2),
            (Some(Tag::Short(-1)), -1),
            (Some(Tag::Byte(-2)), -2),
            (Some(Tag::Int(32768)), -32768),
            (Some(Tag::Long(65535)), -1),
            (Some(Tag::Long(i64::MAX)), -1),
            (Some(Tag::Float((-1.5_f32).to_bits())), -2),
            (Some(Tag::Double((-1.5_f64).to_bits())), -2),
            (Some(Tag::Double(1.5_f64.to_bits())), 1),
            (Some(Tag::Double(32768.75_f64.to_bits())), -32768),
            (Some(Tag::String("wrong_type".into())), 80),
        ];
        for (value, expected) in cases {
            let mut tag = Tag::compound([
                (
                    "Pos",
                    Tag::List(vec![
                        Tag::Double(4.5_f64.to_bits()),
                        Tag::Double(200_f64.to_bits()),
                        Tag::Double(4.5_f64.to_bits()),
                    ]),
                ),
                (
                    "Motion",
                    Tag::List(vec![
                        Tag::Double(0.125_f64.to_bits()),
                        Tag::Double(0),
                        Tag::Double(0),
                    ]),
                ),
                ("NoGravity", Tag::Byte(1)),
                ("block_state", Tag::String("minecraft:stone".into())),
                ("explosion_power", Tag::Float(2.5_f32.to_bits())),
                ("owner", Tag::IntArray(vec![5, 6, 7, 8])),
                ("CustomName", Tag::String("Fuse control".into())),
                ("Tags", Tag::List(vec![Tag::String("h090".into())])),
            ]);
            if let (Some(value), Tag::Compound(fields)) = (value, &mut tag) {
                fields.insert("fuse".into(), value);
            }
            let i = game.tnts.len();
            game.load_tnt(tag.clone(), Some([90, 0, 0, i as i32 + 1]));
            let t = &game.tnts[i];
            assert_eq!(t.fuse, expected, "{tag:?}");
            assert_eq!(t.velocity, Vec3::new(0.125, 0.0, 0.0));
            assert!(t.no_gravity);
            assert_eq!(t.power, 2.5);
            let state = t.block_state_nbt(game.pack.assets.registry);
            assert_eq!(state, Tag::String("minecraft:stone".into()));
            let saved = sunrise_sim::entity_nbt::tnt_to_nbt(
                t.position,
                t.velocity,
                t.ground.on_ground,
                t.fall_distance,
                t.fuse,
                t.power,
                state,
                t.rotation,
                [90, 0, 0, i as i32 + 1],
                None,
                None,
            );
            assert_eq!(saved.get("fuse"), Some(&Tag::Short(expected as i16)));
            let saved = game.tnt_nbt(i);
            for field in [
                "owner",
                "CustomName",
                "Tags",
                "Motion",
                "block_state",
                "explosion_power",
            ] {
                assert_eq!(saved.get(field), tag.get(field), "{field}");
            }
        }
    }
    #[test]
    #[ignore = "daily"]
    fn live_data_commands_adopt_into_the_existing_tnt() {
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay_common::game::commands_common::Target;
        let mut resources = None;
        let mut game = server_game(&mut resources).expect("prepared Game resources are required");
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.server.streaming.forced.contains(&(0, 0)));
        // The existing unit helper supplies a resident column. The paired
        // runtime harness separately admits actual ticket/streaming readiness.
        assert!(game.world.chunks.contains_key(&(0, 0)));
        assert!(game.run_command(
            "summon pig 8.5 200 5.5 {UUID:[I;89,0,0,3],Health:10f,NoAI:1b,NoGravity:1b}"
        ));
        assert!(game.run_command("summon tnt 5.5 200 5.5 {UUID:[I;89,0,0,1],fuse:40s,Motion:[0d,0d,0d],NoGravity:1b,explosion_power:2f,owner:[I;89,0,0,3],Tags:[keep],Silent:1b}"));
        let pig = game
            .mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([89, 0, 0, 3]))
            .unwrap()
            .id;
        assert_eq!(
            game.mobs.iter().find(|m| m.id == pig).unwrap().kind.id,
            "minecraft:pig"
        );
        assert_eq!(game.tnts.len(), 1);
        let id = game.tnts[0].id;
        assert_eq!(game.persist.uuid_of(id), Some([89, 0, 0, 1]));
        assert_eq!(game.tnts[0].fuse, 40);
        assert_eq!(game.tnts[0].velocity, Vec3::ZERO);
        assert!(game.tnts[0].no_gravity);
        // Preserve a live owner referencing an actually registered mob.
        game.tnts[0].owner = Some(Who::Mob(pig));
        let before = game.tnts[0].clone();
        let next_id = game.server.next_entity;
        let clock = game.world.game_time;
        assert!(before.join.is_joined());
        let reference = Target::Tnt(id);
        let saved_before = game.command_target_nbt(reference).unwrap();
        assert_eq!(saved_before.get("NoGravity"), Some(&Tag::Byte(1)));
        assert_eq!(
            saved_before.get("owner"),
            Some(&Tag::IntArray(vec![89, 0, 0, 3]))
        );
        assert!(
            !game.run_command("data merge entity 00000059-0000-0000-0000-000000000001 {fuse:40s}")
        );
        assert!(game.run_command("data merge entity 00000059-0000-0000-0000-000000000001 {fuse:60s,Motion:[0.125d,0d,-0.25d],Tags:[adopted]}"), "valid TNT data must reach the existing live instance");
        assert_eq!(
            game.server.next_entity, next_id,
            "adoption must not spawn another entity"
        );
        assert_eq!(game.tnts.len(), 1);
        let live = game.tnts.iter().find(|t| t.id == id).unwrap();
        assert_eq!(live.fuse, 60, "the actual modeled counter changes");
        assert_eq!(live.velocity, Vec3::new(0.125, 0.0, -0.25));
        assert_eq!(live.position, before.position);
        assert_eq!(live.previous, live.position);
        assert_eq!(live.rotation, before.rotation);
        assert_eq!(live.join, before.join);
        assert_eq!(live.owner, before.owner);
        assert_eq!(live.guest, before.guest);
        assert_eq!(live.power, before.power);
        assert_eq!(live.no_gravity, before.no_gravity);
        assert_eq!(game.world.game_time, clock);
        assert_eq!(game.persist.uuid_of(id), Some([89, 0, 0, 1]));
        let saved = game.command_target_nbt(reference).unwrap();
        assert_eq!(saved.get("fuse"), Some(&Tag::Short(60)));
        assert_eq!(
            saved.get("Tags"),
            Some(&Tag::List(vec![Tag::String("adopted".into())]))
        );
        for field in [
            "UUID",
            "Pos",
            "NoGravity",
            "Rotation",
            "owner",
            "block_state",
            "explosion_power",
            "Silent",
        ] {
            assert!(
                saved_before.get(field).is_some(),
                "positive saved {field} control"
            );
            assert_eq!(
                saved.get(field),
                saved_before.get(field),
                "retained {field}"
            );
        }
    }
    #[test]
    #[ignore = "daily"]
    fn silent_tnt_keeps_the_blast_without_its_sound() {
        run_silent_tnt_keeps_the_blast_without_its_sound();
    }
    pub(crate) fn run_silent_tnt_keeps_the_blast_without_its_sound() {
        on_big_stack(|| {
            let mut resources = None;
            let Some(mut game) = source_silence_game(&mut resources) else {
                return;
            };
            let mut rows = Vec::new();
            for silent in [false, true, false] {
                let glass = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:glass")
                    .unwrap();
                game.world.set(8, 64, 8, glass).unwrap();
                game.server.pending_sounds.clear();
                game.add_primed_tnt(BlockPos::new(8, 64, 8), 0.0, 1, None, None);
                game.tnts[0].no_gravity = true;
                game.tnts[0].velocity = Vec3::ZERO;
                let id = game.tnts[0].id;
                set_silent(&mut game, id, silent);
                let before = particle_count(&game);
                game.tick_one_tnt(0, &[]).unwrap();
                rows.push(format!(
                    "explosion {silent} {} {}",
                    particle_count(&game) > before,
                    sound_count(&game, "minecraft:entity.generic.explode") != 0
                ));
                assert!(game.tnts.is_empty(), "the fuse still expires");
                assert!(
                    game.res.facts().is_air(game.world.block(8, 64, 8)),
                    "silence keeps block destruction"
                );
                assert!(particle_count(&game) > before, "the blast remains visible");
                assert_eq!(
                    sound_count(&game, "minecraft:entity.generic.explode"),
                    u32::from(!silent),
                    "direct source Silent={silent}"
                );
                game.server.pending_sounds.clear();
                game.server_explode(
                    [8.5, 64.0, 8.5],
                    0.0,
                    sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
                    false,
                )
                .unwrap();
                assert_eq!(
                    sound_count(&game, "minecraft:entity.generic.explode"),
                    1,
                    "a source-free blast stays audible"
                );
            }
            write_silence_output("explosion", &rows);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn silent_potions_keep_the_splash_without_its_sound() {
        run_silent_potions_keep_the_splash_without_its_sound();
    }
    pub(crate) fn run_silent_potions_keep_the_splash_without_its_sound() {
        on_big_stack(|| {
            use sunrise_sim::projectiles::{Hit, Projectile, ProjectileKind};
            let mut resources = None;
            let Some(mut game) = source_silence_game(&mut resources) else {
                return;
            };
            let mut rows = Vec::new();
            for kind in ["splash_potion", "lingering_potion"] {
                for potion in ["awkward", "swiftness", "healing"] {
                    for silent in [false, true] {
                        game.server.pending_sounds.clear();
                        game.server.lan_events.clear();
                        let stack = ItemStack::new(&format!("minecraft:{kind}"), 1)
                            .with_potion(&format!("minecraft:{potion}"));
                        let mut projectile = Projectile::new(
                            ProjectileKind::potion(stack),
                            Vec3::new(8.5, 64.0, 8.5),
                            None,
                        );
                        projectile.id = game.take_entity_id();
                        set_silent(&mut game, projectile.id, silent);
                        let before = particle_count(&game);
                        let clouds = game.breath_clouds.len();
                        game.projectile_hit(
                            &projectile,
                            Hit::Block {
                                pos: [8, 63, 8],
                                at: projectile.position,
                                face: [0, 1, 0],
                            },
                        )
                        .unwrap();
                        rows.push(format!(
                            "{kind} {potion} {silent} {} {}",
                            particle_count(&game) > before,
                            sound_count(&game, "minecraft:entity.splash_potion.break") != 0
                        ));
                        assert!(
                            particle_count(&game) > before,
                            "{kind} {potion} Silent={silent}: visible splash"
                        );
                        assert_eq!(
                            sound_count(&game, "minecraft:entity.splash_potion.break"),
                            u32::from(!silent),
                            "{kind} {potion} Silent={silent}"
                        );
                        assert_eq!(
                            game.breath_clouds.len() - clouds,
                            usize::from(kind == "lingering_potion" && potion != "awkward"),
                            "silence leaves the lingering cloud intact"
                        );
                    }
                }
            }
            write_silence_output("potions", &rows);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn silent_creeper_keeps_the_blast_after_its_source_is_removed() {
        run_silent_creeper_keeps_the_blast_after_its_source_is_removed();
    }
    pub(crate) fn run_silent_creeper_keeps_the_blast_after_its_source_is_removed() {
        on_big_stack(|| {
            let mut resources = None;
            let Some(mut game) = source_silence_game(&mut resources) else {
                return;
            };
            for silent in [false, true, false] {
                let id = game
                    .spawn_mob(
                        "minecraft:creeper",
                        [8.5, 64.0, 8.5],
                        true,
                        sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                let mob = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
                mob.silent = silent;
                mob.swell = mob.max_swell;
                mob.extra.flag = true;
                let mut events = Vec::new();
                sunrise_sim::mobs::hostile::creeper_tick(mob, &mut events);
                assert!(mob.removed);
                game.mobs.retain(|m| m.id != id);
                game.server.pending_sounds.clear();
                let particles = particle_count(&game);
                game.handle_mob_events(events).unwrap();
                assert!(particle_count(&game) > particles);
                assert_eq!(
                    sound_count(&game, "minecraft:entity.generic.explode"),
                    u32::from(!silent)
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn silent_launched_potions_and_bottles_keep_their_impact() {
        run_silent_launched_potions_and_bottles_keep_their_impact();
    }
    pub(crate) fn run_silent_launched_potions_and_bottles_keep_their_impact() {
        on_big_stack(|| {
            let mut resources = None;
            let Some(mut game) = source_silence_game(&mut resources) else {
                return;
            };
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(8, 63, 8, stone).unwrap();
            for kind in ["splash_potion", "lingering_potion", "experience_bottle"] {
                for silent in [false, true, false] {
                    let mut shot =
                        sunrise_gameplay::game::server_side::potions_server::thrown_projectile(
                            ItemStack::new(&format!("minecraft:{kind}"), 1)
                                .with_potion("minecraft:swiftness"),
                            Vec3::new(8.5, 64.1, 8.5),
                            false,
                        );
                    shot.velocity = Vec3::new(0.0, -1.0, 0.0);
                    game.add_projectile(shot);
                    let id = game.creatures.projectiles.last().unwrap().id;
                    set_silent(&mut game, id, silent);
                    game.server.pending_sounds.clear();
                    game.server.lan_events.clear();
                    let particles = particle_count(&game);
                    let clouds = game.breath_clouds.len();
                    let xp: u32 = game.orbs.iter().map(|o| o.points() * o.count).sum();
                    game.tick_creature_entities().unwrap();
                    assert!(!game.creatures.projectiles.iter().any(|p| p.id == id));
                    assert!(particle_count(&game) > particles, "{kind} keeps its splash");
                    assert_eq!(
                        sound_count(&game, "minecraft:entity.splash_potion.break"),
                        u32::from(!silent),
                        "{kind} Silent={silent}"
                    );
                    assert_eq!(
                        game.breath_clouds.len() - clouds,
                        usize::from(kind == "lingering_potion")
                    );
                    let gained = game.orbs.iter().map(|o| o.points() * o.count).sum::<u32>() - xp;
                    if kind == "experience_bottle" {
                        assert!((3..=11).contains(&gained), "silence keeps the experience");
                    }
                }
            }
        });
    }
    /// #13201: a dispensed potion or bottle is a level entity
    /// (`ProjectileDispenseBehavior` → `Projectile.spawnProjectileUsingShoot`
    /// → `level.addFreshEntity`), so selectors, `/data get` and `/kill` find
    /// it while it flies.
    #[test]
    #[ignore = "daily"]
    fn h13201_a_dispensed_potion_is_a_level_entity() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        // An operator's commands.
        game.ui.registries.permission = 4;
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay_common::game::commands_common::Target;
        for kind in ["splash_potion", "lingering_potion", "experience_bottle"] {
            let stack = ItemStack::new(&format!("minecraft:{kind}"), 1);
            // The dispenser at (8, 100, 8) facing south.
            game.shoot_projectile_item(
                Vec3::new(8.5, 100.2, 9.2),
                [0.0, 0.1, 1.0],
                &stack,
                1.1,
                6.0,
            );
            let id = game.creatures.projectiles.last().unwrap().id;
            let selector = format!("@e[type=minecraft:{kind},x=8.5,y=100,z=8.5,distance=..12]");
            // Found as shot and again after it flew a tick.
            for _ in 0..2 {
                let found = game.run_command(&format!("execute if entity {selector}"));
                assert!(found, "{kind}");
                assert_eq!(game.level_entity(id as i32), Some(Target::Projectile(id)));
                let saved = game.command_target_nbt(Target::Projectile(id)).unwrap();
                let id_tag = Tag::String(format!("minecraft:{kind}").as_str().into());
                assert_eq!(saved.get("id"), Some(&id_tag));
                assert!(saved.get("Item").is_some() && saved.get("Motion").is_some());
                game.tick_creature_entities().unwrap();
            }
            assert!(game.run_command(&format!("kill {selector}")), "{kind}");
            assert!(!game.creatures.projectiles.iter().any(|p| p.id == id));
        }
    }
    #[test]
    #[ignore = "daily"]
    fn silent_bottles_preserve_xp_and_the_server_random_stream() {
        run_silent_bottles_preserve_xp_and_the_server_random_stream();
    }
    pub(crate) fn run_silent_bottles_preserve_xp_and_the_server_random_stream() {
        on_big_stack(|| {
            use sunrise_sim::projectiles::{Hit, Projectile, ProjectileKind};
            let mut resources = None;
            let Some(mut game) = source_silence_game(&mut resources) else {
                return;
            };
            let mut rows = Vec::new();
            for seed in [0, 1, -1, 42, i64::MIN, i64::MAX] {
                let mut audible = None;
                for silent in [false, true] {
                    game.world.random.set_seed(seed);
                    game.orbs.clear();
                    let mut projectile = Projectile::new(
                        ProjectileKind::ExperienceBottle,
                        Vec3::new(8.5, 64.0, 8.5),
                        None,
                    );
                    projectile.id = game.take_entity_id();
                    set_silent(&mut game, projectile.id, silent);
                    game.projectile_hit(
                        &projectile,
                        Hit::Block {
                            pos: [8, 63, 8],
                            at: projectile.position,
                            face: [0, 1, 0],
                        },
                    )
                    .unwrap();
                    let xp: u32 = game.orbs.iter().map(|o| o.points() * o.count).sum();
                    let next = game.world.random.next_long();
                    assert!((3..=11).contains(&xp));
                    if let Some((audible_xp, audible_next)) = audible {
                        assert_eq!(
                            (xp, next),
                            (audible_xp, audible_next),
                            "Silent must not change XP or level RNG"
                        );
                        rows.push(format!(
                            "bottle_rng {seed} {} {}",
                            xp == audible_xp,
                            next == audible_next
                        ));
                    } else {
                        audible = Some((xp, next));
                    }
                }
            }
            write_silence_output("bottles", &rows);
        });
    }
    /// #10063: TNT lit between level ticks (flint and steel, a command) has its
    /// first tick in the next level tick, so its fuse reads 79 after it, and it
    /// explodes on the 80th (`PrimedTnt.tick`). Sunrise held every TNT that
    /// joined during a tick back one more: the late pass compared its join
    /// against an `announced` already raised past it.
    #[test]
    #[ignore = "daily"]
    fn lit_tnt_ticks_on_the_next_level_tick_and_explodes_on_the_80th() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        game.add_primed_tnt(BlockPos::new(8, 64, 8), 0.0, 80, None, None);
        // Hang it in the empty chunk, so it neither falls out nor lands.
        game.tnts[0].no_gravity = true;
        game.tnts[0].velocity = Vec3::ZERO;
        game.tick_world(true).unwrap();
        assert_eq!(game.tnts.len(), 1);
        assert_eq!(
            game.tnts[0].fuse, 79,
            "its first tick is the next level tick"
        );
        let mut exploded_on = None;
        for tick in 2..=90 {
            game.tick_world(true).unwrap();
            if game.tnts.is_empty() {
                exploded_on = Some(tick);
                break;
            }
        }
        assert_eq!(exploded_on, Some(80), "explodes on the 80th tick");
    }
    /// TNT a blast primes joins while the entities tick (vanilla's
    /// `EntityTickList` loop) and waits for the next tick. The fix for #10063
    /// must not tick it in the tick it joined.
    #[test]
    #[ignore = "daily"]
    fn tnt_primed_by_a_blast_waits_for_the_next_tick() {
        let mut resources = None;
        let Some(mut game) = server_game(&mut resources) else {
            return;
        };
        let block = game
            .pack
            .assets
            .registry
            .default_state("minecraft:tnt")
            .unwrap();
        game.world.set(9, 64, 8, block).unwrap();
        game.add_primed_tnt(BlockPos::new(8, 64, 8), 0.0, 1, None, None);
        game.tnts[0].no_gravity = true;
        game.tnts[0].velocity = Vec3::ZERO;
        let first = game.tnts[0].id;
        for _ in 0..3 {
            game.tick_world(true).unwrap();
            if game.tnts.iter().all(|t| t.id != first) {
                break;
            }
        }
        assert!(
            game.tnts.iter().all(|t| t.id != first),
            "the first TNT went off"
        );
        assert_eq!(game.tnts.len(), 1, "its blast primed the TNT block");
        let primed = &game.tnts[0];
        assert_eq!(
            primed.position, primed.previous,
            "not ticked in the tick the blast primed it"
        );
        game.tick_world(true).unwrap();
        let primed = &game.tnts[0];
        assert_ne!(primed.position, primed.previous, "ticked on the next tick");
    }
}

#[cfg(test)]
pub(crate) mod entity_collision_tests {
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;

    /// `Entity.collide` for primed TNT, a falling block and an experience orb:
    /// a boat holds the TNT and the orb up, a shulker the sand, as they hold
    /// up a dropped item (`canBeCollidedWith`).
    #[test]
    #[ignore = "daily"]
    fn tnt_sand_and_orbs_land_on_boats_and_shulkers() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.ui.difficulty = 2;
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let sand = registry.default_state("minecraft:sand").unwrap();
            let mut chunk = Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 199, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("summon oak_boat 4.5 200 4.5"));
            assert!(game.run_command("summon oak_boat 12.5 200 12.5"));
            assert!(game.run_command("summon shulker 8.5 200 8.5 {NoAI:1b}"));
            assert!(game.run_command("summon tnt 4.5 202 4.5 {fuse:80}"));
            assert!(game.run_command("summon tnt 4.5 202 12.5 {fuse:80}"));
            assert!(game.run_command(
                "summon falling_block 8.5 203 8.5 {BlockState:{Name:\"minecraft:sand\"},Time:1}"
            ));
            assert!(game.run_command("summon experience_orb 12.5 202 12.5 {Value:1}"));
            for _ in 0..40 {
                game.tick_world(true).unwrap();
            }
            let boat_top = game.boats[0].bounds().max_position().y;
            let tnt_at = |z: f64| {
                game.tnts
                    .iter()
                    .find(|t| (t.position.z - z).abs() < 0.5)
                    .map(|t| t.position.y)
                    .unwrap()
            };
            assert_eq!(tnt_at(4.5), boat_top, "the TNT rests on the boat");
            assert_eq!(tnt_at(12.5), 200.0, "the other TNT lands on the stone");
            assert_eq!(
                game.orbs[0].position.y, boat_top,
                "the orb rests on the boat"
            );
            // FallingBlockEntity.tick: it stops on the shulker, but the air
            // under it would let it fall on (`wouldContinueFalling`), so it breaks
            // into an item there instead of passing through to the stone.
            assert_ne!(
                game.world.block(8, 200, 8),
                sand,
                "the sand does not fall through"
            );
            assert_ne!(
                game.world.block(8, 201, 8),
                sand,
                "nor is it placed over air"
            );
            assert!(
                game.dropped
                    .iter()
                    .any(|i| &*i.stack.item == "minecraft:sand"),
                "the sand breaks into an item"
            );
        });
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
