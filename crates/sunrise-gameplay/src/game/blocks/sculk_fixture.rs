//! The Rust side of tools/test_sculk_parity.py: with
//! `SUNRISE_SCULK_FIXTURE=SCENARIOS,OUTPUT` the game loads its resources and
//! runs every scenario (sculk blocks, the player with a warning level, scripted
//! game events and block changes) through its own sculk code on a fresh
//! headless world, ticking in `ServerLevel.tick`'s order, and prints what
//! changed after every tick in the format of `VanillaSculkOracle`.
use super::dispense_fixture;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    io::Write as _,
    sync::Arc,
};
use sunrise_command::args::GameMode;
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_game_data::Result;
use sunrise_level::{
    chunk::{Chunk, MIN_Y},
    containers::BlockEntity,
    sculk::{Info, SculkKind, WardenSpawnTracker},
};
use sunrise_sim::{
    mobs::{Mob, Who},
    motion::PlayerMotion,
};
use sunrise_world::{chunk::BlockState, level::Ticks, registry::BlockStateId};

pub(in crate::game) struct MobSpec {
    pub(in crate::game) kind: String,
    pub(in crate::game) pos: [f64; 3],
    pub(in crate::game) yaw: f32,
    pub(in crate::game) seed: i64,
    pub(in crate::game) flags: Vec<String>,
}
#[derive(Default)]
pub(in crate::game) struct Scenario {
    pub(in crate::game) name: String,
    pub(in crate::game) ticks: i32,
    pub(in crate::game) trace_events: bool,
    pub(in crate::game) origin: [i32; 2],
    pub(in crate::game) level_seed: i64,
    pub(in crate::game) difficulty: String,
    pub(in crate::game) wardens: bool,
    pub(in crate::game) blocks: Vec<Vec<String>>,
    pub(in crate::game) player: Option<[f64; 3]>,
    pub(in crate::game) mode: String,
    pub(in crate::game) sneak: bool,
    pub(in crate::game) warning: Option<i32>,
    pub(in crate::game) mobs: Vec<MobSpec>,
    pub(in crate::game) watch: Vec<[i32; 3]>,
    pub(in crate::game) regions: Vec<[i32; 6]>,
    pub(in crate::game) actions: Vec<(i32, Vec<String>)>,
}

pub(in crate::game::server_side) fn parse(text: &str) -> Vec<Scenario> {
    let mut out = Vec::new();
    let mut cur: Option<Scenario> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() || f[0].starts_with('#') {
            continue;
        }
        let s = cur.get_or_insert_with(Scenario::default);
        match f[0] {
            "scenario" => {
                s.name = f[1].to_owned();
                s.difficulty = "normal".into();
                s.wardens = true;
                s.mode = "survival".into();
                for kv in &f[2..] {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    match k {
                        "traceevents" => s.trace_events = true,
                        "ticks" => s.ticks = v.parse().unwrap_or(0),
                        "levelseed" => s.level_seed = v.parse().unwrap_or(0),
                        "difficulty" => s.difficulty = v.to_owned(),
                        "nowardens" => s.wardens = false,
                        _ => {}
                    }
                }
            }
            "origin" => s.origin = [f[1].parse().unwrap_or(0), f[2].parse().unwrap_or(0)],
            "fill" | "block" => s.blocks.push(f.iter().map(|x| (*x).to_owned()).collect()),
            "player" => {
                s.player = Some([
                    f[1].parse().unwrap_or(0.0),
                    f[2].parse().unwrap_or(0.0),
                    f[3].parse().unwrap_or(0.0),
                ]);
                for w in &f[4..] {
                    if *w == "sneak" {
                        s.sneak = true;
                    } else if let Some(level) = w.strip_prefix("warn=") {
                        s.warning = level.parse().ok();
                    } else {
                        s.mode = (*w).to_owned();
                    }
                }
            }
            "mob" => s.mobs.push(MobSpec {
                kind: f[1].to_owned(),
                pos: [
                    f[2].parse().unwrap_or(0.0),
                    f[3].parse().unwrap_or(0.0),
                    f[4].parse().unwrap_or(0.0),
                ],
                yaw: f[5].parse().unwrap_or(0.0),
                seed: f[6].parse().unwrap_or(0),
                flags: f[7..].iter().map(|x| (*x).to_owned()).collect(),
            }),
            "watch" => s.watch.push([
                f[1].parse().unwrap_or(0),
                f[2].parse().unwrap_or(0),
                f[3].parse().unwrap_or(0),
            ]),
            "region" => {
                let n = |i: usize| f[i].parse::<i32>().unwrap_or(0);
                s.regions.push([n(1), n(2), n(3), n(4), n(5), n(6)]);
            }
            "at" => s.actions.push((
                f[1].parse().unwrap_or(0),
                f[2..].iter().map(|x| (*x).to_owned()).collect(),
            )),
            "end" => out.extend(cur.take()),
            _ => {}
        }
    }
    out
}

