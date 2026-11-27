use super::*;
use mod_api::{Hooks, Identifier, LootTypeContext, Registrar, RegistrationContext};
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Codec {
    observations: Arc<Mutex<Vec<(String, i32)>>>,
    function_decodes: Arc<AtomicUsize>,
    condition_decodes: Arc<AtomicUsize>,
}
impl Codec {
    fn new(observations: Arc<Mutex<Vec<(String, i32)>>>) -> Self {
        Self {
            observations,
            function_decodes: Arc::new(AtomicUsize::new(0)),
            condition_decodes: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct ScaleRule {
    label: String,
    factor: i32,
    observations: Arc<Mutex<Vec<(String, i32)>>>,
}
impl ModLootFunctionCodec for Codec {
    fn decode(&self, data: &serde_json::Value) -> Result<Arc<dyn ModLootFunction>, String> {
        self.function_decodes.fetch_add(1, Ordering::Relaxed);
        let factor = data["factor"]
            .as_i64()
            .ok_or_else(|| "factor: expected integer".to_owned())? as i32;
        let label = data["label"].as_str().unwrap_or("scale").to_owned();
        Ok(Arc::new(ScaleRule {
            label,
            factor,
            observations: self.observations.clone(),
        }))
    }
}
impl ModLootFunction for ScaleRule {
    fn apply(
        &self,
        stack: &mut mod_api::LootStackEditor<'_>,
        context: &mut dyn mod_api::LootTypeContext,
    ) -> Result<(), String> {
        assert!(context.params().origin().is_none());
        let draw = context.random().next_int_bound(5);
        self.observations
            .lock()
            .unwrap()
            .push((self.label.clone(), draw));
        let count = stack
            .view()
            .count()
            .wrapping_mul(self.factor)
            .wrapping_add(draw);
        stack.set_count(count);
        Ok(())
    }
}

struct GateRule {
    label: String,
    pass: bool,
    observations: Arc<Mutex<Vec<(String, i32)>>>,
}
impl ModLootConditionCodec for Codec {
    fn decode(&self, data: &serde_json::Value) -> Result<Arc<dyn ModLootCondition>, String> {
        self.condition_decodes.fetch_add(1, Ordering::Relaxed);
        let pass = data["pass"]
            .as_bool()
            .ok_or_else(|| "pass: expected boolean".to_owned())?;
        let label = data["label"].as_str().unwrap_or("gate").to_owned();
        Ok(Arc::new(GateRule {
            label,
            pass,
            observations: self.observations.clone(),
        }))
    }
}
impl ModLootCondition for GateRule {
    fn referenced_context_params(&self) -> Vec<Identifier> {
        vec![Identifier::parse("minecraft:block_entity").unwrap()]
    }
    fn test(&self, context: &mut dyn LootTypeContext) -> Result<bool, String> {
        assert!(context.params().origin().is_none());
        let draw = context.random().next_int_bound(5);
        self.observations
            .lock()
            .unwrap()
            .push((self.label.clone(), draw));
        Ok(self.pass)
    }
}

#[mod_api::__subscriptions]
impl Hooks for Codec {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.item_key("minecraft:stick");
        r.register_loot_function_type(&Identifier::parse("test:scale").unwrap(), self.clone());
        r.register_loot_condition_type(&Identifier::parse("test:gate").unwrap(), self.clone());
    }
}
fn gate(label: &str, pass: bool) -> serde_json::Value {
    json!({"type":"test:gate", "label":label, "pass":pass})
}
fn function(label: &str, factor: i32) -> serde_json::Value {
    json!({"type":"test:scale", "label":label, "factor":factor})
}

fn fixture_defaults() -> sunrise_world::item_defaults::ItemDefaults {
    sunrise_world::item_defaults::ItemDefaults::decode(
        br#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
    )
    .unwrap()
}

fn file(path: &str, value: serde_json::Value) -> (String, Vec<Vec<u8>>) {
    (path.to_owned(), vec![serde_json::to_vec(&value).unwrap()])
}

#[test]
fn registered_types_bind_once_and_preserve_both_evaluators_order_short_circuit_and_selected_random()
{
    use sunrise_core::random::{LegacyRandom, RandomSource};
    let observations = Arc::new(Mutex::new(Vec::new()));
    let codec = Codec::new(observations.clone());
    let fn_decodes = codec.function_decodes.clone();
    let cond_decodes = codec.condition_decodes.clone();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("test", Box::new(codec))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let enchantment = json!({
                "supported_items": ["minecraft:stick"],
                "weight": 1,
                "max_level": 1,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 10, "per_level_above_first": 0},
                "slots": ["mainhand"],
                "effects": {
                    "minecraft:equipment_drops": [{
                        "enchanted": "attacker",
                        "affected": "victim",
                        "requirements": gate("enchant drop", true),
                        "effect": {"type": "minecraft:add", "value": 0.25}
                    }]
                }
            });
            let mut skipped = function("skipped body", 9);
            skipped["condition"] = gate("gate false", false);
            let mut scaled = function("entry", 3);
            scaled["condition"] = gate("gate true", true);
            let table = json!({"type":"minecraft:empty", "pools":[{"rolls":1,
            "condition":{"type":"minecraft:all_of", "terms":[gate("pool", true), {"type":"minecraft:any_of", "terms":[gate("any",true), gate("skipped any",false)]}]},
            "entries":[{"type":"minecraft:item","name":"minecraft:stick",
                "condition":{"type":"minecraft:any_of","terms":[gate("entry false",false), gate("entry true",true), gate("skipped entry",false)]},
                "modifier":{"type":"minecraft:sequence","functions":[{"type":"minecraft:set_count","count":2},skipped,scaled,{"type":"minecraft:filtered","item_filter":{"items":"minecraft:stick"},"on_pass":function("sequence",2),"on_fail":function("skipped filter fail",9)},{"type":"minecraft:filtered","item_filter":{"items":"minecraft:diamond"},"on_pass":function("skipped filter pass",9),"on_fail":function("filtered fail",1)}]}}],
            "modifier":function("pool function",2)}], "modifier":function("table function",2)});
            let mut files = [
                file("loot_table/test:root", table.clone()),
                file("predicate/test:shared", gate("shared pred", true)),
                file("item_modifier/test:shared", function("shared mod", 1)),
                file("enchantment/test:drop_bonus", enchantment),
                file(
                    "loot_table/test:bonus",
                    json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]}),
                ),
            ];
            let defaults = fixture_defaults();
            let mut data = loot::LootData::load_with_types(
                root,
                &loot::PackOverlay::new(&files),
                &defaults,
                capture(),
            )
            .unwrap();
            // 8 inline functions in root + 1 named modifier = 9 function decodes.
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 9);
            // 8 inline conditions in root + 1 named predicate + 1 enchantment equipment_drops requirement = 10 condition decodes.
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 10);
            // The split at the end of a roll uses the item catalog's prototype.
            let mut items =
                sunrise_items::items::Items::catalog(root, &loot::PackOverlay::NONE, &defaults)
                    .unwrap();
            items.set_loot_data(&data);
            let mut expected_random = LegacyRandom::new(123);
            let labels = [
                "pool",
                "any",
                "entry false",
                "entry true",
                "gate false",
                "gate true",
                "entry",
                "sequence",
                "filtered fail",
                "pool function",
                "table function",
            ];
            let expected: Vec<_> = labels
                .iter()
                .map(|label| {
                    (
                        label.to_string(),
                        RandomSource::next_int_bound(&mut expected_random, 5).unwrap(),
                    )
                })
                .collect();
            let count = ((((2 * 3 + expected[6].1) * 2 + expected[7].1) + expected[8].1) * 2
                + expected[9].1)
                * 2
                + expected[10].1;
            // Repeated rolls across both evaluators perform zero decodes.
            for _ in 0..3 {
                for item_engine in [false, true] {
                    observations.lock().unwrap().clear();
                    let mut random = LegacyRandom::new(123);
                    let total = if item_engine {
                        items
                            .loot_table("test:root", false, None, &mut random)
                            .iter()
                            .map(|stack| stack.count as i32)
                            .sum::<i32>()
                    } else {
                        data.roll("test:root", &mut random, 0.0, None, None)
                            .iter()
                            .map(|stack| stack.count)
                            .sum::<i32>()
                    };
                    assert_eq!(total, count);
                    assert_eq!(*observations.lock().unwrap(), expected);
                    assert_eq!(
                        RandomSource::next_long(&mut random),
                        RandomSource::next_long(&mut expected_random.clone())
                    );
                }
            }
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 9);
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 10);

            // Enchantment equipment_drops requirement runs bound rule with zero decodes.
            let mut attacker = loot::LootEntity::new("minecraft:player");
            attacker.living = true;
            let weapon = loot::LootStack {
                item: "minecraft:stick".into(),
                count: 1,
                damage: 0,
                enchantments: vec![("test:drop_bonus".into(), 1)],
                potion: None,
                map: false,
                exploration: None,
                ominous_amplifier: None,
                stew_effects: Vec::new(),
                names: Vec::new(),
                components: Vec::new(),
            };
            attacker.equipment.push(("mainhand".into(), weapon));
            let params = loot::EntityLootParams {
                this: loot::LootEntity::new("minecraft:zombie"),
                damage: Some(loot::LootDamage {
                    kind: "minecraft:player_attack".into(),
                    source: Some(attacker),
                    direct: None,
                    is_direct: false,
                }),
                ..Default::default()
            };
            observations.lock().unwrap().clear();
            let chance = data.equipment_drop_chance(&params, 0.5);
            assert_eq!(chance, 0.75);
            assert_eq!(observations.lock().unwrap().len(), 1);
            assert_eq!(observations.lock().unwrap()[0].0, "enchant drop");
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 10);

            // Inline table binds its own nodes once and reuses named predicates/modifiers without re-decoding.
            let inline = json!({
                "pools": [{
                    "rolls": 1,
                    "condition": "test:shared",
                    "entries": [{"type": "minecraft:item", "name": "minecraft:stick", "modifier": ["test:shared", function("inline fn", 4)]}]
                }]
            });
            let prepared = items.prepare_inline_loot_table(&inline).unwrap();
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 10);
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 10);
            let mut expected_inline_rng = LegacyRandom::new(7);
            let mut draw = || RandomSource::next_int_bound(&mut expected_inline_rng, 5).unwrap();
            let expected_draws = [
                ("shared pred", draw()),
                ("shared mod", draw()),
                ("inline fn", draw()),
            ];
            let inline_count = (1 + expected_draws[1].1) * 4 + expected_draws[2].1;
            for _ in 0..2 {
                for item_engine in [false, true] {
                    observations.lock().unwrap().clear();
                    let mut random = LegacyRandom::new(7);
                    let total = if item_engine {
                        items
                            .roll_prepared_inline_loot_table_for_command(
                                &prepared,
                                None,
                                &mut random,
                                false,
                            )
                            .iter()
                            .map(|s| s.count as i32)
                            .sum::<i32>()
                    } else {
                        data.roll_prepared_inline(&prepared, &mut random)
                            .iter()
                            .map(|s| s.count)
                            .sum::<i32>()
                    };
                    assert_eq!(total, inline_count);
                    assert_eq!(
                        *observations.lock().unwrap(),
                        expected_draws.map(|(label, draw)| (label.to_owned(), draw))
                    );
                    assert_eq!(
                        RandomSource::next_long(&mut random),
                        RandomSource::next_long(&mut expected_inline_rng.clone())
                    );
                }
            }
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 10);
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 10);

            // A global modifier evaluates the prepared inline predicate and its named reference
            // on the same random stream in both engines. Neither condition is decoded by a roll.
            use loot::global_modifiers::{
                AddTableModifier, DecodedModifier, GlobalLootModifiers, ModifierFile,
            };
            let modifiers = GlobalLootModifiers::load(
                [ModifierFile {
                    id: "test:bonus".into(),
                    pack: "test".into(),
                    path: "test:bonus.json".into(),
                    read: Box::new(|| Ok(br#"{"type":"neoforge:add_table"}"#.to_vec())),
                }],
                &sunrise_world::conditions::LoadConditions::vanilla(),
                |_, _, _| {
                    Ok(DecodedModifier {
                    priority: 1000,
                    condition: Some(data.bindings().prepare_condition(
                        json!({"type":"minecraft:all_of","terms":["test:shared",gate("glm", true)]}),
                        "loot_modifier test:bonus $.condition",
                    ).map_err(|e| e.to_string())?),
                    apply: Arc::new(AddTableModifier { table: "test:bonus".into() }),
                })
                },
                |error| panic!("{error}"),
            );
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 11);
            data.set_global_modifiers(Arc::new(modifiers));
            items.set_loot_data(&data);
            let mut expected_global_rng = expected_random.clone();
            let expected_global = [
                (
                    "shared pred".to_owned(),
                    RandomSource::next_int_bound(&mut expected_global_rng, 5).unwrap(),
                ),
                (
                    "glm".to_owned(),
                    RandomSource::next_int_bound(&mut expected_global_rng, 5).unwrap(),
                ),
            ];
            for item_engine in [false, true] {
                observations.lock().unwrap().clear();
                let mut random = LegacyRandom::new(123);
                let total = if item_engine {
                    items
                        .loot_table("test:root", false, None, &mut random)
                        .iter()
                        .map(|s| s.count as i32)
                        .sum::<i32>()
                } else {
                    data.roll("test:root", &mut random, 0.0, None, None)
                        .iter()
                        .map(|s| s.count)
                        .sum::<i32>()
                };
                assert_eq!(total, count + 1);
                assert_eq!(
                    &observations.lock().unwrap()[expected.len()..],
                    &expected_global
                );
                assert_eq!(
                    RandomSource::next_long(&mut random),
                    RandomSource::next_long(&mut expected_global_rng.clone())
                );
            }
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 10);
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 11);
            data.set_global_modifiers(Arc::default());

            // Reloading a modified table decodes into a fresh bound rule for the new generation.
            let reloaded_table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick","modifier":function("reloaded", 5)}]}]});
            files[0] = file("loot_table/test:root", reloaded_table);
            let reloaded = loot::LootData::load_with_types(
                root,
                &loot::PackOverlay::new(&files),
                &defaults,
                capture(),
            )
            .unwrap();
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 12);
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 13);
            items.set_loot_data(&reloaded);
            let expected_draw = RandomSource::next_int_bound(&mut LegacyRandom::new(9), 5).unwrap();
            for item_engine in [false, true] {
                observations.lock().unwrap().clear();
                let mut random = LegacyRandom::new(9);
                let (item, count) = if item_engine {
                    let dropped = items.loot_table("test:root", false, None, &mut random);
                    assert_eq!(dropped.len(), 1);
                    (dropped[0].item.to_string(), dropped[0].count as i32)
                } else {
                    let dropped = reloaded.roll("test:root", &mut random, 0.0, None, None);
                    assert_eq!(dropped.len(), 1);
                    (dropped[0].item.clone(), dropped[0].count)
                };
                assert_eq!(
                    (item.as_str(), count),
                    ("minecraft:stick", 5 + expected_draw)
                );
                assert_eq!(
                    *observations.lock().unwrap(),
                    [("reloaded".to_owned(), expected_draw)]
                );
            }
            assert_eq!(fn_decodes.load(Ordering::Relaxed), 12);
            assert_eq!(cond_decodes.load(Ordering::Relaxed), 13);

            let mut bad_inline = inline.clone();
            bad_inline["pools"][0]["entries"][0]["modifier"][1]["factor"] = json!(false);
            let error = items
                .prepare_inline_loot_table(&bad_inline)
                .err()
                .expect("invalid custom rule accepted as a command argument")
                .to_string();
            assert!(
                error.contains("inline loot_table") && error.contains("factor"),
                "{error}"
            );
            let mut random = LegacyRandom::new(7);
            assert!(data.roll_inline(&bad_inline, &mut random).is_empty());
            assert_eq!(
                RandomSource::next_long(&mut random),
                RandomSource::next_long(&mut LegacyRandom::new(7)),
            );

            // Refusing a bad reload never mutates the already published generation.
            for (entry, expected) in [
                (json!({"type":"test:absent"}), "unknown loot type"),
                (json!({"type":"test:scale","factor":false}), "factor"),
                (json!({"type":"minecraft:absent"}), "unknown loot type"),
            ] {
                let mut bad = table.clone();
                bad["modifier"] = entry;
                files[0] = file("loot_table/test:root", bad);
                let error = loot::LootData::load_with_types(
                    root,
                    &loot::PackOverlay::new(&files),
                    &defaults,
                    capture(),
                )
                .err()
                .unwrap()
                .to_string();
                assert!(
                    error.contains("loot_table test:root")
                        && error.contains("[data/test/loot_table/root.json]")
                        && error.contains("$.modifier")
                        && error.contains(expected),
                    "{error}"
                );
                observations.lock().unwrap().clear();
                assert!(
                    !data
                        .roll("test:root", &mut LegacyRandom::new(123), 0.0, None, None)
                        .is_empty()
                );
            }
        },
    );
}

