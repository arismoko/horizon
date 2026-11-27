//! Build a complete immutable builtin generation alongside its common-store values.
use crate::game::ServerGame;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use sunrise_core::{identifier::Identifier, nbt::Tag};
use sunrise_gameplay_common::game::native_data_maps_common::Loaded;
use sunrise_sim::neoforge_mob_maps::{self as builtin, BuiltinMap, Kind, MAPS, NeoForgeMobMaps};

type Tags = HashMap<String, HashSet<String>>;

struct Loader<'a, 'p> {
    definition: BuiltinMap,
    id: Identifier,
    overlay: &'a sunrise_world::loot::PackOverlay<'p>,
    tags: &'a Tags,
    known: &'a dyn Fn(&str, &str) -> bool,
    snapshot: &'a mut NeoForgeMobMaps,
}
impl Loader<'_, '_> {
    fn install_and_encode<T>(
        &mut self,
        values: BTreeMap<String, T>,
        applied: bool,
        install: impl FnOnce(&mut NeoForgeMobMaps, BTreeMap<String, T>),
        encode: impl Fn(&T) -> Tag,
    ) -> BTreeMap<String, Tag> {
        let field = self.definition.kind.field();
        let encoded = values
            .iter()
            .map(|(id, value)| (id.clone(), Tag::compound([(field, encode(value))])))
            .collect();
        if applied {
            install(self.snapshot, values);
        }
        encoded
    }
    fn merge<T: Clone>(
        &mut self,
        codec: impl Fn(&Value) -> Result<T, String>,
        install: impl FnOnce(&mut NeoForgeMobMaps, BTreeMap<String, T>),
        encode: impl Fn(&T) -> Tag,
    ) -> Result<BTreeMap<String, Tag>, String> {
        let definition = self.definition;
        let base = builtin::vanilla_layer(definition.kind)
            .into_iter()
            .map(|(id, raw)| {
                codec(&raw)
                    .map(|value| (id, value))
                    .map_err(|error| format!("{} vanilla seed: {error}", definition.id))
            })
            .collect::<Result<_, _>>()?;
        let tags = self.tags;
        let (values, applied) = super::data_map_files::load(
            &self.id,
            definition.registry,
            base,
            self.overlay,
            move |tag| tags.get(tag).into_iter().flatten(),
            |raw| codec(super::data_map_files::entry_value(raw)?),
            |id| (self.known)(definition.registry, id),
        );
        Ok(self.install_and_encode(values, applied, install, encode))
    }
    fn merge_number<T: Clone>(
        &mut self,
        codec: impl Fn(&serde_json::value::RawValue) -> Result<T, String>,
        install: impl FnOnce(&mut NeoForgeMobMaps, BTreeMap<String, T>),
        encode: impl Fn(&T) -> Tag,
    ) -> Result<BTreeMap<String, Tag>, String> {
        let definition = self.definition;
        let field = definition.kind.field();
        let base = builtin::vanilla_layer(definition.kind)
            .into_iter()
            .map(|(id, raw)| {
                let number = serde_json::value::RawValue::from_string(raw[field].to_string())
                    .expect("static JSON number");
                codec(&number)
                    .map(|value| (id, value))
                    .map_err(|error| format!("{} vanilla seed: {error}", definition.id))
            })
            .collect::<Result<_, _>>()?;
        let tags = self.tags;
        let (values, applied) = super::data_map_files::load_numeric(
            &self.id,
            definition.registry,
            base,
            self.overlay,
            move |tag| tags.get(tag).into_iter().flatten(),
            super::data_map_files::NumberCodec {
                field,
                decode: codec,
            },
            |id| (self.known)(definition.registry, id),
        );
        Ok(self.install_and_encode(values, applied, install, encode))
    }
}
pub(in crate::game::server_side) fn load<'a>(
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    tags: impl Fn(&str) -> Option<&'a Tags>,
    known: impl Fn(&str, &str) -> bool,
) -> Result<Loaded, String> {
    let none = Tags::new();
    let mut snapshot = NeoForgeMobMaps::default();
    let mut values = Vec::with_capacity(MAPS.len());
    for definition in MAPS {
        let mut loader = Loader {
            definition,
            id: Identifier::parse(definition.id).expect("built-in data map ID"),
            overlay,
            tags: tags(definition.directory).unwrap_or(&none),
            known: &known,
            snapshot: &mut snapshot,
        };
        let encoded = match definition.kind {
            Kind::VillagerDistances => loader.merge_number(
                builtin::decode_distance,
                NeoForgeMobMaps::set_distances,
                |value| Tag::Float(value.to_bits()),
            ),
            Kind::ParrotImitations => loader.merge(
                |value| builtin::decode_sound(value, |id| known("minecraft:sound_event", id)),
                NeoForgeMobMaps::set_parrots,
                |sound| {
                    Tag::String(if sound.is_empty() {
                        "minecraft:intentionally_empty".into()
                    } else {
                        sunrise_sim::mobs::entity_sounds::full_id(sound)
                            .as_ref()
                            .into()
                    })
                },
            ),
            Kind::VillagerTypes => loader.merge(
                builtin::decode_villager_type,
                NeoForgeMobMaps::set_villager_types,
                |value| {
                    let id = format!(
                        "minecraft:{}",
                        sunrise_sim::mobs::villager::VILLAGER_TYPES[*value as usize]
                    );
                    Tag::String(id.as_str().into())
                },
            ),
            Kind::RaidHeroGifts => {
                loader.merge(builtin::decode_gift, NeoForgeMobMaps::set_gifts, |value| {
                    Tag::String(value.as_str().into())
                })
            }
            Kind::VibrationFrequencies => loader.merge_number(
                builtin::decode_frequency,
                NeoForgeMobMaps::set_frequencies,
                |value| Tag::Int(*value),
            ),
            Kind::VillagerCompostables => loader.merge(
                builtin::decode_compostable,
                NeoForgeMobMaps::set_compostables,
                |value| Tag::Byte(i8::from(*value)),
            ),
        };
        values.push((definition, encoded?));
    }
    Ok(Loaded {
        values,
        snapshot: Arc::new(snapshot),
    })
}
impl ServerGame<'_> {
    /// Generation workers pin one world-owned snapshot per finalization, not
    /// a process-global last-loaded world. In-flight work retains its snapshot.
    pub(in crate::game::server_side) fn share_mob_maps(&self) {
        if !self.common.options.execution_role.is_logical_server() {
            return;
        }
        let snapshot = self.mob_map_snapshot();
        for terrain in self
            .common
            .end
            .locator()
            .iter()
            .chain(self.server.levels.0.parked_terrains())
        {
            terrain.set_mob_maps(snapshot);
        }
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invalid_values_refuse_replace_and_removals_before_unknown_target_filtering() {
        let files = vec![("data_maps/neoforge:entity_type/acceptable_villager_distances".into(), vec![
            serde_json::to_vec(&json!({"values": {"test:wraith": {"acceptable_villager_distance": 21}}})).unwrap(),
            serde_json::to_vec(&json!({"replace": true, "values": {"test:unknown": {"acceptable_villager_distance": "bad"}}, "remove": ["minecraft:zombie", "test:wraith"]})).unwrap(),
            serde_json::to_vec(&json!({"values": {"test:unknown": {"acceptable_villager_distance": 40}}, "remove": ["test:absent"]})).unwrap(),
        ])];
        let loaded = load(
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            |_| None,
            |_, id| id.starts_with("minecraft:") || id == "test:wraith",
        )
        .unwrap();
        assert_eq!(loaded.snapshot.villager_distance("test:wraith"), Some(21.0));
        let values = &loaded
            .values
            .iter()
            .find(|(definition, _)| definition.id == "neoforge:acceptable_villager_distances")
            .unwrap()
            .1;
        assert_eq!(
            values.get("test:wraith"),
            Some(&Tag::compound([(
                "acceptable_villager_distance",
                Tag::Float(21.0f32.to_bits())
            )]))
        );
        assert_eq!(
            values.get("minecraft:zombie"),
            Some(&Tag::compound([(
                "acceptable_villager_distance",
                Tag::Float(8.0f32.to_bits())
            )]))
        );
        assert!(!values.contains_key("test:unknown"));
    }

    #[test]
    fn lexical_numeric_files_keep_infinity_and_exact_decimal_truncation_in_both_views() {
        let files = vec![
            ("data_maps/neoforge:entity_type/acceptable_villager_distances".into(), vec![br#"{"values": {
                "test:overflow": {"value": {"acceptable_villager_distance": 1e500}},
                "test:negative": -3, "test:zero": -0.0,
                "test:disabled": {"neoforge:conditions": [{"type":"neoforge:mod_loaded","modid":"absent"}], "value":"bad"}
            }}"#.to_vec()]),
            ("data_maps/neoforge:game_event/vibration_frequencies".into(), vec![br#"{"values": {
                "test:wrap": 4294967297, "test:top": {"frequency":4294967311},
                "test:decimal": {"frequency":15.999999999999999999999}
            }}"#.to_vec()]),
        ];
        let loaded = load(
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            |_| None,
            |_, _| true,
        )
        .unwrap();
        assert_eq!(
            loaded.snapshot.villager_distance("test:overflow"),
            Some(f32::INFINITY)
        );
        assert_eq!(
            loaded.snapshot.villager_distance("test:negative"),
            Some(-3.0)
        );
        assert_eq!(
            loaded
                .snapshot
                .villager_distance("test:zero")
                .unwrap()
                .to_bits(),
            0x8000_0000
        );
        assert_eq!(loaded.snapshot.villager_distance("test:disabled"), None);
        assert_eq!(loaded.snapshot.vibration_frequency("test:wrap"), 1);
        assert_eq!(loaded.snapshot.vibration_frequency("test:top"), 15);
        assert_eq!(loaded.snapshot.vibration_frequency("test:decimal"), 15);
        let tables: HashMap<_, _> = loaded
            .values
            .into_iter()
            .map(|(definition, values)| (definition.id, values))
            .collect();
        let distances = &tables["neoforge:acceptable_villager_distances"];
        assert_eq!(
            distances.get("test:overflow"),
            Some(&Tag::compound([(
                "acceptable_villager_distance",
                Tag::Float(f32::INFINITY.to_bits())
            )]))
        );
        let frequencies = &tables["neoforge:vibration_frequencies"];
        assert_eq!(
            frequencies.get("test:decimal"),
            Some(&Tag::compound([("frequency", Tag::Int(15))]))
        );
    }

    #[test]
    fn profession_and_event_tags_use_their_own_registry_and_unknown_sounds_refuse_files() {
        let files: Vec<_> = [
            ("data_maps/neoforge:villager_profession/raid_hero_gifts", json!({"values": {"#test:workers": {"loot_table": "test:gifts"}}})),
            ("data_maps/neoforge:game_event/vibration_frequencies", json!({"values": {"#test:steps": {"frequency": 7.9}}})),
            ("data_maps/neoforge:entity_type/parrot_imitations", json!({"replace": true, "values": {"test:unknown": {"sound": "test:missing"}}, "remove": ["minecraft:zombie"]})),
        ].into_iter().map(|(path, value)| (path.to_owned(), vec![serde_json::to_vec(&value).unwrap()])).collect();
        let professions = HashMap::from([(
            "test:workers".into(),
            HashSet::from(["test:artisan".into()]),
        )]);
        let events = HashMap::from([("test:steps".into(), HashSet::from(["test:rustle".into()]))]);
        let loaded = load(
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            |directory| match directory {
                "villager_profession" => Some(&professions),
                "game_event" => Some(&events),
                _ => None,
            },
            |_, id| id.starts_with("minecraft:") || matches!(id, "test:artisan" | "test:rustle"),
        )
        .unwrap();
        assert_eq!(
            loaded.snapshot.hero_gift("test:artisan"),
            Some("test:gifts".into())
        );
        assert_eq!(loaded.snapshot.vibration_frequency("test:rustle"), 7);
        assert!(loaded.snapshot.parrot_sounds().is_none());
        assert_eq!(
            loaded.snapshot.parrot_imitation("minecraft:zombie"),
            Some("entity.parrot.imitate.zombie")
        );
    }
}