/// An entity's own random is unseeded in vanilla, and so are its sensors'
/// first delays and its brain's `RunOne` shuffles: the oracle seeds what
/// appeared, and so does this.
pub(in crate::game::server_side) fn seed_fresh(m: &mut Mob, seed: i64) {
    m.random.set_seed(seed);
    sunrise_sim::mobs::warden::init_brain(m, seed);
}
pub(in crate::game::server_side) fn d(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
pub(in crate::game::server_side) fn f(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

/// Sounds the client plays for level events (3006 sculk charge, 3007
/// shriek) and for itself (the warden's heartbeat): vanilla's server never
/// sends them.
pub(in crate::game) const CLIENT_SOUNDS: &[&str] = &[
    "minecraft:block.sculk_shrieker.shriek",
    "minecraft:block.sculk.charge",
    "minecraft:entity.warden.heartbeat",
];

impl crate::game::ServerGame<'_> {
    /// Run every scenario of `spec` ("SCENARIOS,OUTPUT") and write the results.
    pub fn sculk_fixture(&mut self, spec: &str) -> Result<()> {
        assert!(
            !self.options.execution_role.is_client(),
            "fixture requires a server Game"
        );
        let (input, output) = spec
            .split_once(',')
            .ok_or("SUNRISE_SCULK_FIXTURE=SCENARIOS,OUTPUT")?;
        let text = std::fs::read_to_string(input)?;
        // The fixture enters before world_entry, which normally loads the
        // server's death-loot tables (including experience drops).
        self.load_server_loot();
        let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
        for s in parse(&text) {
            writeln!(out, "== {}", s.name)?;
            match self.sculk_scenario(&s) {
                Ok(lines) => out.write_all(lines.as_bytes())?,
                Err(e) => writeln!(out, "0 failed {e}")?,
            }
        }
        Ok(())
    }

    /// `minecraft:name[key=value,...]` to a state.
    pub(in crate::game::server_side) fn fixture_state(&self, text: &str) -> Result<BlockStateId> {
        let (name, props) = match text.split_once('[') {
            Some((n, rest)) => (n, rest.trim_end_matches(']')),
            None => (text, ""),
        };
        let registry = self.common.pack.assets.registry;
        let base = registry
            .default_state(name)
            .ok_or_else(|| format!("unknown block {name}"))?;
        let mut properties: BTreeMap<String, String> = registry
            .state(base)
            .map(|s| s.properties.clone().into_iter().collect())
            .unwrap_or_default();
        for kv in props.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or("property")?;
            properties.insert(k.to_owned(), v.to_owned());
        }
        Ok(registry.resolve(&BlockState {
            name: name.to_owned(),
            properties: properties.into(),
        })?)
    }

    /// `BlockStateParser.serialize`.
    pub(in crate::game::server_side) fn fixture_state_text(&self, state: BlockStateId) -> String {
        let Some(s) = self.common.pack.assets.registry.state(state) else {
            return "?".into();
        };
        if s.properties.is_empty() {
            return s.name.clone();
        }
        let mut sorted: Vec<(&String, &String)> = s.properties.iter().collect();
        sorted.sort();
        let props: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{}[{}]", s.name, props.join(","))
    }

    pub(in crate::game::server_side) fn fixture_who(
        &self,
        who: Option<Who>,
        names: &HashMap<u32, String>,
    ) -> String {
        match who {
            None => "none".into(),
            Some(Who::Player) => "player".into(),
            Some(Who::Mob(id)) => names.get(&id).cloned().unwrap_or_else(|| format!("e{id}")),
        }
    }

    pub(in crate::game::server_side) fn fixture_info(
        &self,
        info: Option<&Info>,
        names: &HashMap<u32, String>,
    ) -> String {
        let Some(i) = info else {
            return "-".into();
        };
        // A source entity that is gone reads as none (VibrationInfo.getEntity).
        let alive = |w: Option<Who>| match w {
            Some(Who::Mob(id)) if !self.common.mobs.iter().any(|m| m.id == id && !m.removed) => {
                None
            }
            other => other,
        };
        format!(
            "{}/{}/{},{},{}/{}/{}",
            i.event.trim_start_matches("minecraft:"),
            f(i.distance),
            d(i.pos[0]),
            d(i.pos[1]),
            d(i.pos[2]),
            match i.source {
                Some(sunrise_game_data::who::EventSource::Entity(id))
                    if self.level_entity(id as i32).is_some() =>
                {
                    names.get(&id).cloned().unwrap_or_else(|| format!("e{id}"))
                }
                source => self.fixture_who(alive(source.and_then(|s| s.actor())), names),
            },
            self.fixture_who(alive(i.owner), names)
        )
    }

    pub(in crate::game::server_side) fn sculk_describe(
        &self,
        p: [i32; 3],
        names: &HashMap<u32, String>,
    ) -> String {
        let state = self.common.world.block(p[0], p[1], p[2]);
        let mut line = self.fixture_state_text(state);
        if let Some(BlockEntity::Sculk(e)) =
            self.common.world.block_entities.get(&(p[0], p[1], p[2]))
        {
            let vibration = |line: &mut String| {
                let v = &e.vibration;
                let _ = write!(
                    line,
                    " cur={} travel={}",
                    self.fixture_info(v.current.as_ref(), names),
                    v.travel
                );
                match v.selector.current() {
                    None => line.push_str(" sel=-"),
                    Some((info, tick)) => {
                        let _ = write!(
                            line,
                            " sel={}@{tick}",
                            self.fixture_info(Some(&info), names)
                        );
                    }
                }
            };
            match e.kind {
                SculkKind::Sensor | SculkKind::Calibrated => {
                    let _ = write!(line, " freq={}", e.last_frequency);
                    vibration(&mut line);
                }
                SculkKind::Shrieker => {
                    let _ = write!(line, " warn={}", e.warning_level);
                    vibration(&mut line);
                }
                SculkKind::Catalyst => {
                    let _ = write!(line, " cursors={}", e.spreader.cursors.len());
                    for c in &e.spreader.cursors {
                        let _ = write!(
                            line,
                            " {},{},{}:{}:{}:{}:",
                            c.pos.x, c.pos.y, c.pos.z, c.charge, c.update_delay, c.decay_delay
                        );
                        match &c.facings {
                            None => line.push('-'),
                            Some(faces) => {
                                let mut faces = faces.clone();
                                faces.sort_by_key(|d| *d as u8);
                                for d in faces {
                                    line.push(match d {
                                        Direction::Down => 'd',
                                        Direction::Up => 'u',
                                        Direction::North => 'n',
                                        Direction::South => 's',
                                        Direction::West => 'w',
                                        Direction::East => 'e',
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        line
    }

    pub(in crate::game::server_side) fn sculk_fixture_reset(&mut self, s: &Scenario) -> Result<()> {
        self.common.world =
            sunrise_gameplay_common::game::world_for_mods(self.common.res.facts().air, 0);
        let plains = u32::from(world_biomes!(self).id("minecraft:plains"));
        let layers = [
            self.fixture_state("minecraft:bedrock")?,
            self.fixture_state("minecraft:dirt")?,
            self.fixture_state("minecraft:dirt")?,
            self.fixture_state("minecraft:grass_block")?,
        ];
        let (ocx, ocz) = (s.origin[0] >> 4, s.origin[1] >> 4);
        for cx in ocx - 4..=ocx + 4 {
            for cz in ocz - 4..=ocz + 4 {
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    Arc::make_mut(section).biomes = sunrise_core::paletted::PalettedContainer::new(
                        sunrise_level::chunk::BIOME_STRATEGY,
                        plains,
                    );
                }
                chunk.lit = true;
                self.common.world.chunks.insert((cx, cz), Arc::new(chunk));
                for x in 0..16 {
                    for z in 0..16 {
                        for (y, layer) in layers.iter().enumerate() {
                            self.common.world.set(
                                cx * 16 + x,
                                MIN_Y + y as i32,
                                cz * 16 + z,
                                *layer,
                            )?;
                        }
                    }
                }
            }
        }
        self.common.block_ticks = Ticks::new(0);
        self.server.tick_chunks.clear();
        self.sync_tick_chunks();
        self.common.mobs.clear();
        if s.trace_events {
            self.common.creatures.projectiles.clear();
        }
        self.common.dropped.clear();
        self.common.orbs.clear();
        self.common.level_events.clear();
        self.common.ui.rules = Default::default();
        for (rule, value) in [
            ("spawn_mobs", false),
            ("spawn_monsters", false),
            ("spawn_patrols", false),
            ("spawn_phantoms", false),
            ("spawn_wandering_traders", false),
            ("advance_time", false),
            ("advance_weather", false),
            ("raids", false),
            ("spawn_wardens", s.wardens),
        ] {
            self.common
                .ui
                .rules
                .set(rule, sunrise_game_data::gamerules::Value::Bool(value));
        }
        self.common.ui.rules.set(
            "random_tick_speed",
            sunrise_game_data::gamerules::Value::Int(0),
        );
        use sunrise_world::food::Difficulty;
        let difficulty = match s.difficulty.as_str() {
            "peaceful" => Difficulty::Peaceful,
            "easy" => Difficulty::Easy,
            "hard" => Difficulty::Hard,
            _ => Difficulty::Normal,
        };
        self.common.world.health_rules.difficulty = difficulty;
        self.common.ui.difficulty = match difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        };
        self.common.level.seed = 0;
        self.common.potions.effects = Default::default();
        self.server.sculk = Default::default();
        self.server.sculk.tracker_loaded = true;
        self.sculk_prepare();
        Ok(())
    }

    pub(in crate::game::server_side) fn sculk_scenario(&mut self, s: &Scenario) -> Result<String> {
        self.sculk_fixture_reset(s)?;
        let at = s.player.unwrap_or([
            f64::from(s.origin[0]) + 0.5,
            100.0,
            f64::from(s.origin[1]) + 0.5,
        ]);
        self.common.mode = match s.mode.as_str() {
            "creative" => GameMode::Creative,
            "spectator" => GameMode::Spectator,
            "adventure" => GameMode::Adventure,
            _ => GameMode::Survival,
        };
        self.common.creative = self.common.mode == GameMode::Creative;
        let mut player = PlayerMotion::new(self.common.motion, Vec3::new(at[0], at[1], at[2]))?;
        player.spectator = self.common.mode == GameMode::Spectator;
        self.common.player = Some(player);
        if s.trace_events {
            self.common.previous_position = Vec3::new(at[0], at[1], at[2]);
        }
        self.server_clear_acting_input();
        // The oracle sets the new player's shift state for every scenario.
        self.server_set_acting_sneak(s.sneak);
        self.server.sculk.tracker = WardenSpawnTracker::default();
        if let Some(level) = s.warning {
            self.server.sculk.tracker.set_warning_level(level);
        }
        // The blocks, in order (sculk block entities tick in the order they
        // were placed), with `Level.setBlock(pos, state, 2 | 16)` at game time 1000.
        self.common.world.game_time = 1000;
        let mut cells = Vec::new();
        for b in &s.blocks {
            let n = |i: usize| b[i].parse::<i32>().unwrap_or(0);
            if b[0] == "block" {
                cells.push((BlockPos::new(n(1), n(2), n(3)), self.fixture_state(&b[4])?));
            } else {
                let state = self.fixture_state(&b[7])?;
                for x in n(1).min(n(4))..=n(1).max(n(4)) {
                    for y in n(2).min(n(5))..=n(2).max(n(5)) {
                        for z in n(3).min(n(6))..=n(3).max(n(6)) {
                            cells.push((BlockPos::new(x, y, z), state));
                        }
                    }
                }
            }
        }
        self.with_level(|b, level, ticks| {
            for (pos, state) in cells {
                b.set_block(level, ticks, pos, state, 2 | 16);
            }
        });
        self.common.level_events.clear();
        self.server.sculk.known = self.common.world.chunks.keys().copied().collect();
        self.server.sculk.known_revision = Some(self.common.world.chunks.membership_revision());
        self.sculk_sync_index();
        let mut names: HashMap<u32, String> = HashMap::new();
        let mut sources: Vec<u32> = Vec::new();
        for spec in &s.mobs {
            let kind = self
                .common
                .res
                .mob_types()
                .get(&spec.kind)
                .cloned()
                .ok_or_else(|| format!("unknown mob {}", spec.kind))?;
            let id = 100_000 + sources.len() as u32;
            let mut m = Mob::new(
                id,
                kind,
                Vec3::new(spec.pos[0], spec.pos[1], spec.pos[2]),
                spec.yaw,
            );
            for flag in &spec.flags {
                match flag.as_str() {
                    "persistent" => m.persistent = true,
                    "noai" => m.no_ai = true,
                    _ => {}
                }
            }
            m.random.set_seed(spec.seed);
            // EntityType.create equips nothing (that is finalizeSpawn's job).
            for slot in sunrise_items::equipment::MAINHAND..=sunrise_items::equipment::HEAD {
                m.extra.set_slot_stack(slot, None);
            }
            if s.trace_events {
                sunrise_sim::mobs::warden::finalize(&mut m, false, &mut Vec::new());
            }
            names.insert(id, format!("m{}", sources.len()));
            sources.push(id);
            self.common.mobs.push(m);
        }
        self.server.next_entity = self.server.next_entity.max(200_000);
        self.common.world.day_time = 6000;
        self.common.world.random.set_seed(s.level_seed);
        self.start_audio_capture();
        let mut known: HashSet<u32> = self.common.mobs.iter().map(|m| m.id).collect();
        let mut seeded: HashSet<u32> = HashSet::new();
        let mut known_items = self.common.dropped.len();
        let mut known_orbs = self.common.orbs.len();
        let mut last: HashMap<[i32; 3], String> = HashMap::new();
        let mut region_last: HashMap<[i32; 3], String> = HashMap::new();
        let (mut last_player, mut last_rng) = (String::new(), String::new());
        let mut out = String::new();
        self.server.lan_entity_events.clear();
        for t in 0..=s.ticks {
            dispense_fixture::start_recording();
            if t > 0 {
                self.sculk_fixture_tick(t, s, &sources, &known, names.len(), &mut seeded)?;
            }
            let mut lines: Vec<String> = Vec::new();
            for p in &s.watch {
                let line = self.sculk_describe(*p, &names);
                if last.get(p) != Some(&line) {
                    last.insert(*p, line.clone());
                    lines.push(format!("blk {},{},{} {line}", p[0], p[1], p[2]));
                }
            }
            // Regions: every block whose state changed.
            for r in &s.regions {
                for x in r[0]..=r[3] {
                    for y in r[1]..=r[4] {
                        for z in r[2]..=r[5] {
                            let state = self.fixture_state_text(self.common.world.block(x, y, z));
                            let before = region_last.insert([x, y, z], state.clone());
                            if t > 0 && before.as_ref() != Some(&state) {
                                lines.push(format!("set {x},{y},{z} {state}"));
                            }
                        }
                    }
                }
            }
            let tr = self.server.sculk.tracker;
            let dark = self
                .common
                .potions
                .effects
                .get("darkness")
                .map_or("-".to_owned(), |e| e.duration.to_string());
            let now = format!(
                "warn {} {} dark {dark}",
                tr.warning_level, tr.cooldown_ticks
            );
            if now != last_player {
                last_player = now.clone();
                lines.push(now);
            }
            let mut fresh: Vec<&Mob> = self
                .common
                .mobs
                .iter()
                .filter(|m| !known.contains(&m.id))
                .collect();
            fresh.sort_by_key(|m| m.id);
            let fresh: Vec<(u32, String)> = fresh
                .iter()
                .map(|m| {
                    (
                        m.id,
                        format!(
                            "new {} {},{},{}",
                            m.kind.id,
                            d(m.position.x),
                            d(m.position.y),
                            d(m.position.z)
                        ),
                    )
                })
                .collect();
            for (id, line) in fresh {
                known.insert(id);
                names.insert(id, format!("n{}", names.len()));
                let seed = 0x5eed + names.len() as i64;
                if seeded.insert(id)
                    && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
                {
                    seed_fresh(m, seed);
                }
                lines.push(line);
            }
            for i in known_items..self.common.dropped.len() {
                let d = &self.common.dropped[i];
                names.insert(u32::MAX - i as u32, format!("n{}", names.len()));
                lines.push(format!(
                    "new minecraft:item {},{},{}",
                    self::d(d.position.x),
                    self::d(d.position.y),
                    self::d(d.position.z)
                ));
            }
            known_items = self.common.dropped.len();
            // Where an orb flies comes from its own random: its value only.
            for orb in &self.common.orbs[known_orbs.min(self.common.orbs.len())..] {
                lines.push(format!("new minecraft:experience_orb value={}", orb.value));
            }
            known_orbs = self.common.orbs.len();
            if s.trace_events {
                for m in &self.common.mobs {
                    if let Some(st) = &m.extra.warden {
                        let selected = st.vibration.selector.current().map(|(i, _)| {
                            serde_json::json!({"event": i.event, "pos": i.pos, "distance": i.distance})
                        });
                        lines.push(format!(
                            "listener {}",
                            serde_json::json!({"selector": selected})
                        ));
                    }
                }
                for p in &self.common.creatures.projectiles {
                    lines.push(format!(
                        "projectile {}",
                        serde_json::json!({"kind": p.kind.id(),
                        "pos": [p.position.x, p.position.y, p.position.z]})
                    ));
                }
                for (_, event) in self.server.lan_entity_events.drain(..) {
                    lines.push(format!("entity_event {event}"));
                }
            }
            for line in dispense_fixture::stop_recording() {
                if let Some(rest) = line.strip_prefix("L ") {
                    lines.push(format!("level {rest}"));
                } else if s.trace_events
                    && let Some(rest) = line.strip_prefix("G ")
                {
                    lines.push(format!("game {rest}"));
                }
            }
            // The sounds bound to an entity are the server's too.
            let bound = std::mem::take(&mut self.server.bound_sounds)
                .into_iter()
                .map(|s| (s.name, s.at, s.volume, s.pitch));
            for (sound, at, volume, pitch) in std::mem::take(&mut self.server.pending_sounds)
                .into_iter()
                .map(|s| (s.event, s.position, s.volume, s.pitch))
                .chain(bound)
            {
                if CLIENT_SOUNDS.contains(&sound.as_str()) {
                    continue;
                }
                lines.push(format!(
                    "sound {sound} {},{},{} {} {}",
                    at[0].floor() as i32,
                    at[1].floor() as i32,
                    at[2].floor() as i32,
                    f(volume),
                    f(pitch)
                ));
            }
            let rng = format!("rng {:x}", self.world.random.state());
            if rng != last_rng {
                last_rng = rng.clone();
                lines.push(rng);
            }
            for line in lines {
                let _ = writeln!(out, "{t} {line}");
            }
        }
        self.stop_audio_capture();
        Ok(out)
    }

    /// One `ServerLevel.tick` of a scenario.
    pub(in crate::game::server_side) fn sculk_fixture_tick(
        &mut self,
        t: i32,
        s: &Scenario,
        sources: &[u32],
        known: &HashSet<u32>,
        named: usize,
        seeded: &mut HashSet<u32>,
    ) -> Result<()> {
        self.common.world.game_time += 1;
        // Scheduled block and fluid ticks, then block events.
        self.tick_scheduled()?;
        self.with_level(|b, level, ticks| b.run_block_events(level, ticks));
        self.server_process_level_events()?;
        // The scenario's actions happen among the entities.
        for (tick, words) in &s.actions {
            if *tick != t {
                continue;
            }
            match words[0].as_str() {
                "summon_snowball" => {
                    let owner = self
                        .acting_player_uuid()
                        .iter()
                        .map(i32::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    let nbt = words[1..]
                        .join(" ")
                        .replace("$owner", &format!("[I;{owner}]"));
                    let before = self.common.creatures.projectiles.len();
                    self.summon_with_nbt("minecraft:snowball", [30.5, 205.0, 10.5], &nbt)
                        .map_err(|_| "fixture summon failed")?;
                    if self.common.creatures.projectiles.len() != before + 1 {
                        return Err("fixture summon created no projectile".into());
                    }
                }
                "throw_snowball" => {
                    self.throw_item(&sunrise_items::items::ItemStack::new(
                        "minecraft:snowball",
                        1,
                    ))?;
                }
                "mob_snowball" => {
                    let owner = *sources.first().ok_or("mob_snowball needs a mob")?;
                    let mut p = sunrise_sim::projectiles::Projectile::new(
                        sunrise_sim::projectiles::ProjectileKind::Snowball,
                        Vec3::new(4.5, 201.5, 10.5),
                        Some(Who::Mob(owner)),
                    );
                    p.velocity = Vec3::new(0.0, 0.0, 1.5);
                    self.handle_mob_events(vec![sunrise_sim::mobs::MobEvent::Projectile(p)])?;
                }
                "event" => {
                    let event = sunrise_level::sculk::intern(&words[1])
                        .ok_or_else(|| format!("unknown game event {}", words[1]))?;
                    let num = |i: usize| words[i].parse::<f64>().unwrap_or(0.0);
                    let pos = [num(2), num(3), num(4)];
                    let who = match words[5].as_str() {
                        "none" => None,
                        "player" => Some(Who::Player),
                        m => sources
                            .get(m[1..].parse::<usize>().unwrap_or(usize::MAX))
                            .map(|id| Who::Mob(*id)),
                    };
                    let state = match words.get(6) {
                        Some(text) => Some(self.fixture_state(text)?),
                        None => None,
                    };
                    let ctx = self.who_context(who, state);
                    self.post_game_event(event, pos, ctx);
                }
                "kill" => {
                    // LivingEntity.hurtServer with a player attack.
                    let id = sources
                        .get(words[1][1..].parse::<usize>().unwrap_or(usize::MAX))
                        .copied()
                        .ok_or("kill: no such mob")?;
                    let amount = words
                        .get(2)
                        .and_then(|a| a.parse::<f32>().ok())
                        .unwrap_or(1000.0);
                    let at = self
                        .common
                        .player
                        .as_ref()
                        .map(|p| [p.position.x, p.position.y, p.position.z])
                        .unwrap_or_default();
                    let attack = self.attack_of_who(Some(sunrise_sim::mobs::Who::Player));
                    let plan = self.prepare_knockback_credit(
                        id as i32,
                        Some(sunrise_sim::mobs::Who::Player),
                        &attack,
                    );
                    let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                    if let Some(index) = self.common.mob_index.find(&self.common.mobs, id) {
                        self.common.mobs[index].damage(
                            crate::game::server_side::entity_server::damage_level_of!(
                                self,
                                &attack,
                                &mut credit,
                                None
                            ),
                            amount,
                            sunrise_sim::mobs::Damage::player(at),
                        );
                        self.remember_mob_player_credit(id);
                        self.finish_mob_damage_at(index);
                    }
                }
                // Creeper.ignite: the next tick primes the fuse.
                "ignite" | "ate" => {
                    let id = sources
                        .get(words[1][1..].parse::<usize>().unwrap_or(usize::MAX))
                        .copied()
                        .ok_or("no such mob")?;
                    let mut events = Vec::new();
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                        if words[0] == "ignite" {
                            m.extra.flag = true;
                        } else {
                            // Mob.ate.
                            sunrise_sim::mobs::ai::ate(m, &mut events);
                        }
                    }
                    self.handle_mob_events(events)?;
                }
                "set" => {
                    let n = |i: usize| words[i].parse::<i32>().unwrap_or(0);
                    let pos = BlockPos::new(n(1), n(2), n(3));
                    let state = self.fixture_state(&words[4])?;
                    self.with_level(|b, level, ticks| {
                        b.set_block(level, ticks, pos, state, 3);
                    });
                    self.server_process_level_events()?;
                }
                other => return Err(format!("unknown action {other}").into()),
            }
        }
        // Mobs that appeared during the block ticks (a summoned warden) tick
        // in this tick's entity phase: seed them now, as they are seeded
        // after the tick.
        let mut born: Vec<u32> = self
            .common
            .mobs
            .iter()
            .map(|m| m.id)
            .filter(|id| !known.contains(id))
            .collect();
        born.sort_unstable();
        for (i, id) in born.into_iter().enumerate() {
            if seeded.insert(id)
                && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
            {
                seed_fresh(m, 0x5eed + (named + i + 1) as i64);
            }
        }
        // The player's tickets, as `tick_world` takes them.
        self.update_chunk_tickets();
        // Entities: the player's warden spawn tracker, then the mobs.
        self.tick_sculk_entities();
        // (ServerChunkCache.tick, natural spawning and its chunk shuffle, is not run.)
        self.tick_creature_entities()?;
        self.tick_items();
        // Block entities.
        self.tick_sculk_blocks()?;
        Ok(())
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
