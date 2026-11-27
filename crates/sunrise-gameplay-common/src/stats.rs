//! `ServerStatsCounter`: a player's statistics, their dirty set and their JSON file.
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    path::{Path, PathBuf},
};
use sunrise_core::{identifier::Identifier, uuid::uuid_text};
use sunrise_world::datafix::json::{Json, migrate_stats};

/// `stats[type][key]`, with `type` and `key` namespaced ids.
pub type StatValues = BTreeMap<String, BTreeMap<String, i32>>;

/// Operations applied in order to the player's statistic-criterion scores.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatScoreOp {
    Add(i32),
    Set(i32),
}

/// A statistic change identifies its type, entry and score operation.
pub type StatScoreChange = (String, String, StatScoreOp);

/// The player's live counters and pending updates.
#[derive(Clone, Default)]
pub struct Stats {
    pub values: StatValues,
    pub loaded: bool,
    /// Ticks until the phantom spawner runs again.
    phantom_tick: i32,
    /// Pending statistic changes for the scoreboard criteria to follow.
    pub events: Vec<StatScoreChange>,
    /// Changed since the client was last sent them (`StatsCounter`'s dirty
    /// set), by type.
    dirty: BTreeMap<String, BTreeSet<String>>,
}

/// `minecraft:` and `name`, given to `f` from the stack when short: some
/// counters go up every tick for every player, and they are found without
/// a string made for each.
pub fn namespaced<T>(name: &str, f: impl FnOnce(&str) -> T) -> T {
    const NAMESPACE: &str = "minecraft:";
    let mut buf = [0u8; 96];
    let len = NAMESPACE.len() + name.len();
    if len > buf.len() {
        return f(&format!("{NAMESPACE}{name}"));
    }
    buf[..NAMESPACE.len()].copy_from_slice(NAMESPACE.as_bytes());
    buf[NAMESPACE.len()..len].copy_from_slice(name.as_bytes());
    // Two strings one after the other are one.
    f(std::str::from_utf8(&buf[..len]).unwrap_or_default())
}

/// The map under `key`, made when missing, found without an owned key
/// when there.
fn entry<'a, V: Default>(map: &'a mut BTreeMap<String, V>, key: &str) -> &'a mut V {
    if !map.contains_key(key) {
        map.insert(key.to_owned(), V::default());
    }
    map.get_mut(key).expect("just inserted")
}

