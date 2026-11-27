//! The Rust side of tools/test_dispenser_parity.py: with
//! `SUNRISE_DISPENSE_FIXTURE=CASES,OUTPUT` the game loads its resources, runs
//! every case (a dispenser or dropper with items, blocks and target entities
//! around it) through `dispense_from` on a fresh headless world and prints what
//! changed in the format of `VanillaDispenserOracle`.
use super::dispenser::{Slot, body_equipment_species, mob_set_slot, mob_slot_item};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, fmt::Write as _, io::Write as _, sync::Arc};
use sunrise_command::args::GameMode;
use sunrise_core::{position::BlockPos, random::RandomSource};
use sunrise_game_data::{Result, gamerules::Value as RuleValue};
use sunrise_items::{
    item_data::{canon_stack, parse_stack},
    items::ItemStack,
};
use sunrise_level::{chunk::Chunk, containers::BlockEntity};
use sunrise_sim::{
    mobs::{
        Mob, Species,
        spawner::{SpawnDataExt, SpawnerExt},
    },
    motion::PlayerMotion,
    projectiles::ProjectileKind,
};
use sunrise_world::{level::Ticks, registry::BlockStateId};

thread_local! {
    /// Level and game events recorded while a fixture case runs.
    pub(in crate::game) static LOG: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    /// The running case is exact (`"exact": true`): `num` prints raw bits.
    pub(in crate::game) static EXACT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Record a line if a fixture case is running.
pub(in crate::game) fn record(line: impl FnOnce() -> String) {
    LOG.with(|log| {
        if let Some(lines) = log.borrow_mut().as_mut() {
            lines.push(line());
        }
    });
}
/// Start recording level and game event lines (another fixture's case).
pub fn start_recording() {
    LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
}
/// The lines recorded since `start_recording`; recording stops.
pub fn stop_recording() -> Vec<String> {
    LOG.with(|log| log.borrow_mut().take()).unwrap_or_default()
}
/// `Level.levelEvent` lines for the level events the behaviour library reported.
pub(in crate::game::server_side) fn record_events(events: &[sunrise_world::level::LevelEvent]) {
    for event in events {
        if let sunrise_world::level::LevelEvent::LevelEvent { id, pos, data, .. } = event {
            record(|| format!("L {id} {},{},{} {data}", pos.x, pos.y, pos.z));
        }
    }
}

/// The `S` line for a level event whose sound `LevelEventHandler` plays on the
/// client (`playLocalSound`), so Sunrise plays it unshared and never captures
/// it: the lava fizz (1501, #7652) and `ComposterBlock.handleFill` (1500,
/// `data > 0` = success, #9069).
pub(in crate::game::server_side) fn event_sound(line: &str) -> Option<String> {
    let mut parts = line.strip_prefix("L ")?.split(' ');
    let (id, pos, data) = (parts.next()?, parts.next()?, parts.next()?);
    let sound = match id {
        "1501" => "minecraft:block.lava.extinguish",
        "1500" if data.parse::<i32>().ok()? > 0 => "minecraft:block.composter.fill_success",
        "1500" => "minecraft:block.composter.fill",
        _ => return None,
    };
    Some(format!("S {pos} {sound}"))
}

/// Java's `String.format("%.6f")`: the exact value rounded half up (away
/// from zero) where Rust rounds a tie to even. An exact case prints the raw
/// bits instead, as the oracle's `num` does then.
pub(in crate::game::server_side) fn num(v: f64) -> String {
    if EXACT.get() {
        return format!("{:x}", v.to_bits());
    }
    let exact = format!("{:.40}", v.abs());
    let tie = exact
        .split_once('.')
        .is_some_and(|(_, f)| f[6..].starts_with('5') && f[7..].bytes().all(|b| b == b'0'));
    if tie {
        format!("{:.6}", v + 0.5e-6_f64.copysign(v))
    } else {
        format!("{v:.6}")
    }
}
pub(in crate::game) fn floor_pos(p: [f64; 3]) -> String {
    format!(
        "{},{},{}",
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32
    )
}

/// The targets a case put in front of the dispenser.
pub(in crate::game) enum Target {
    Mob(u32),
    Stand(u32),
    /// A chest boat or raft (`ContainerEntity`).
    Boat(u32),
    /// A chest or hopper minecart (`ContainerEntity`).
    Cart(u32),
}

impl crate::game::ServerGame<'_> {
    /// Run every case of `spec` ("CASES,OUTPUT") and write the results.
    pub fn dispense_fixture(&mut self, spec: &str) -> Result<()> {
        assert!(
            !self.options.execution_role.is_client(),
            "fixture requires a server Game"
        );
        let (input, output) = spec
            .split_once(',')
            .ok_or("SUNRISE_DISPENSE_FIXTURE=CASES,OUTPUT")?;
        // Saplings and feature placers grow from the live feature registry.
        self.install_live_features();
        let text = std::fs::read_to_string(input)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
        for (index, line) in text.lines().enumerate() {
            let case: Value = serde_json::from_str(line)?;
            writeln!(out, "case {index}")?;
            match self.dispense_case(&case) {
                Ok(lines) => {
                    for l in lines {
                        writeln!(out, "{l}")?;
                    }
                }
                Err(e) => writeln!(out, "failed {e}")?,
            }
        }
        LOG.with(|log| *log.borrow_mut() = None);
        Ok(())
    }

    pub(in crate::game::server_side) fn fixture_reset(&mut self, seed: i64) {
        self.common.world =
            sunrise_gameplay_common::game::world_for_mods(self.common.res.facts().air, seed);
        // The oracle level is plains everywhere (bone meal's flowers, freezing).
        let plains = u32::from(world_biomes!(self).id("minecraft:plains"));
        for cx in -2..=2 {
            for cz in -2..=2 {
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    Arc::make_mut(section).biomes = sunrise_core::paletted::PalettedContainer::new(
                        sunrise_level::chunk::BIOME_STRATEGY,
                        plains,
                    );
                }
                self.common.world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
        self.common.block_ticks = Ticks::new(0);
        self.server.tick_chunks.clear();
        self.sync_tick_chunks();
        self.common.mobs.clear();
        self.common.dragon.discard();
        self.common.dropped.clear();
        self.common.creatures.projectiles.clear();
        self.common.tnts.clear();
        self.common.boats.clear();
        self.common.carts.clear();
        self.common.armor_stands.clear();
        self.common.crystals.clear();
        self.common.paintings.clear();
        self.common.item_frames.clear();
        self.common.rockets.clear();
        self.common.potions.clear_thrown();
        self.common.level_events.clear();
        self.common.orbs.clear();
        self.common.falling.clear();
        // Hopper cooldowns of an earlier case.
        self.server.hoppers.clear();
        self.common.ui.rules = Default::default();
        self.common.dim = super::Dim::Overworld;
        self.server.respawn_config = None;
        // The oracle level plays on normal difficulty.
        self.common.ui.difficulty = 2;
        // The oracle level has world seed 0 and fresh random sequences.
        self.common.level.seed = 0;
        self.server.rand_sequences = Default::default();
    }

    pub(in crate::game::server_side) fn dispense_case(
        &mut self,
        case: &Value,
    ) -> Result<Vec<String>> {
        let seed = case["seed"].as_i64().unwrap_or(0);
        EXACT.set(case["exact"].as_bool().unwrap_or(false));
        self.fixture_reset(seed);
        if let Some(op) = case.get("blocking_wear") {
            return self.fixture_blocking_wear(op);
        }
        // The oracle level answers one sky and one block light everywhere.
        let light = case["light"].as_array();
        let level = |i: usize, default: i64| {
            light
                .and_then(|l| l.get(i))
                .and_then(Value::as_i64)
                .unwrap_or(default) as u8
        };
        let packed = (level(0, 15) << 4) | level(1, 0);
        for chunk in self.common.world.chunks.values_mut() {
            for section in &mut Arc::make_mut(chunk).sections {
                Arc::make_mut(section).light = sunrise_level::chunk::Light::Uniform(packed);
            }
        }
        self.common.world.game_time = case["time"].as_i64().unwrap_or(0);
        match case["dim"].as_str() {
            Some("nether") => self.common.dim = super::Dim::Nether,
            Some("end") => self.common.dim = super::Dim::End,
            _ => {}
        }
        // The level's build range follows its dimension, as `enter_dimension` sets it.
        self.common.world.shape = self.common.dim.shape();
        // The oracle level's clock stands at day time 6000.
        self.common.world.day_time = 6000;
        if let Some(rules) = case["rules"].as_object() {
            for (name, value) in rules {
                if let Some(b) = value.as_bool() {
                    self.common.ui.rules.set(name, RuleValue::Bool(b));
                }
            }
        }
        let mut before: HashMap<(i32, i32, i32), u32> = HashMap::new();
        for cell in case["cells"].as_array().into_iter().flatten() {
            let p = (
                cell[0].as_i64().unwrap_or(0) as i32,
                cell[1].as_i64().unwrap_or(0) as i32,
                cell[2].as_i64().unwrap_or(0) as i32,
            );
            let state = BlockStateId(cell[3].as_u64().unwrap_or(0) as u32);
            self.common.world.set(p.0, p.1, p.2, state)?;
            before.insert(p, state.0);
            if let Some(entity) = BlockEntity::for_block(&self.common.res.facts().get(state).name) {
                self.common.world.block_entities.insert(p, entity);
            }
        }
        for item in case["items"].as_array().into_iter().flatten() {
            let p = (
                item[0].as_i64().unwrap_or(0) as i32,
                item[1].as_i64().unwrap_or(0) as i32,
                item[2].as_i64().unwrap_or(0) as i32,
            );
            let slot = item[3].as_u64().unwrap_or(0) as usize;
            let stack = parse_stack(item[4].as_str().unwrap_or("-"))?;
            self.fixture_fill(p, slot, stack);
        }
        // CrafterBlock.newBlockEntity copies the state's triggered flag;
        // CrafterBlockEntity.setSlotState disables only empty slots.
        let crafter_positions: Vec<(i32, i32, i32)> = self
            .common
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::Crafter(_)))
            .map(|(p, _)| *p)
            .collect();
        for p in crafter_positions {
            let state = self.common.world.block(p.0, p.1, p.2);
            let triggered = self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("triggered"))
                .is_some_and(|v| v == "true");
            if let Some(BlockEntity::Crafter(c)) = self.common.world.block_entities.get_mut(&p) {
                c.triggered = i32::from(triggered);
                c.state_triggered = Some(triggered);
            }
        }
        for d in case["disabled"].as_array().into_iter().flatten() {
            let p = (
                d[0].as_i64().unwrap_or(0) as i32,
                d[1].as_i64().unwrap_or(0) as i32,
                d[2].as_i64().unwrap_or(0) as i32,
            );
            if let Some(BlockEntity::Crafter(c)) = self.common.world.block_entities.get_mut(&p) {
                c.set_slot_state(d[3].as_u64().unwrap_or(0) as usize, false);
            }
        }
        let mut targets = Vec::new();
        for e in case["entities"].as_array().into_iter().flatten() {
            targets.push(self.fixture_target(e)?);
        }
        // Leashable.setLeashedTo: a lead from a target to an earlier one.
        for (e, t) in case["entities"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(&targets)
        {
            let (Some(holder), Target::Mob(id)) = (e["leash"].as_u64(), t) else {
                continue;
            };
            let holder = match targets.get(holder as usize) {
                Some(Target::Mob(h)) => sunrise_sim::mobs::leash::Holder::Mob(*h),
                _ => continue,
            };
            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == *id) {
                m.extra.leash = Some(holder);
            }
        }
        // Creating the targets must not move the stream the dispenser draws from.
        self.common.world.random.set_seed(seed);
        self.common.world.block_log = Some(Vec::new());
        self.start_audio_capture();
        LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
        let old_mobs: Vec<u32> = self.common.mobs.iter().map(|m| m.id).collect();
        let old_stands: Vec<u32> = self.common.armor_stands.iter().map(|s| s.id).collect();
        let old_vehicles: Vec<u32> = (self.common.boats.iter().map(|b| b.id))
            .chain(self.common.carts.iter().map(|c| c.id))
            .collect();

        let pos = &case["pos"];
        let origin = BlockPos::new(
            pos[0].as_i64().unwrap_or(0) as i32,
            pos[1].as_i64().unwrap_or(0) as i32,
            pos[2].as_i64().unwrap_or(0) as i32,
        );
        let state = self.common.world.block(origin.x, origin.y, origin.z);
        let mut used = Vec::new();
        if let Some(op) = case.get("shield_hit") {
            used = self.fixture_shield_hit(op)?;
        } else if let Some(op) = case.get("special_attack") {
            used = self.fixture_special_attack(op)?;
        } else if let Some(op) = case.get("wind_burst") {
            let center = [
                op["center"][0].as_f64().ok_or("wind center x")?,
                op["center"][1].as_f64().ok_or("wind center y")?,
                op["center"][2].as_f64().ok_or("wind center z")?,
            ];
            let breeze = op["breeze"].as_bool().unwrap_or(false);
            self.wind_burst(
                center,
                op["radius"].as_f64().unwrap_or(3.0) as f32,
                1.0,
                super::wind::Source::charge(breeze, false),
            )?;
        } else if let Some(op) = case.get("remove") {
            used = self.fixture_remove(origin, op)?;
        } else if let Some(op) = case.get("use") {
            used = self.fixture_use(origin, op)?;
        } else if let Some(op) = case.get("use_item") {
            used = self.fixture_use_item(origin, op)?;
        } else if let Some(op) = case.get("respawn_at") {
            // ServerPlayer.findRespawnAndUseSpawnBlock for a respawn point at the origin.
            let config = super::respawn_anchor::RespawnConfig::wrapped(
                self.common.dim,
                [origin.x, origin.y, origin.z],
                op["yaw"].as_f64().map_or(0.0, |v| v as f32),
                op["forced"].as_bool().unwrap_or(false),
            );
            let found =
                self.find_respawn_and_use(config, op["consume"].as_bool().unwrap_or(true))?;
            used.push(match found {
                Some((p, yaw, _pitch)) => format!(
                    "respawn {} {} {} {}",
                    num(p.x),
                    num(p.y),
                    num(p.z),
                    yaw.to_bits() as i32
                ),
                None => "respawn none".to_owned(),
            });
        } else if let Some(op) = case.get("wake") {
            used.push(self.fixture_wake(origin, op)?);
        } else if let Some(op) = case.get("attack") {
            used = self.fixture_attack(op, &targets)?;
        } else if case.get("hopper").is_some() {
            // HopperBlockEntity.pushItemsTick, once.
            self.hopper_try_move_items((origin.x, origin.y, origin.z));
            self.server_process_level_events()?;
        } else {
            self.dispense_from(origin, state)?;
            // CrafterBlockEntity.serverTick, the crafting animation running down.
            let mut crafters: Vec<(i32, i32, i32)> = self
                .common
                .world
                .block_entities
                .iter()
                .filter(|(_, e)| matches!(e, BlockEntity::Crafter(_)))
                .map(|(p, _)| *p)
                .collect();
            crafters.sort_unstable();
            for _ in 0..case["ticks"].as_u64().unwrap_or(0) {
                for &p in &crafters {
                    self.tick_crafter(p);
                }
            }
        }

        let mut out = Vec::new();
        let mut changed: Vec<(i32, i32, i32)> =
            self.common.world.block_log.take().unwrap_or_default();
        // Quiet edits such as a sapling advancing its stage do not send a client update.
        // Compare the input cells too, rather than using network notifications as all edits.
        changed.extend(before.keys().copied());
        changed.sort_unstable();
        changed.dedup();
        for p in changed {
            let now = self.common.world.block(p.0, p.1, p.2).0;
            if before
                .get(&p)
                .copied()
                .unwrap_or(self.common.res.facts().air.0)
                != now
            {
                out.push(format!("set {},{},{} {now}", p.0, p.1, p.2));
            }
        }
        let mut containers: Vec<_> = self.common.world.block_entities.keys().copied().collect();
        containers.sort_unstable();
        for p in containers {
            let entity = &self.common.world.block_entities[&p];
            let single = match entity {
                BlockEntity::Jukebox(j) => &j.items,
                _ => entity.slots(),
            };
            for (i, slot) in single.iter().enumerate() {
                if let Some(stack) = slot {
                    out.push(format!(
                        "inv {},{},{} {i} {}",
                        p.0,
                        p.1,
                        p.2,
                        canon_stack(Some(stack), self.res.items())
                    ));
                }
            }
        }
        let mut crafters: Vec<_> = self
            .common
            .world
            .block_entities
            .iter()
            .filter_map(|(p, e)| match e {
                BlockEntity::Crafter(c) => Some((*p, c)),
                _ => None,
            })
            .collect();
        crafters.sort_unstable_by_key(|c| c.0);
        for (p, c) in crafters {
            let disabled: Vec<String> = (0..9)
                .filter(|&i| c.disabled[i])
                .map(|i| i.to_string())
                .collect();
            out.push(format!(
                "crafter {},{},{} ticks={} triggered={} signal={} disabled={}",
                p.0,
                p.1,
                p.2,
                c.crafting_ticks,
                c.is_triggered(),
                c.redstone_signal(),
                disabled.join(",")
            ));
        }
        // A spawner's next spawn data (its id and number of keys), potentials and delay.
        let mut spawners: Vec<_> = self
            .common
            .world
            .block_entities
            .iter()
            .filter_map(|(p, e)| match e {
                BlockEntity::Spawner(s) => Some((*p, s)),
                _ => None,
            })
            .collect();
        spawners.sort_unstable_by_key(|s| s.0);
        for (p, s) in spawners {
            let next = s.next_data().map_or("-".to_owned(), |d| {
                format!(
                    "{}/{}",
                    d.id().unwrap_or_else(|| "?".into()),
                    d.entity.len()
                )
            });
            out.push(format!(
                "spawner {},{},{} next={next} potentials={} delay={}",
                p.0,
                p.1,
                p.2,
                s.potentials.len(),
                s.spawn_delay as i16
            ));
        }
        self.fixture_entities(&old_mobs, &old_stands, &old_vehicles, &mut out);
        if case["pickup_delay"].as_bool().unwrap_or(false) {
            for item in &self.common.dropped {
                out.push(format!(
                    "pickup {} {}",
                    canon_stack(Some(&item.stack), self.res.items()),
                    item.pickup_delay
                ));
            }
        }
        for (i, t) in targets.iter().enumerate() {
            out.push(self.fixture_describe(i, t));
            self.fixture_vehicle_items(i, t, &mut out);
        }
        // Each level event's sound right after the event, as the oracle frames its log.
        for line in LOG.with(|log| log.borrow_mut().take()).unwrap_or_default() {
            let sound = event_sound(&line);
            out.push(line);
            out.extend(sound);
        }
        for s in std::mem::take(&mut self.server.pending_sounds) {
            out.push(format!("S {} {}", floor_pos(s.position), s.event));
        }
        self.stop_audio_capture();
        // The sounds bound to an entity (a sheared mob's) are the server's too.
        for s in std::mem::take(&mut self.server.bound_sounds) {
            out.push(format!("S {} {}", floor_pos(s.at), s.name));
        }
        out.extend(used);
        out.push(format!("rng {}", self.world.random.next_long()));
        Ok(out)
    }

    /// Actual player combat and blocking-item side effects, without a window or connection.
    pub(in crate::game::server_side) fn fixture_shield_hit(
        &mut self,
        op: &Value,
    ) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().ok_or("shield_hit item")?)?
            .ok_or("shield_hit requires an item")?;
        let item = stack.item.clone();
        self.common.creative = op["creative"].as_bool().unwrap_or(false);
        self.common.mode = if self.common.creative {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        let mut player = PlayerMotion::new(
            self.common.motion,
            sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
        )?;
        // A creative player is invulnerable (`GameType.updatePlayerAbilities`): the
        // arrow's hurt is refused before the shield is reached.
        player.invulnerable = self.common.creative;
        self.common.player = Some(player);
        self.common.yaw = 0.0;
        self.common.armor = Default::default();
        self.common.offhand = Some(stack);
        self.common.using = Some((super::shield::OFFHAND_SLOT, 100));
        self.server.combat.0 = Default::default();
        let before = self.common.stats.get("used", &item);
        let from = sunrise_core::geometry::Vec3::new(
            0.0,
            0.0,
            if op["behind"].as_bool().unwrap_or(false) {
                -1.0
            } else {
                1.0
            },
        );
        let mut arrow = sunrise_sim::projectiles::Projectile::new(
            ProjectileKind::Arrow {
                base_damage: op["damage"].as_f64().unwrap_or(8.0),
                critical: false,
                effect: None,
            },
            from,
            None,
        );
        arrow.velocity = sunrise_core::geometry::Vec3::new(0.0, 0.0, 1.0);
        arrow.arrow.as_mut().unwrap().pierce = u8::from(op["pierce"].as_bool().unwrap_or(false));
        self.projectile_hit(
            &arrow,
            sunrise_sim::projectiles::Hit::Entity {
                who: sunrise_sim::mobs::Who::Player,
                at: sunrise_core::geometry::Vec3::ZERO,
            },
        )?;
        let out = vec![format!(
            "shield health={} used={} stack={}",
            num(f64::from(self.player.as_ref().unwrap().health.health())),
            self.stats.get("used", &item) - before,
            self.offhand
                .as_ref()
                .map_or_else(|| "-".to_owned(), |s| format!("{}:{}", s.count, s.damage))
        )];
        self.common.player = None;
        self.common.offhand = None;
        self.common.using = None;
        Ok(out)
    }

    /// A player holding the case's item right-clicks the block at `pos`
    /// (`use_item_as_player`'s block steps): the statistics, the result
    /// and the held stack, as `VanillaDispenserOracle.use` prints them.
    pub(in crate::game::server_side) fn fixture_use(
        &mut self,
        pos: BlockPos,
        op: &Value,
    ) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.common.creative = op["creative"].as_bool().unwrap_or(false);
        self.common.inventory[self.common.selected] = stack.clone();
        let used_before = stack
            .as_ref()
            .map_or(0, |s| self.common.stats.get("used", &s.item));
        let at = [pos.x, pos.y, pos.z];
        let mut consumed = self.use_composter(at, stack.as_ref());
        if !consumed {
            let player = sunrise_world::level::PlayerContext {
                yaw: 0.0,
                pitch: 90.0,
                may_build: true,
                can_eat: self.common.creative,
                game_master: false,
                respawn_point: None,
            };
            let hit = sunrise_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 0.5,
            );
            let state = self.common.world.block(pos.x, pos.y, pos.z);
            let result = self.with_level(|b, level, ticks| {
                b.use_without_item(
                    level,
                    ticks,
                    state,
                    sunrise_core::geometry::BlockHit {
                        block: pos,
                        direction: sunrise_core::geometry::Direction::Up,
                        location: hit,
                        inside: false,
                    },
                    player,
                )
            });
            consumed = result.consumes_action();
        }
        self.server_process_level_events()?;
        let mut out = Vec::new();
        if let Some(s) = &stack {
            let used = self.common.stats.get("used", &s.item) - used_before;
            if used != 0 {
                out.push(format!("stat {} minecraft:used {used}", s.item));
            }
        }
        out.push(format!(
            "use {} held {}",
            if consumed { "consumed" } else { "pass" },
            canon_stack(self.inventory[self.selected].as_ref(), self.res.items())
        ));
        Ok(out)
    }

    /// A player three blocks south of `pos`, turned to the case's `yaw`,
    /// right-clicks its top face (or the case's `face`) holding
    /// the case's item (`use_item_as_player` on that hit): the statistics,
    /// the held stack and the item it started using, as
    /// `VanillaDispenserOracle.useItem` prints them.
    pub(in crate::game::server_side) fn fixture_use_item(
        &mut self,
        pos: BlockPos,
        op: &Value,
    ) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.common.mode = if op["creative"].as_bool().unwrap_or(false) {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        self.common.creative = self.common.mode == GameMode::Creative;
        let distance = op["distance"].as_f64().unwrap_or(3.0);
        let mut player = PlayerMotion::new(
            self.common.motion,
            sunrise_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 0.5 + distance,
            ),
        )?;
        if let Some(r) = op["respawn"].as_array() {
            let p = |i: usize| r.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
            self.server.respawn_config = Some(super::respawn_anchor::RespawnConfig {
                dim: self.common.dim,
                pos: [p(0), p(1), p(2)],
                yaw: 0.0,
                pitch: 0.0,
                forced: false,
            });
        }
        player.health.food.food = op["food"].as_i64().unwrap_or(20) as i32;
        self.common.player = Some(player);
        self.common.yaw = op["yaw"].as_f64().unwrap_or(0.0) as f32;
        self.common.pitch = 90.0;
        self.server_clear_acting_input();
        // The oracle supplies the acting player's logical sneak state for each click.
        self.server_set_acting_sneak(op["sneaking"].as_bool().unwrap_or(false));
        self.common.using = None;
        // The oracle's player wears nothing: no armor left on from an earlier case.
        self.common.armor = Default::default();
        self.common.inventory[self.common.selected] = stack.clone();
        self.common.offhand = parse_stack(op["offhand"].as_str().unwrap_or("-"))?;
        // The used statistic of each item held, once.
        let mut items: Vec<_> = [stack.as_ref(), self.common.offhand.as_ref()]
            .into_iter()
            .flatten()
            .map(|s| s.item.clone())
            .collect();
        items.dedup();
        let used_before: Vec<_> = items
            .iter()
            .map(|i| self.common.stats.get("used", i))
            .collect();
        // The top face, or the case's `face`, clicked at its centre.
        let face = match op["face"].as_str() {
            Some("down") => [0, -1, 0],
            Some("north") => [0, 0, -1],
            Some("south") => [0, 0, 1],
            Some("west") => [-1, 0, 0],
            Some("east") => [1, 0, 0],
            _ => [0, 1, 0],
        };
        let centre = |c: i32, f: i32| f64::from(c) + 0.5 + f64::from(f) * 0.5;
        self.server.forced_hit = Some(super::Hit {
            pos: [pos.x, pos.y, pos.z],
            face,
            point: [
                centre(pos.x, face[0]),
                centre(pos.y, face[1]),
                centre(pos.z, face[2]),
            ],
            inside: false,
        });
        self.server.chat_capture = Some(Vec::new());
        let used = self.use_item_as_player();
        self.server.forced_hit = None;
        let messages = self.server.chat_capture.take().unwrap_or_default();
        used?;
        self.server_process_level_events()?;
        let mut out = Vec::new();
        for m in messages {
            let key = m.component()["translate"]
                .as_str()
                .map_or_else(|| m.plain(), str::to_owned);
            // The oracle player's advancements are not tracked.
            if !key.starts_with("chat.type.advancement") {
                out.push(format!("msg {key}"));
            }
        }
        for (item, before) in items.iter().zip(used_before) {
            let used = self.common.stats.get("used", item) - before;
            if used != 0 {
                out.push(format!("stat {item} minecraft:used {used}"));
            }
        }
        let using = self
            .common
            .using
            .and_then(|(slot, _)| self.common.stack_in(slot))
            .map_or("-".to_owned(), |s| s.item.to_string());
        out.push(format!(
            "held {} offhand {} using {using}",
            canon_stack(self.inventory[self.selected].as_ref(), self.res.items()),
            canon_stack(self.offhand.as_ref(), self.res.items())
        ));
        out.push(match self.server.respawn_config {
            Some(c) => format!(
                "respawn {} {},{},{}",
                c.dim.id(),
                c.pos[0],
                c.pos[1],
                c.pos[2]
            ),
            None => "respawn -".to_owned(),
        });
        self.common.player = None;
        self.common.using = None;
        self.common.offhand = None;
        Ok(out)
    }

    /// Compare a raised item's durability across two frontal hits.
    pub(in crate::game::server_side) fn fixture_blocking_wear(
        &mut self,
        op: &Value,
    ) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().ok_or("missing blocking stack")?)?
            .ok_or("empty blocking stack")?;
        // Reload the stack just as a player inventory does between sessions.
        let saved = sunrise_items::item_nbt::item_to_nbt(&stack, self.common.res.items());
        self.common.inventory[self.common.selected] =
            sunrise_items::item_nbt::item_from_nbt(&saved);
        let (creative, mode) = (self.common.creative, self.common.mode);
        self.common.creative = op["creative"].as_bool().unwrap_or(false);
        self.common.mode = if self.common.creative {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        self.server.combat.0 = Default::default();
        self.common.using = None;
        self.server.shield_cooldown = 0;
        self.common.yaw = 0.0;
        self.common.player = Some(PlayerMotion::new(
            self.common.motion,
            sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
        )?);
        if !self.raise_shield(self.common.selected) || self.common.using.is_none() {
            return Err("could not raise blocking stack".into());
        }
        let mut lines = Vec::new();
        for _ in 0..2 {
            self.hurt_player_from(
                4.0,
                Some([0.0, 64.0, 2.0]),
                "minecraft:arrow",
                sunrise_gameplay_common::game::creatures::KnownCause::None,
                false,
            );
            let stack = self.common.inventory[self.common.selected].as_ref();
            let (count, damage) = stack.map_or((0, 0), |stack| (stack.count, stack.damage));
            lines.push(format!("blocking wear {count} {damage}"));
        }
        self.common.player = None;
        self.common.using = None;
        self.common.inventory[self.common.selected] = None;
        self.common.creative = creative;
        self.common.mode = mode;
        Ok(lines)
    }

    /// The player's hit on the first target with the case's item, from the
    /// attack strength's scaled `base` damage on: the item's damage bonus and
    /// source, the hurt with the weapon's armor effects, then `weapon_hit`
    /// (the mace's smash, post-attack enchantments, the wear), as
    /// `VanillaDispenserOracle.attack` prints them.
    pub(in crate::game::server_side) fn fixture_attack(
        &mut self,
        op: &Value,
        targets: &[Target],
    ) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.common.mode = if op["creative"].as_bool().unwrap_or(false) {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        self.common.creative = self.common.mode == GameMode::Creative;
        let v3 = |v: &Value| {
            sunrise_core::geometry::Vec3::new(
                v[0].as_f64().unwrap_or(0.0),
                v[1].as_f64().unwrap_or(0.0),
                v[2].as_f64().unwrap_or(0.0),
            )
        };
        // A new player: no combat of an earlier case's.
        self.server.combat.0 = Default::default();
        let mut player = PlayerMotion::new(self.common.motion, v3(&op["player"]))?;
        player.velocity = v3(&op["motion"]);
        player.fall_distance = op["fall"].as_f64().unwrap_or(0.0);
        player.fall_flying = op["fall_flying"].as_bool().unwrap_or(false);
        player.flying = op["flying"].as_bool().unwrap_or(false);
        // A creative player's abilities: it may fly and is invulnerable, as
        // `GameType.updatePlayerAbilities` sets them (a victim's Thorns hurts no one then).
        player.may_fly = self.common.creative;
        player.invulnerable = self.common.creative;
        if !op["impact"].is_null() {
            player.impulse.impact_position = Some(v3(&op["impact"]));
        }
        let feet = [player.position.x, player.position.y, player.position.z];
        self.common.player = Some(player);
        self.common.inventory[self.common.selected] = stack.clone();
        let used_before = stack
            .as_ref()
            .map_or(0, |s| self.common.stats.get("used", &s.item));
        let taken_before = self.common.stats.get("custom", "minecraft:damage_taken");
        let base = op["base"].as_f64().unwrap_or(0.0) as f32;
        let source = self.attack_source(feet);
        let (bonus, hurt) = match targets.first() {
            Some(&Target::Mob(id)) => {
                let index = self
                    .common
                    .mobs
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or("the target is gone")?;
                let kind = self.common.mobs[index].kind.id.clone();
                let bonus = self.smash_damage_bonus(&kind, Some(id));
                let join = self.common.mobs[index].join;
                let (hurt, _) = self.hurt_by_player(index, base + bonus, source);
                let index = self
                    .common
                    .mob_index
                    .find(&self.common.mobs, id)
                    .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
                    .ok_or("the target is gone")?;
                // The hurt sound the mob raised, where it stands.
                for event in std::mem::take(&mut self.common.mobs[index].outbox) {
                    if let sunrise_sim::mobs::MobEvent::Sound(s, at, volume, pitch) = event {
                        self.server_play_sound(&format!("minecraft:{s}"), at, volume, pitch);
                    }
                }
                if hurt {
                    self.weapon_hit(index, sunrise_sim::mobs::Damage::player([0.0; 3]));
                }
                (bonus, hurt)
            }
            Some(&Target::Stand(id)) => {
                let index = self
                    .common
                    .armor_stands
                    .iter()
                    .position(|s| s.id == id)
                    .ok_or("the target is gone")?;
                let bonus = self.smash_damage_bonus("minecraft:armor_stand", Some(id));
                (bonus, self.hit_armor_stand(index, |_, _, _| {}))
            }
            _ => return Err("an attack case has a living target or armor stand".into()),
        };
        self.server_process_level_events()?;
        let mut out = Vec::new();
        // What the player's tick makes of its own hurt (the victim's Thorns):
        // the combat record with its statistic, the hurt sound where it
        // stands and the damage game event of the health it lost.
        let events = self
            .common
            .player
            .as_mut()
            .map(|p| std::mem::take(&mut p.events))
            .unwrap_or_default();
        self.record_combat(&events);
        for event in &events {
            if let sunrise_sim::motion::MotionEvent::HurtSound(sound) = event {
                out.push(format!("S {} {sound}", floor_pos(feet)));
            }
        }
        if self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.health.health() < 20.0)
        {
            self.post_entity_event("minecraft:entity_damage", sunrise_sim::mobs::Who::Player);
        }
        if let Some(s) = &stack {
            let used = self.common.stats.get("used", &s.item) - used_before;
            if used != 0 {
                out.push(format!("stat {} minecraft:used {used}", s.item));
            }
        }
        let taken = self.common.stats.get("custom", "minecraft:damage_taken") - taken_before;
        if taken != 0 {
            out.push(format!(
                "stat minecraft:damage_taken minecraft:custom {taken}"
            ));
        }
        let source = match source.kind {
            sunrise_sim::mobs::DamageKind::MaceSmash => "minecraft:mace_smash",
            sunrise_sim::mobs::DamageKind::Spear => "minecraft:spear",
            _ => "minecraft:player_attack",
        };
        out.push(format!(
            "attack bonus={} source={source} hurt={hurt}",
            num(f64::from(bonus))
        ));
        for (i, t) in targets.iter().enumerate() {
            let (health, v) = match *t {
                Target::Mob(id) => match self.common.mobs.iter().find(|m| m.id == id) {
                    Some(m) => (num(f64::from(m.health)), m.velocity),
                    None => continue,
                },
                Target::Stand(id) => match self.common.armor_stands.iter().find(|s| s.id == id) {
                    Some(s) => ("-".to_owned(), s.velocity),
                    None => {
                        out.push(format!("V {i} gone"));
                        continue;
                    }
                },
                Target::Boat(_) | Target::Cart(_) => continue,
            };
            out.push(format!(
                "V {i} {health} {} {} {}",
                num(v.x),
                num(v.y),
                num(v.z)
            ));
        }
        // What the first target wears, worn by its Thorns.
        if let Some(&Target::Mob(id)) = targets.first()
            && let Some(m) = self.common.mobs.iter().find(|m| m.id == id)
        {
            use sunrise_sim::mobs::finalize as f;
            for (slot, name) in [
                (f::FEET, "feet"),
                (f::LEGS, "legs"),
                (f::CHEST, "chest"),
                (f::HEAD, "head"),
            ] {
                if let Some(s) = f::item_stack(m, slot) {
                    out.push(format!(
                        "W {name} {}",
                        canon_stack(Some(&s), self.res.items())
                    ));
                }
            }
        }
        if let Some(p) = self.common.player.as_ref() {
            out.push(format!("H {}", num(f64::from(p.health.health()))));
        }
        if let Some(p) = self.common.player.take() {
            let impact = p.impulse.impact_position.map_or("-".to_owned(), |i| {
                format!("{},{},{}", num(i.x), num(i.y), num(i.z))
            });
            out.push(format!(
                "P {} {} {} fall={} impact={impact} grace={} extra={}",
                num(p.velocity.x),
                num(p.velocity.y),
                num(p.velocity.z),
                num(p.fall_distance),
                p.impulse.reset_grace_time,
                p.spawn_extra_particles_on_fall
            ));
        }
        out.push(format!(
            "held {}",
            canon_stack(self.inventory[self.selected].as_ref(), self.res.items())
        ));
        Ok(out)
    }

    /// A stack in a block entity's slot: a container's, a jukebox's record, a
    /// lectern's book, or the items of a block entity Sunrise keeps as saved.
    pub(in crate::game::server_side) fn fixture_fill(
        &mut self,
        p: (i32, i32, i32),
        slot: usize,
        stack: Option<ItemStack>,
    ) {
        let jukebox = &*self
            .common
            .res
            .facts()
            .get(self.common.world.block(p.0, p.1, p.2))
            .name
            == "minecraft:jukebox";
        match self.common.world.block_entities.get_mut(&p) {
            Some(BlockEntity::Jukebox(j)) => j.items[0] = stack,
            Some(BlockEntity::Lectern(l)) => l.book = stack,
            Some(BlockEntity::Campfire(c)) => {
                if let Some(cell) = c.items.get_mut(slot) {
                    *cell = stack;
                }
            }
            Some(e) => {
                if let Some(cell) = e.slots_mut().get_mut(slot) {
                    *cell = stack;
                }
            }
            None if jukebox => {
                // JukeboxBlockEntity holding its record (setSongItemWithoutPlaying).
                self.common.world.block_entities.insert(
                    p,
                    BlockEntity::Jukebox(sunrise_level::containers::Jukebox {
                        song_clock_present: stack
                            .as_ref()
                            .and_then(|s| self.common.res.items().get(&s.item))
                            .is_some_and(|i| i.jukebox_song.is_some()),
                        items: [stack],
                        ticks: 0,
                        playing: false,
                        loaded_clock: false,
                    }),
                );
            }
            None => self.fixture_fill_saved(p, slot, stack),
        }
    }
    /// The saved block entity of a campfire, decorated pot, chiseled
    /// bookshelf or shelf, as a loaded world keeps it.
    pub(in crate::game::server_side) fn fixture_fill_saved(
        &mut self,
        p: (i32, i32, i32),
        slot: usize,
        stack: Option<ItemStack>,
    ) {
        use sunrise_core::nbt::Tag;
        let name = self
            .common
            .res
            .facts()
            .get(self.common.world.block(p.0, p.1, p.2))
            .name
            .clone();
        let id = match name.strip_prefix("minecraft:").unwrap_or(&name) {
            "campfire" | "soul_campfire" => "minecraft:campfire",
            "decorated_pot" => "minecraft:decorated_pot",
            "chiseled_bookshelf" => "minecraft:chiseled_bookshelf",
            n if n.ends_with("_shelf") => "minecraft:shelf",
            _ => return,
        };
        let Some(chunk) = self.common.world.chunks.get_mut(&(p.0 >> 4, p.2 >> 4)) else {
            return;
        };
        let chunk = Arc::make_mut(chunk);
        let saved = Arc::make_mut(chunk.saved.get_or_insert_with(Default::default));
        let at = |t: &Tag| {
            t.get("x") == Some(&Tag::Int(p.0))
                && t.get("y") == Some(&Tag::Int(p.1))
                && t.get("z") == Some(&Tag::Int(p.2))
        };
        let index = match saved.block_entities.iter().position(at) {
            Some(i) => i,
            None => {
                saved.block_entities.push(Tag::compound([
                    ("id", Tag::String(id.into())),
                    ("x", Tag::Int(p.0)),
                    ("y", Tag::Int(p.1)),
                    ("z", Tag::Int(p.2)),
                ]));
                saved.block_entities.len() - 1
            }
        };
        let Tag::Compound(map) = &mut saved.block_entities[index] else {
            return;
        };
        let Some(stack) = stack else { return };
        if id == "minecraft:decorated_pot" {
            map.insert(
                "item".into(),
                sunrise_sim::save::item_to_nbt(&stack, self.common.res.items()),
            );
            return;
        }
        let mut item = sunrise_sim::save::item_to_nbt(&stack, self.common.res.items());
        if let Tag::Compound(c) = &mut item {
            c.insert("Slot".into(), Tag::Byte(slot as i8));
        }
        match map
            .entry("Items".into())
            .or_insert_with(|| Tag::List(Vec::new()))
        {
            Tag::List(list) => list.push(item),
            other => *other = Tag::List(vec![item]),
        }
    }
    /// `Level.setBlock`, `destroyBlock` or `removeBlock` on the case's block, or
    /// a player mining it (`fixture_mine`).
    pub(in crate::game::server_side) fn fixture_remove(
        &mut self,
        origin: BlockPos,
        op: &Value,
    ) -> Result<Vec<String>> {
        match op["op"].as_str().unwrap_or("") {
            "set" => {
                let state = BlockStateId(op["state"].as_u64().unwrap_or(0) as u32);
                let flags = op["flags"].as_i64().unwrap_or(3) as i32;
                self.with_level(|b, level, ticks| b.set_block(level, ticks, origin, state, flags));
            }
            "destroy" => {
                let drop = op["drop"].as_bool().unwrap_or(true);
                self.with_level(|b, level, ticks| b.destroy_block(level, ticks, origin, drop));
            }
            "remove" => {
                self.with_level(|b, level, ticks| b.remove_block(level, ticks, origin, false));
            }
            "mine" => return self.fixture_mine(origin, op),
            other => return Err(format!("unknown removal {other}").into()),
        }
        self.server_process_level_events()?;
        Ok(Vec::new())
    }

    /// A player three blocks south of `pos` mines it holding the case's item
    /// (`break_block`, which every player's mining goes through), in survival
    /// or in creative, where the block and its contents never drop: the
    /// statistics and the held stack, as `VanillaDispenserOracle.mine` prints
    /// them.
    pub(in crate::game::server_side) fn fixture_mine(
        &mut self,
        pos: BlockPos,
        op: &Value,
    ) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.common.mode = if op["creative"].as_bool().unwrap_or(false) {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        self.common.creative = self.common.mode == GameMode::Creative;
        self.common.player = Some(PlayerMotion::new(
            self.common.motion,
            sunrise_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 3.5,
            ),
        )?);
        self.common.yaw = 0.0;
        self.common.pitch = 90.0;
        self.server_clear_acting_input();
        self.common.inventory[self.common.selected] = stack.clone();
        self.common.offhand = None;
        let state = self.common.world.block(pos.x, pos.y, pos.z);
        let block = self.common.res.facts().get(state).name.clone();
        let item = stack.as_ref().map(|s| s.item.clone());
        let before = |game: &Self| {
            let used = item.as_ref().map_or(0, |i| game.stats.get("used", i));
            (used, game.stats.get("mined", &block))
        };
        let (used, mined) = before(self);
        self.break_block([pos.x, pos.y, pos.z])?;
        let (used_after, mined_after) = before(self);
        let mut out = Vec::new();
        if let Some(item) = &item
            && used_after != used
        {
            out.push(format!("stat {item} minecraft:used {}", used_after - used));
        }
        if mined_after != mined {
            out.push(format!(
                "stat {block} minecraft:mined {}",
                mined_after - mined
            ));
        }
        out.push(format!(
            "mine held {}",
            canon_stack(self.inventory[self.selected].as_ref(), self.res.items())
        ));
        Ok(out)
    }

    /// A villager lying in the bed at `origin` with `yaw` wakes up
    /// (`Villager.stopSleeping`): where it stands and the yaw it turns to.
    pub(in crate::game::server_side) fn fixture_wake(
        &mut self,
        origin: BlockPos,
        op: &Value,
    ) -> Result<String> {
        let kind = self
            .common
            .res
            .mob_types()
            .get("minecraft:villager")
            .cloned()
            .ok_or("no mob type minecraft:villager")?;
        let bed = [origin.x, origin.y, origin.z];
        let lying = sunrise_core::geometry::Vec3::new(
            f64::from(bed[0]) + 0.5,
            f64::from(bed[1]) + 0.6875,
            f64::from(bed[2]) + 0.5,
        );
        let yaw = op["yaw"].as_f64().unwrap_or(0.0) as f32;
        let mut villager = Mob::new(1, kind, lying, yaw);
        sunrise_sim::mobs::villager::ext_mut(&mut villager).sleeping_pos = Some(bed);
        let mut events = Vec::new();
        let w = self.mob_world_base(&[]);
        sunrise_sim::mobs::villager::stop_sleeping(&mut villager, &w, &mut events);
        let p = villager.position;
        Ok(format!(
            "wake {} {} {} {}",
            num(p.x),
            num(p.y),
            num(p.z),
            villager.yaw.to_bits() as i32
        ))
    }

    /// `EntityType.create` and `snapTo` for one target: no spawn finalization.
    pub(in crate::game::server_side) fn fixture_target(&mut self, e: &Value) -> Result<Target> {
        let kind = e["type"].as_str().unwrap_or("minecraft:pig");
        let p = &e["pos"];
        let at = sunrise_core::geometry::Vec3::new(
            p[0].as_f64().unwrap_or(0.0),
            p[1].as_f64().unwrap_or(0.0),
            p[2].as_f64().unwrap_or(0.0),
        );
        let yaw = e["yaw"].as_f64().unwrap_or(0.0) as f32;
        self.server.next_entity += 1;
        let id = self.server.next_entity;
        if let Some(cart) = super::minecarts::CartKind::from_id(kind) {
            let mut c = super::minecarts::Minecart::new(id, cart, at);
            c.yaw = yaw;
            self.common.carts.push(c);
            return Ok(Target::Cart(id));
        }
        if let Some((wood, chest)) =
            sunrise_gameplay_common::game::boats_common::any_boat_wood(kind)
        {
            let mut boat =
                sunrise_gameplay_common::game::boats_common::Boat::new(id, wood.into(), at, yaw);
            if chest {
                boat = boat.with_chest();
            }
            self.common.boats.push(boat);
            return Ok(Target::Boat(id));
        }
        if kind == "minecraft:armor_stand" {
            let mut stand = sunrise_sim::entities::ArmorStand::new(id, at, yaw);
            stand.marker = e["marker"].as_bool().unwrap_or(false);
            stand.ground.on_ground = e["ground"].as_bool().unwrap_or(false);
            stand.show_arms = e["arms"].as_bool().unwrap_or(false);
            stand.disabled_slots = e["disabled"].as_i64().unwrap_or(0) as i32;
            if let Some(last_hit) = e["last_hit"].as_i64() {
                stand.last_hit = last_hit;
            }
            self.common.armor_stands.push(stand);
            return Ok(Target::Stand(id));
        }
        let t = self
            .common
            .res
            .mob_types()
            .get(kind)
            .cloned()
            .ok_or_else(|| format!("no mob type {kind}"))?;
        let mut mob = Mob::new(id, t, at, yaw);
        // EntityType.create equips nothing (that is finalizeSpawn's job).
        mob.extra.set_body_stack(None);
        for slot in sunrise_items::equipment::MAINHAND..=sunrise_items::equipment::HEAD {
            mob.extra.set_slot_stack(slot, None);
        }
        let s = mob.species();
        let flag = |name: &str| e[name].as_bool().unwrap_or(false);
        let ageable = !matches!(
            s,
            Species::Zombie | Species::Skeleton | Species::Bogged | Species::SnowGolem
        );
        if flag("baby") && ageable {
            mob.extra.age = -24000;
        }
        let tamable = body_equipment_species(s)
            || matches!(s, Species::Cat | Species::Parrot | Species::Ocelot);
        if flag("tame") && tamable {
            mob.extra.tame = true;
        }
        if flag("sheared") && matches!(s, Species::Sheep | Species::Bogged | Species::SnowGolem) {
            mob.extra.sheared = true;
        }
        if flag("chest")
            && matches!(
                s,
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
            )
        {
            mob.extra.chested = true;
        }
        if flag("loot") {
            mob.extra.pick_up_loot = true;
        }
        mob.ground.on_ground = flag("ground");
        if let Some(health) = e["health"].as_f64() {
            mob.attrs.max_health = health as f32;
            mob.health = health as f32;
        }
        if let Some(resistance) = e["knockback_resistance"].as_f64() {
            mob.attrs.knockback_resistance = resistance;
        }
        if flag("brown") && s == Species::Mooshroom {
            mob.extra.variant = 1;
        }
        let equip = e["equip"].as_object();
        for (name, snbt) in equip.into_iter().flatten() {
            if let (Some(slot), Some(stack)) = (
                Slot::from_name(name),
                parse_stack(snbt.as_str().unwrap_or("-"))?,
            ) {
                // `setItemSlot` keeps the whole stack: a piece's enchantments and damage.
                let kept = ["mainhand", "offhand", "feet", "legs", "chest", "head"]
                    .iter()
                    .position(|s| s == name)
                    .filter(|_| stack.extra.is_some() || stack.damage > 0);
                match kept {
                    Some(at) => sunrise_sim::mobs::finalize::set_item_stack(&mut mob, at, stack),
                    None => mob_set_slot(&mut mob, slot, stack),
                }
            }
        }
        // VanillaDispenserOracle's target has ticked since it was equipped: its
        // collectEquipmentChanges put the worn pieces' armor and other modifiers
        // on its attributes, which the hit's absorption reads.
        if equip.is_some_and(|equip| !equip.is_empty()) {
            crate::game::living_attributes::collect_equipment(
                &self.common.motion.attributes,
                self.items(),
                &mut mob,
            );
        }
        self.common.mobs.push(mob);
        Ok(Target::Mob(id))
    }

    /// Occupied vehicle slots, shared by the target summary and full item records.
    pub(in crate::game::server_side) fn fixture_vehicle_stacks(
        &self,
        target: &Target,
    ) -> impl Iterator<Item = (usize, &ItemStack)> {
        let items = match *target {
            Target::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| &b.items),
            Target::Cart(id) => self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| &c.items),
            Target::Mob(_) | Target::Stand(_) => None,
        };
        items
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(slot, stack)| stack.as_ref().map(|stack| (slot, stack)))
    }

    /// `Tinv` lines: a container entity target's items by slot, as
    /// `VanillaDispenserOracle` prints a `ContainerEntity`'s.
    pub(in crate::game::server_side) fn fixture_vehicle_items(
        &self,
        index: usize,
        target: &Target,
        out: &mut Vec<String>,
    ) {
        for (slot, stack) in self.fixture_vehicle_stacks(target) {
            out.push(format!(
                "Tinv {index} {slot} {}",
                canon_stack(Some(stack), self.res.items())
            ));
        }
    }

    /// `T` lines: removal, equipment by `EquipmentSlot`, shearing readiness, chests.
    pub(in crate::game::server_side) fn fixture_describe(
        &self,
        index: usize,
        target: &Target,
    ) -> String {
        let mut line = format!("T {index} ");
        match *target {
            Target::Boat(id) => match self.common.boats.iter().find(|b| b.id == id) {
                Some(b) => line.push_str(&b.entity_type()),
                None => line.push_str("removed"),
            },
            Target::Cart(id) => match self.common.carts.iter().find(|c| c.id == id) {
                Some(c) => line.push_str(c.kind.id()),
                None => line.push_str("removed"),
            },
            Target::Stand(id) => {
                line.push_str("minecraft:armor_stand");
                let Some(stand) = self.common.armor_stands.iter().find(|s| s.id == id) else {
                    line.push_str(" removed");
                    return line;
                };
                for (slot, name) in [
                    (0, "mainhand"),
                    (1, "offhand"),
                    (2, "feet"),
                    (3, "legs"),
                    (4, "chest"),
                    (5, "head"),
                ] {
                    if let Some(s) = &stand.equipment[slot] {
                        let _ = write!(line, " {name}={}x{}", s.item, s.count);
                    }
                }
            }
            Target::Mob(id) => {
                let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
                    return line + "removed";
                };
                line.push_str(&m.kind.id);
                if m.removed || !m.alive() {
                    line.push_str(" removed");
                }
                let s = m.species();
                let x = &m.extra;
                for (slot, name) in [
                    (Slot::MainHand, "mainhand"),
                    (Slot::OffHand, "offhand"),
                    (Slot::Feet, "feet"),
                    (Slot::Legs, "legs"),
                    (Slot::Chest, "chest"),
                    (Slot::Head, "head"),
                    (Slot::Body, "body"),
                    (Slot::Saddle, "saddle"),
                ] {
                    if let Some(item) = mob_slot_item(m, slot) {
                        let _ = write!(line, " {name}={item}x1");
                    }
                }
                let ready = match s {
                    Species::Sheep => Some(!x.sheared && !m.is_baby()),
                    Species::Mooshroom => Some(!m.is_baby()),
                    Species::Bogged => Some(!x.sheared),
                    Species::SnowGolem => Some(!x.sheared),
                    Species::CopperGolem => Some(
                        self.common.golem_shearable(
                            self.common
                                .mobs
                                .iter()
                                .position(|o| o.id == id)
                                .unwrap_or(0),
                        ),
                    ),
                    _ => None,
                };
                if let Some(r) = ready {
                    line.push_str(if r { " ready" } else { " unready" });
                }
                if x.chested
                    && matches!(
                        s,
                        Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
                    )
                {
                    line.push_str(" chest");
                }
                if x.leash.is_some() {
                    line.push_str(" leashed");
                }
            }
        }
        for (slot, stack) in self.fixture_vehicle_stacks(target) {
            let _ = write!(
                line,
                " slot{slot}={}",
                canon_stack(Some(stack), self.res.items())
            );
        }
        line
    }

    /// `N` lines for every entity the dispenser added.
    pub(in crate::game::server_side) fn fixture_entities(
        &self,
        old_mobs: &[u32],
        old_stands: &[u32],
        old_vehicles: &[u32],
        out: &mut Vec<String>,
    ) {
        let line = |kind: &str, p: [f64; 3], v: [f64; 3], extra: &str| {
            let mut l = format!(
                "N {kind} {} {} {} {} {} {}",
                num(p[0]),
                num(p[1]),
                num(p[2]),
                num(v[0]),
                num(v[1]),
                num(v[2])
            );
            if !extra.is_empty() {
                l.push(' ');
                l.push_str(extra);
            }
            l
        };
        for m in self
            .common
            .mobs
            .iter()
            .filter(|m| !old_mobs.contains(&m.id))
        {
            let (p, v) = (m.position, m.velocity);
            out.push(line(&m.kind.id, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        if let Some(d) = self.common.dragon.as_ref() {
            let p = d.position;
            out.push(line(
                "minecraft:ender_dragon",
                [p.x, p.y, p.z],
                [0.0; 3],
                "",
            ));
        }
        for s in self
            .common
            .armor_stands
            .iter()
            .filter(|s| !old_stands.contains(&s.id))
        {
            let (p, v) = (s.position, s.velocity);
            out.push(line(
                "minecraft:armor_stand",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                "",
            ));
        }
        for d in &self.common.dropped {
            let (p, v) = (d.position, d.velocity);
            out.push(line(
                "minecraft:item",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                &canon_stack(Some(&d.stack), self.common.res.items()),
            ));
        }
        for shot in &self.common.creatures.projectiles {
            let kind = match &shot.kind {
                ProjectileKind::Arrow {
                    effect: Some(("minecraft:glowing", 200)),
                    ..
                } => "minecraft:spectral_arrow",
                ProjectileKind::Arrow { .. } => "minecraft:arrow",
                ProjectileKind::Snowball => "minecraft:snowball",
                ProjectileKind::Egg => "minecraft:egg",
                ProjectileKind::SmallFireball => "minecraft:small_fireball",
                ProjectileKind::WindCharge => "minecraft:wind_charge",
                ProjectileKind::Potion(_, true) => "minecraft:lingering_potion",
                ProjectileKind::Potion(..) => "minecraft:splash_potion",
                ProjectileKind::ExperienceBottle => "minecraft:experience_bottle",
                _ => "minecraft:unknown_projectile",
            };
            let (p, v) = (shot.position, shot.velocity);
            out.push(line(kind, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        for (kind, p, v) in self.common.potions.thrown_entities() {
            out.push(line(kind, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        for (p, v) in self.common.rockets.iter().map(|r| r.motion()) {
            out.push(line(
                "minecraft:firework_rocket",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                "",
            ));
        }
        for t in &self.common.tnts {
            let (p, v) = (t.position, t.velocity);
            out.push(line(
                "minecraft:tnt",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                &format!("fuse={}", t.fuse),
            ));
        }
        for b in self
            .common
            .boats
            .iter()
            .filter(|b| !old_vehicles.contains(&b.id))
        {
            let kind = b.entity_type();
            let (p, v) = (b.position, b.velocity);
            out.push(line(&kind, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        // A merged orb prints once per count, as the oracle does: whether orbs
        // merge depends on entity ids, which differ from vanilla's.
        for o in &self.common.orbs {
            let (p, v) = (o.position, o.velocity);
            for _ in 0..o.count.max(1) {
                out.push(line(
                    "minecraft:experience_orb",
                    [p.x, p.y, p.z],
                    [v.x, v.y, v.z],
                    &format!("value={}", o.value),
                ));
            }
        }
        for c in self
            .common
            .carts
            .iter()
            .filter(|c| !old_vehicles.contains(&c.id))
        {
            let (p, v) = (c.position, c.velocity);
            out.push(line(c.kind.id(), [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        for c in &self.common.crystals {
            let p = c.position;
            out.push(line("minecraft:end_crystal", [p.x, p.y, p.z], [0.0; 3], ""));
        }
        // A hanging entity's position is its box's centre (HangingEntity.recalculateBoundingBox).
        let centre =
            |(lo, hi): ([f64; 3], [f64; 3])| std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0);
        for p in &self.common.paintings {
            out.push(line("minecraft:painting", centre(p.bounds()), [0.0; 3], ""));
        }
        for f in &self.common.item_frames {
            let kind = if f.glow {
                "minecraft:glow_item_frame"
            } else {
                "minecraft:item_frame"
            };
            out.push(line(kind, centre(f.bounds()), [0.0; 3], ""));
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