#[derive(Clone)]
struct PanickingCodec {
    decode_calls: Arc<AtomicUsize>,
    apply_calls: Arc<AtomicUsize>,
}
struct PanickingRule {
    apply_calls: Arc<AtomicUsize>,
}
impl ModLootFunctionCodec for PanickingCodec {
    fn decode(&self, _: &serde_json::Value) -> Result<Arc<dyn ModLootFunction>, String> {
        self.decode_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(PanickingRule {
            apply_calls: self.apply_calls.clone(),
        }))
    }
}
impl ModLootFunction for PanickingRule {
    fn apply(
        &self,
        _: &mut mod_api::LootStackEditor<'_>,
        _: &mut dyn mod_api::LootTypeContext,
    ) -> Result<(), String> {
        self.apply_calls.fetch_add(1, Ordering::Relaxed);
        panic!("boom in panicking loot rule");
    }
}
#[mod_api::__subscriptions]
impl Hooks for PanickingCodec {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.item_key("minecraft:stick");
        r.register_loot_function_type(&Identifier::parse("panicmod:boom").unwrap(), self.clone());
    }
}

#[derive(Clone)]
struct HealthyCodec {
    apply_calls: Arc<AtomicUsize>,
}
impl ModLootFunctionCodec for HealthyCodec {
    fn decode(&self, _: &serde_json::Value) -> Result<Arc<dyn ModLootFunction>, String> {
        Ok(Arc::new(self.clone()))
    }
}
impl ModLootFunction for HealthyCodec {
    fn apply(
        &self,
        stack: &mut mod_api::LootStackEditor<'_>,
        _: &mut dyn mod_api::LootTypeContext,
    ) -> Result<(), String> {
        self.apply_calls.fetch_add(1, Ordering::Relaxed);
        stack.set_count(stack.view().count() + 10);
        Ok(())
    }
}
#[mod_api::__subscriptions]
impl Hooks for HealthyCodec {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.item_key("minecraft:stick");
        r.register_loot_function_type(&Identifier::parse("goodmod:add").unwrap(), self.clone());
    }
}