impl Stats {
    pub fn add(&mut self, ty: &str, key: &str, n: i32) {
        namespaced(ty, |ty| {
            let v = entry(entry(&mut self.values, ty), key);
            *v = v.saturating_add(n);
            self.events
                .push((ty.to_owned(), key.to_owned(), StatScoreOp::Add(n)));
            self.mark_dirty(ty, key);
        });
    }
    fn mark_dirty(&mut self, ty: &str, key: &str) {
        let keys = entry(&mut self.dirty, ty);
        if !keys.contains(key) {
            keys.insert(key.to_owned());
        }
    }
    pub fn get(&self, ty: &str, key: &str) -> i32 {
        self.values
            .get(&format!("minecraft:{ty}"))
            .and_then(|m| m.get(key))
            .copied()
            .unwrap_or(0)
    }
    pub fn reset(&mut self, ty: &str, key: &str) {
        if let Some(m) = self.values.get_mut(&format!("minecraft:{ty}")) {
            m.insert(key.to_owned(), 0);
        }
        self.mark_dirty(&format!("minecraft:{ty}"), key);
        self.events.push((
            format!("minecraft:{ty}"),
            key.to_owned(),
            StatScoreOp::Set(0),
        ));
    }
    /// `ServerPlayer.die`'s `resetStat` of `TIME_SINCE_DEATH` and `TIME_SINCE_REST`.
    pub fn reset_death_timers(&mut self) {
        self.reset("custom", "minecraft:time_since_death");
        self.reset("custom", "minecraft:time_since_rest");
    }
    pub fn custom(&mut self, key: &str, n: i32) {
        namespaced(key, |key| self.add("custom", key, n));
    }
    /// `ServerStatsCounter.markAllDirty`: everything goes out on the next request.
    pub fn mark_all_dirty(&mut self) {
        self.dirty = self
            .values
            .iter()
            .map(|(ty, m)| (ty.clone(), m.keys().cloned().collect()))
            .collect();
    }
    /// `ServerStatsCounter.sendStats`' entries: the dirty statistics and
    /// their values, the set cleared.
    pub fn take_dirty(&mut self) -> Vec<(String, String, i32)> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .flat_map(|(ty, keys)| keys.into_iter().map(move |key| (ty.clone(), key)))
            .map(|(ty, key)| {
                let v = self.values.get(&ty).and_then(|m| m.get(&key)).copied();
                (ty, key, v.unwrap_or(0))
            })
            .collect()
    }
    /// `ServerStatsCounter.parseLocal`: the counters of a stats file.
    pub fn read(path: &Path) -> Self {
        let json = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .unwrap_or_default();
        Self::from_json(&json)
    }
    /// The counters of a stats file's JSON, or of a hand-over's record (`player-import`).
    pub fn from_json(json: &serde_json::Value) -> Self {
        let mut stats = Self {
            loaded: true,
            ..Self::default()
        };
        // Both keys decode as `Identifier`s, as in a restore, so the live map holds only the
        // namespaced spelling that reads, criteria and packets use.
        if let Some(all) = json.get("stats").and_then(|s| s.as_object()) {
            for (ty, entries) in all {
                let (Ok(ty), Some(entries)) = (Identifier::parse(ty), entries.as_object()) else {
                    continue;
                };
                let map = stats.values.entry(ty.to_string()).or_default();
                for (key, v) in entries {
                    if let (Ok(key), Some(n)) = (Identifier::parse(key), v.as_i64()) {
                        map.entry(key.to_string()).or_insert(n as i32);
                    }
                }
            }
        }
        stats
    }
    /// `ServerStatsCounter.parse`: restore candidates from the save format, using
    /// the live registries to decode both map keys. Malformed documents or a
    /// non-map `stats` have no result; map errors retain recoverable entries.
    pub fn decode_snapshot(
        text: &str,
        known: impl Fn(&str, &Identifier) -> bool,
    ) -> Option<StatValues> {
        let Some(root) = migrate_stats(text) else {
            log::warn!("Could not read or migrate statistics snapshot");
            return None;
        };
        let mut values = StatValues::new();
        let Some(groups) = root.get("stats") else {
            return Some(values);
        };
        let Json::Obj(groups) = groups else {
            log::warn!("Could not decode statistics snapshot: 'stats' is not a map");
            return None;
        };
        for (raw_ty, group) in groups {
            let raw_ty = String::from_utf16_lossy(raw_ty);
            let Ok(id) = Identifier::parse(&raw_ty) else {
                log::warn!("Ignoring invalid statistics snapshot type {raw_ty:?}");
                continue;
            };
            let ty = id.to_string();
            let Some(registry) = stat_registry(&ty).filter(|_| known("minecraft:stat_type", &id))
            else {
                log::warn!("Ignoring unknown statistics snapshot type {ty:?}");
                continue;
            };
            let Json::Obj(entries) = group else {
                log::warn!("Ignoring non-map statistics snapshot type {ty:?}");
                continue;
            };
            let Entry::Vacant(group) = values.entry(ty.clone()) else {
                log::warn!("Ignoring duplicate statistics snapshot type {ty:?}");
                continue;
            };
            let mut counts = BTreeMap::new();
            for (raw_key, count) in entries {
                let raw_key = String::from_utf16_lossy(raw_key);
                let Ok(id) = Identifier::parse(&raw_key) else {
                    log::warn!("Ignoring invalid statistics snapshot entry {ty:?}/{raw_key:?}");
                    continue;
                };
                let key = id.to_string();
                if !known(registry, &id) {
                    log::warn!("Ignoring unknown statistics snapshot entry {ty:?}/{key:?}");
                    continue;
                }
                let Some(count) = count.as_i32() else {
                    log::warn!("Ignoring invalid statistics snapshot count for {ty:?}/{key:?}");
                    continue;
                };
                match counts.entry(key) {
                    Entry::Vacant(entry) => {
                        entry.insert(count);
                    }
                    Entry::Occupied(entry) => log::warn!(
                        "Ignoring duplicate statistics snapshot entry {ty:?}/{:?}",
                        entry.key()
                    ),
                }
            }
            group.insert(counts);
        }
        Some(values)
    }

    /// Replace the map without awarding stats or disturbing pending changes.
    /// Removed keys still go out with count zero on the next statistics request.
    pub fn replace_values(&mut self, values: StatValues) {
        let old = std::mem::take(&mut self.values);
        let mut record = |ty: &str, key: &str, before: Option<i32>, after: Option<i32>| {
            if before != after {
                self.mark_dirty(ty, key);
            }
            let (before, after) = (before.unwrap_or(0), after.unwrap_or(0));
            if before != after {
                self.events
                    .push((ty.to_owned(), key.to_owned(), StatScoreOp::Set(after)));
            }
        };
        for (ty, counts) in &old {
            let new = values.get(ty);
            for (key, &before) in counts {
                record(
                    ty,
                    key,
                    Some(before),
                    new.and_then(|new| new.get(key)).copied(),
                );
            }
        }
        for (ty, counts) in &values {
            let previous = old.get(ty);
            for (key, &after) in counts {
                if previous.is_none_or(|previous| !previous.contains_key(key)) {
                    record(ty, key, None, Some(after));
                }
            }
        }
        self.values = values;
        self.loaded = true;
    }

    /// `ServerStatsCounter.toJson`: what `write` puts in the file, and what a
    /// hand-over's record carries (`player-export`).
    pub fn to_json(&self) -> serde_json::Value {
        let stats: serde_json::Map<String, serde_json::Value> = self
            .values
            .iter()
            .filter(|(_, m)| !m.is_empty())
            .map(|(ty, m)| {
                let entries: serde_json::Map<String, serde_json::Value> =
                    m.iter().map(|(k, v)| (k.clone(), (*v).into())).collect();
                (ty.clone(), serde_json::Value::Object(entries))
            })
            .collect();
        serde_json::json!({
            "stats": stats,
            "DataVersion": sunrise_world::DATA_VERSION,
        })
    }
    /// Counts the phantom spawner's timer down one tick and returns what is left.
    pub fn phantom_tick_down(&mut self) -> i32 {
        self.phantom_tick -= 1;
        self.phantom_tick
    }
    /// Adds `ticks` to the phantom spawner's timer.
    pub fn phantom_tick_add(&mut self, ticks: i32) {
        self.phantom_tick += ticks;
    }
    /// Sets the phantom spawner's timer.
    pub fn set_phantom_tick(&mut self, ticks: i32) {
        self.phantom_tick = ticks;
    }
    /// `ServerStatsCounter.save`.
    pub fn write(&self, path: &Path) {
        let json = self.to_json();
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = sunrise_sim::save::write_durable(path, text.as_bytes());
        }
    }
}
/// `players/<dir>/<uuid>.json` of a world (advancements, stats).
pub fn player_json(world: &Path, dir: &str, uuid: [i32; 4]) -> PathBuf {
    let text = uuid_text(uuid);
    world.join("players").join(dir).join(format!("{text}.json"))
}
/// `FileUtil.isValidPathSegment`: not `.` or `..`, and only the characters
/// `[-._a-z0-9]`. A player name with a capital letter in it is refused like any
/// other segment vanilla would not write to disk.
fn valid_path_segment(segment: &str) -> bool {
    segment != "."
        && segment != ".."
        && segment.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_')
        })
}

/// `PlayerList.locateStatsFile`: the player's stats file, with the one an older
/// version named after the player (`stats/<name>.json`) moved to the uuid file the
/// first time it is asked for. A uuid file that is already there wins, and a move
/// that fails leaves the old one in place.
pub fn locate_player_json(world: &Path, dir: &str, uuid: [i32; 4], name: &str) -> PathBuf {
    let uuid_file = player_json(world, dir, uuid);
    if uuid_file.exists() {
        return uuid_file;
    }
    let file = format!("{name}.json");
    if !valid_path_segment(&file) {
        return uuid_file;
    }
    let named = uuid_file.with_file_name(&file);
    if !named.is_file() {
        return uuid_file;
    }
    match std::fs::rename(&named, &uuid_file) {
        Ok(()) => uuid_file,
        Err(e) => {
            log::warn!(
                "Sunrise: failed to move the {} file {} to {}: {e}",
                dir,
                named.display(),
                uuid_file.display()
            );
            named
        }
    }
}

/// The value registry owned by each vanilla statistic type.
pub fn stat_registry(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "minecraft:mined" => "minecraft:block",
        "minecraft:crafted"
        | "minecraft:used"
        | "minecraft:broken"
        | "minecraft:picked_up"
        | "minecraft:dropped" => "minecraft:item",
        "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
        "minecraft:custom" => "minecraft:custom_stat",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_keeps_registered_counts_from_partial_maps() {
        let json = format!(
            r#"{{"DataVersion":{},"stats":{{
                "custom":{{"jump":-7,"minecraft:jump":55,"walk_one_cm":4294967297.9,
                    "deaths":true,"play_time":"12","total_world_time":null,
                    "time_since_rest":1e10000,"time_since_death":-69e-1,"missing":8}},
                "minecraft:custom":{{"jump":18}},"mined":42,
                "bad:type":{{"minecraft:stone":30}}
            }}}}"#,
            sunrise_world::DATA_VERSION,
        );
        let values = Stats::decode_snapshot(&json, |registry, id| {
            id.namespace() == "minecraft"
                && match registry {
                    "minecraft:stat_type" => matches!(id.path(), "custom" | "mined"),
                    "minecraft:custom_stat" => matches!(
                        id.path(),
                        "jump"
                            | "walk_one_cm"
                            | "deaths"
                            | "play_time"
                            | "total_world_time"
                            | "time_since_rest"
                            | "time_since_death"
                    ),
                    _ => false,
                }
        })
        .unwrap();
        assert_eq!(
            values,
            BTreeMap::from([(
                "minecraft:custom".into(),
                BTreeMap::from([
                    ("minecraft:jump".into(), -7),
                    ("minecraft:time_since_death".into(), -6),
                    ("minecraft:walk_one_cm".into(), 1),
                ]),
            )])
        );
    }

    /// A save's unqualified keys load as their namespaced ids, so a snapshot of the loaded
    /// counter restores the same live count.
    #[test]
    fn a_loaded_unqualified_key_round_trips_through_a_snapshot() {
        let mut stats = Stats::from_json(&serde_json::json!({
            "DataVersion": sunrise_world::DATA_VERSION,
            "stats": {"custom": {"jump": 7}}
        }));
        assert_eq!(stats.get("custom", "minecraft:jump"), 7);
        let snapshot = stats.to_json().to_string();
        let values = Stats::decode_snapshot(&snapshot, |_, _| true).unwrap();
        stats.replace_values(values);
        assert_eq!(stats.get("custom", "minecraft:jump"), 7);
        assert!(stats.events.is_empty());
    }

    #[test]
    fn snapshot_distinguishes_an_empty_map_from_no_result() {
        let current = sunrise_world::DATA_VERSION;
        let read = |text: &str| Stats::decode_snapshot(text, |_, _| false);
        assert_eq!(
            read(&format!(r#"{{"DataVersion":{current}}}"#)),
            Some(StatValues::new())
        );
        assert_eq!(
            read(&format!(
                r#"{{"DataVersion":{current},"stats":{{"unknown":{{}}}}}}"#
            )),
            Some(StatValues::new()),
        );
        assert_eq!(
            read(&format!(r#"{{"DataVersion":{current},"stats":null}}"#)),
            None
        );
        // A grouped legacy document whose only group is named like the boolean marker.
        assert_eq!(
            read(r#"{"DataVersion":2000,"stats":{"\u0000bool":{}}}"#),
            Some(StatValues::new())
        );
        for malformed in [
            r#"[1,2,3]"#,
            r#"{"stats":{}"#,
            r#"{"stats":{},"unrelated":"invalid\'escape"}"#,
            r#"{"stats":{},"unrelated":"\u+001"}"#,
        ] {
            assert_eq!(read(malformed), None);
        }
        assert_eq!(
            read(&format!("\u{feff}{{\"DataVersion\":{current}}}")),
            Some(StatValues::new())
        );
    }

    #[test]
    fn snapshot_replacement_preserves_pending_updates_and_sends_removals() {
        let mut stats = Stats::default();
        stats.add("mined", "minecraft:stone", 6);
        stats.custom("jump", 9);
        stats.custom("deaths", 2);
        stats.set_phantom_tick(20);
        stats.take_dirty();
        stats.events.clear();
        stats.custom("play_time", 1);
        stats.custom("jump", 1);

        stats.replace_values(BTreeMap::from([(
            "minecraft:custom".into(),
            BTreeMap::from([
                ("minecraft:deaths".into(), 2),
                ("minecraft:jump".into(), -7),
                ("minecraft:play_time".into(), 1),
            ]),
        )]));

        assert_eq!(stats.phantom_tick_down(), 19);
        assert_eq!(stats.get("mined", "minecraft:stone"), 0);
        assert_eq!(stats.get("custom", "minecraft:jump"), -7);
        assert_eq!(
            stats.to_json()["stats"],
            serde_json::json!({"minecraft:custom": {
                "minecraft:deaths": 2, "minecraft:jump": -7, "minecraft:play_time": 1
            }})
        );
        assert_eq!(
            stats.take_dirty(),
            [
                ("minecraft:custom".into(), "minecraft:jump".into(), -7),
                ("minecraft:custom".into(), "minecraft:play_time".into(), 1),
                ("minecraft:mined".into(), "minecraft:stone".into(), 0),
            ]
        );
        assert_eq!(
            stats.events,
            [
                (
                    "minecraft:custom".into(),
                    "minecraft:play_time".into(),
                    StatScoreOp::Add(1)
                ),
                (
                    "minecraft:custom".into(),
                    "minecraft:jump".into(),
                    StatScoreOp::Add(1)
                ),
                (
                    "minecraft:custom".into(),
                    "minecraft:jump".into(),
                    StatScoreOp::Set(-7)
                ),
                (
                    "minecraft:mined".into(),
                    "minecraft:stone".into(),
                    StatScoreOp::Set(0)
                ),
            ]
        );
    }

    /// `PlayerList.locateStatsFile`: a stats file an older version named after the player is moved
    /// to the uuid file the first time it is asked for; a uuid file that is already there wins, and
    /// a name vanilla would not write as a path segment is not looked for at all.
    #[test]
    fn an_old_stats_file_named_after_the_player_moves_to_its_uuid() {
        let world = std::env::temp_dir().join(format!(
            "sunrise-stats-file-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let stats = world.join("players").join("stats");
        std::fs::create_dir_all(&stats).unwrap();
        let uuid = [1, 2, 3, 4];
        let located = player_json(&world, "stats", uuid);
        // Nothing to move: the uuid name is the answer, and looking writes nothing.
        assert_eq!(locate_player_json(&world, "stats", uuid, "notch"), located);
        assert!(!located.exists(), "looking for the file writes none");
        // The old name-based file is moved onto the uuid name, and not left behind.
        let old = stats.join("notch.json");
        std::fs::write(&old, "{}").unwrap();
        assert_eq!(locate_player_json(&world, "stats", uuid, "notch"), located);
        assert!(located.is_file(), "the old file landed on the uuid name");
        assert!(!old.exists(), "and is not left behind");
        // A uuid file that exists is the answer, whether or not an old one does.
        std::fs::write(&old, "{}").unwrap();
        assert_eq!(locate_player_json(&world, "stats", uuid, "notch"), located);
        assert!(old.is_file(), "the old file is left alone");
        // FileUtil.isValidPathSegment on the file name: capitals and separators are not a segment,
        // while a lowercase name with dashes, dots or digits is.
        for name in ["Notch", "no/tch", "no tch"] {
            assert!(
                !valid_path_segment(&format!("{name}.json")),
                "{name:?} is not a path segment"
            );
        }
        for name in ["notch", "a-b_c.1", "jeb_"] {
            assert!(
                valid_path_segment(&format!("{name}.json")),
                "{name:?} is a path segment"
            );
        }
        let _ = std::fs::remove_dir_all(&world);
    }

    /// ServerPlayer.die zeroes the two timers (and only them) and sends the new zero.
    #[test]
    fn death_resets_the_timers_since_death_and_rest() {
        let mut s = Stats::default();
        s.custom("time_since_death", 700);
        s.custom("time_since_rest", 900);
        s.custom("deaths", 2);
        s.events.clear();
        s.reset_death_timers();
        assert_eq!(s.get("custom", "minecraft:time_since_death"), 0);
        assert_eq!(s.get("custom", "minecraft:time_since_rest"), 0);
        assert_eq!(s.get("custom", "minecraft:deaths"), 2);
        assert_eq!(s.events.len(), 2);
    }

    #[test]
    fn counters_events_and_dirty_entries() {
        let mut s = Stats::default();
        s.custom("play_time", 1);
        s.custom("play_time", 1);
        s.add("mined", "minecraft:stone", 2);
        s.custom(&"x".repeat(100), 3);
        assert_eq!(s.get("custom", "minecraft:play_time"), 2);
        assert_eq!(s.get("mined", "minecraft:stone"), 2);
        assert_eq!(
            s.get("custom", &format!("minecraft:{}", "x".repeat(100))),
            3
        );
        assert_eq!(s.events.len(), 4);
        assert_eq!(
            s.events[1],
            (
                "minecraft:custom".into(),
                "minecraft:play_time".into(),
                StatScoreOp::Add(1)
            )
        );
        let long = format!("minecraft:{}", "x".repeat(100));
        let sent = |v: &[(&str, &str, i32)]| -> Vec<(String, String, i32)> {
            v.iter()
                .map(|&(t, k, n)| (t.to_owned(), k.to_owned(), n))
                .collect()
        };
        assert_eq!(
            s.take_dirty(),
            sent(&[
                ("minecraft:custom", "minecraft:play_time", 2),
                ("minecraft:custom", &long, 3),
                ("minecraft:mined", "minecraft:stone", 2),
            ])
        );
        assert!(s.take_dirty().is_empty());
        s.reset("mined", "minecraft:stone");
        assert_eq!(
            s.take_dirty(),
            sent(&[("minecraft:mined", "minecraft:stone", 0)])
        );
        s.mark_all_dirty();
        assert_eq!(s.take_dirty().len(), 3);
    }
}