#[test]
fn panicking_rule_stays_poisoned_across_reloads_while_other_registrations_keep_running() {
    use sunrise_core::random::LegacyRandom;
    let panic_decodes = Arc::new(AtomicUsize::new(0));
    let panic_applies = Arc::new(AtomicUsize::new(0));
    let healthy_applies = Arc::new(AtomicUsize::new(0));
    let bad = PanickingCodec {
        decode_calls: panic_decodes.clone(),
        apply_calls: panic_applies.clone(),
    };
    let good = HealthyCodec {
        apply_calls: healthy_applies.clone(),
    };
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("panicmod", Box::new(bad)),
            mod_api::ModEntry::new("goodmod", Box::new(good)),
        ]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let root = std::path::Path::new("scoped-loot-panic-overlay");
            let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick",
                "modifier":[{"type":"panicmod:boom"},{"type":"goodmod:add"}]}]}]});
            let mut files = [file("loot_table/test:root", table)];
            let defaults = fixture_defaults();
            let data = loot::LootData::load_with_types(
                root,
                &loot::PackOverlay::new(&files),
                &defaults,
                capture(),
            )
            .unwrap();
            assert_eq!(panic_decodes.load(Ordering::Relaxed), 1);
            // First roll panics inside panicmod:boom, poisons its shared CallbackOwner,
            // and still runs goodmod:add.
            let first = data.roll("test:root", &mut LegacyRandom::new(1), 0.0, None, None);
            assert_eq!(first[0].count, 11);
            assert_eq!(panic_applies.load(Ordering::Relaxed), 1);
            assert_eq!(healthy_applies.load(Ordering::Relaxed), 1);
            // Second roll never invokes panicmod:boom again, while goodmod:add runs.
            let second = data.roll("test:root", &mut LegacyRandom::new(1), 0.0, None, None);
            assert_eq!(second[0].count, 11);
            assert_eq!(panic_applies.load(Ordering::Relaxed), 1);
            assert_eq!(healthy_applies.load(Ordering::Relaxed), 2);
            // A fresh capture/reload still shares the poisoned CallbackOwner:
            // panicmod:boom's decode is refused without invoking user code again.
            let reload = loot::LootData::load_with_types(
                root,
                &loot::PackOverlay::new(&files),
                &defaults,
                capture(),
            );
            assert!(reload.is_err());
            assert_eq!(panic_decodes.load(Ordering::Relaxed), 1);
            // A reload that drops panicmod:boom still succeeds and runs goodmod:add.
            let good_only = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick",
                "modifier":[{"type":"goodmod:add"}]}]}]});
            files[0] = file("loot_table/test:root", good_only);
            let reloaded = loot::LootData::load_with_types(
                root,
                &loot::PackOverlay::new(&files),
                &defaults,
                capture(),
            )
            .unwrap();
            let third = reloaded.roll("test:root", &mut LegacyRandom::new(1), 0.0, None, None);
            assert_eq!(third[0].count, 11);
            assert_eq!(healthy_applies.load(Ordering::Relaxed), 3);
        },
    );
}
