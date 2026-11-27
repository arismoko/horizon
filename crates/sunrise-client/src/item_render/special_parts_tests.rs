//! A registered renderer submits its baked geometry with the base model's display transforms,
//! particle and GUI light, in every display context and after a reload.
use super::*;
use mod_api::{
    __private::parts_installation::item_key, Component, ItemStack, SpecialModelBakeContext,
    SpecialModelCollector, SpecialModelFactory, SpecialModelGeometry, SpecialModelQuad,
    SpecialModelRenderer, SpecialModelStack, testing::FakeHost,
};
use std::fs;
use sunrise_assets::resource_source::{Resource, ResourceRoot};
use sunrise_core::nbt::Tag;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PanConfig {
    handle: u8,
}

struct Pan {
    handle: u8,
    a: SpecialModelGeometry,
    b: SpecialModelGeometry,
    palette: Arc<[u32]>,
}

impl Pan {
    fn bake(config: PanConfig, bake: &dyn SpecialModelBakeContext) -> Result<Self, String> {
        Ok(Self {
            handle: config.handle,
            a: bake.bake_model("spt:item/a")?,
            b: bake.bake_model("spt:item/b")?,
            palette: Arc::new([0xFFCC_2211]),
        })
    }
}

struct PanTint;
impl Component for PanTint {
    type Value = i32;

    fn id(&self) -> &str {
        "spt:tint"
    }

    fn decode(&self, value: &Tag) -> Option<i32> {
        match value {
            Tag::Int(value) => Some(*value),
            _ => None,
        }
    }

    fn encode(&self, value: i32) -> Tag {
        Tag::Int(value)
    }
}

const fn translated(x: f32) -> [f32; 12] {
    [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}
const DOUBLED: [f32; 12] = [2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0];

impl SpecialModelRenderer for Pan {
    type Argument = (u32, Option<i32>);

    fn extract_argument(
        &self,
        _: &dyn ClientHost,
        stack: &SpecialModelStack<'_>,
    ) -> Self::Argument {
        // Only the pan's component tints its geometry, not an unrelated stack's.
        let tint = if stack.item() == item_key(0) {
            stack.get(&PanTint)
        } else {
            None
        };
        (stack.count(), tint)
    }

    fn submit(
        &self,
        &(count, tint): &Self::Argument,
        context: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        let stack = |collector: &mut SpecialModelCollector<'_>, key, count| {
            collector.submit_stack(
                &ItemStack::new(item_key(key), count).unwrap(),
                ItemDisplayContext::Gui,
            )
        };
        let geometry = |collector: &mut SpecialModelCollector<'_>, model: &SpecialModelGeometry| {
            let tints = tint.map_or_else(
                || SpecialModelTints::Indexed(Arc::clone(&self.palette)),
                |tint| SpecialModelTints::Uniform(tint as u32),
            );
            collector.with_pose(translated(f32::from(self.handle) / 8.0), |pose| {
                pose.submit_model(model, tints, mod_api::SpecialModelSubmitOptions::default())
            })
        };
        match count {
            1 => geometry(collector, &self.a),
            3 => {
                geometry(collector, &self.a)?;
                stack(collector, u32::MAX, 1)
            }
            4 => {
                geometry(collector, &self.a)?;
                stack(collector, 1, 1)?;
                Err("the test renderer refuses this stack".into())
            }
            5 => collector.with_pose(DOUBLED, |pose| {
                pose.submit_model(
                    &self.a,
                    SpecialModelTints::Untinted,
                    mod_api::SpecialModelSubmitOptions::default(),
                )
            }),
            6 | 7 => collector.submit_model(
                &self.a,
                SpecialModelTints::Uniform([WHITE, GREY][count as usize - 6] as u32),
                mod_api::SpecialModelSubmitOptions::default(),
            ),
            // A tinted, foiled cube beside B; then the pan itself, a cycle.
            8 => {
                collector.with_pose(translated(0.25), |pose| stack(pose, 2, 1))?;
                geometry(collector, &self.b)
            }
            9 => collector.with_pose(translated(0.25), |pose| stack(pose, 0, 9)),
            // A to B to A with geometry staged at both levels.
            10 => {
                geometry(collector, &self.b)?;
                stack(collector, 0, 11)
            }
            11 => {
                geometry(collector, &self.a)?;
                stack(collector, 0, 10)
            }
            // Ignore the nested refusal on purpose: neither layer may commit.
            12 | 13 => {
                let (model, inner, count) = if count == 12 {
                    (&self.b, 0, 13)
                } else {
                    (&self.a, 3, 12)
                };
                geometry(collector, model)?;
                let _ = stack(collector, inner, count);
                let _ = geometry(collector, model);
                Ok(())
            }
            14 => {
                geometry(collector, &self.b)?;
                stack(collector, 4, 4)
            }
            15 => {
                geometry(collector, &self.a)?;
                stack(collector, 1, 1)?;
                panic!("the pan fell")
            }
            16 => {
                geometry(collector, &self.a)?;
                // Both input transforms are finite; their composed pose is not.
                let _ = collector.with_pose(DOUBLED, |pose| {
                    pose.with_pose(translated(f32::MAX), |_| Ok(()))
                });
                Ok(())
            }
            20 | 21 | 24 => {
                let cubes = if count == 24 { 63 } else { 44 + count };
                for _ in 0..cubes {
                    stack(collector, 1, 1)?;
                }
                Ok(())
            }
            22 | 23 => {
                geometry(collector, &self.b)?;
                stack(collector, 0, if count == 22 { 20 } else { 24 })
            }
            25 => {
                collector.with_pose(translated(0.25), |pose| {
                    pose.with_pose(DOUBLED, |pose| {
                        pose.submit_model(
                            &self.a,
                            SpecialModelTints::Untinted,
                            mod_api::SpecialModelSubmitOptions::default(),
                        )
                    })?;
                    pose.submit_model(
                        &self.a,
                        SpecialModelTints::Untinted,
                        mod_api::SpecialModelSubmitOptions::default(),
                    )
                })?;
                collector.submit_model(
                    &self.a,
                    SpecialModelTints::Untinted,
                    mod_api::SpecialModelSubmitOptions::default(),
                )
            }
            26 => {
                if context.display == ItemDisplayContext::Fixed {
                    collector.submit_stack(
                        &ItemStack::new(item_key(0), 26).unwrap(),
                        ItemDisplayContext::Gui,
                    )
                } else {
                    geometry(collector, &self.a)
                }
            }
            30 => {
                let lease = if context.display == ItemDisplayContext::Ground
                    && context.game_time == Some(41)
                {
                    &self.b
                } else {
                    &self.a
                };
                let tints = tint.map_or(SpecialModelTints::Untinted, |tint| {
                    SpecialModelTints::Uniform(tint as u32)
                });
                collector.submit_model(lease, tints, mod_api::SpecialModelSubmitOptions::default())
            }
            1000.. => {
                geometry(collector, &self.a)?;
                stack(collector, 0, count + 1)
            }
            _ => geometry(collector, &self.b),
        }
    }
}

/// Bridge the fixture's synthetic items to the same collector as the game uses.
struct PanEnv(FakeHost);
impl ItemEnv for PanEnv {}
impl ItemDrawEnv for PanEnv {
    fn special_model_input(
        &self,
        stack: &StackView,
        context: &Context<'_>,
    ) -> Option<SpecialModelInput<'_>> {
        let item = match stack.item.as_str() {
            "spt:pan" | "spt:lit" | "spt:sheets" => 0,
            "spt:composite" => 3,
            "spt:fault" | "spt:alias" | "spt:bare" | "spt:empty_path" => 4,
            "spt:bundle" => 5,
            _ => return None,
        };
        let display = ItemDisplayContext::ALL
            .into_iter()
            .find(|value| value.name() == context.display.name())?;
        Some(SpecialModelInput {
            host: &self.0,
            item: item_key(item),
            count: NonZeroU32::new(stack.count.try_into().ok()?)?,
            context: SpecialModelContext::new(
                display,
                context.level.map(|level| level.game_time),
                0.25,
            ),
        })
    }

    fn special_model_stack(
        &self,
        inner: &ItemStack,
        display: ItemDisplayContext,
    ) -> Option<(StackView, DisplayContext)> {
        let display = DisplayContext::ALL
            .into_iter()
            .find(|value| value.name() == display.name())?;
        let (item, glint) = match inner.item() {
            key if key == item_key(0) => ("spt:pan", false),
            key if key == item_key(1) => ("spt:cube", false),
            key if key == item_key(2) => ("spt:grey_cube", true),
            key if key == item_key(3) => ("spt:composite", false),
            key if key == item_key(4) => ("spt:fault", false),
            _ => return None,
        };
        let mut stack = stack(item, inner.count() as i32);
        if glint {
            stack.patch.insert(
                "minecraft:enchantment_glint_override".into(),
                Some(serde_json::json!(true)),
            );
        }
        Some((stack, display))
    }
}

fn owned_factory<C: 'static, R: SpecialModelRenderer>(
    decode: impl for<'a> Fn(mod_api::SpecialModelConfig<'a>) -> Result<C, String>
    + Send
    + Sync
    + 'static,
    bake: impl Fn(C, &dyn SpecialModelBakeContext) -> Result<R, String> + Send + Sync + 'static,
) -> OwnedSpecialModelFactory {
    OwnedSpecialModelFactory::new("special_test", SpecialModelFactory::new(decode, bake))
}

fn pan_factories(on_bake: impl Fn(u8) + Send + Sync + 'static) -> SpecialModelFactories {
    BTreeMap::from([(
        mod_api::Identifier::parse("spt:pan").unwrap(),
        owned_factory(
            |config| config.decode::<PanConfig>(),
            move |config, bake| {
                on_bake(config.handle);
                Pan::bake(config, bake)
            },
        ),
    )])
}

/// White and an encoded 0.5 grey, packed vanilla-space ARGB.
const WHITE: i32 = 0xFFFF_FFFF_u32 as i32;
const GREY: i32 = 0xFF80_8080_u32 as i32;

/// `n` separated 4-pixel cubes, every face tinted by index 0.
fn cubes(n: usize) -> String {
    let faces = ["north", "east", "south", "west", "up", "down"]
        .map(|f| format!(r##""{f}":{{"uv":[0,0,4,4],"texture":"#all","tintindex":0}}"##))
        .join(",");
    let elements = (0..n)
        .map(|i| {
            let x = i * 5;
            format!(
                r#"{{"from":[{x},0,0],"to":[{},4,4],"faces":{{{faces}}}}}"#,
                x + 4
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"textures":{{"all":"spt:item/grey"}},"elements":[{elements}]}}"#)
}

fn write(root: &std::path::Path, name: &str, data: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// A pan node between ordinary grey and white cube models, to test layer-local refusal.
fn write_composite(root: &std::path::Path) {
    write(
        root,
        "assets/spt/items/composite.json",
        &format!(
            r#"{{"model":{{"type":"composite","models":[
            {{"type":"model","model":"spt:item/a","tints":[{{"type":"constant","value":{GREY}}}]}},
            {{"type":"special","base":"spt:item/base","model":{{"type":"spt:pan","handle":2}}}},
            {{"type":"model","model":"spt:item/b","tints":[{{"type":"constant","value":{WHITE}}}]}}
        ]}}}}"#
        ),
    );
}

/// The packs in `root`, with B of `b_cubes` cubes.
fn renderer(
    root: &std::path::Path,
    b_cubes: usize,
    handle: u8,
    factories: &SpecialModelFactories,
) -> ItemRenderer {
    write(root, "items.json", "{}");
    write(
        root,
        "assets/minecraft/atlases/items.json",
        r#"{"sources":[{"type":"single","resource":"spt:item/grey"},{"type":"single","resource":"spt:item/flour"},{"type":"single","resource":"spt:item/glass"}]}"#,
    );
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/grey.png"),
        16,
        16,
        [128, 128, 128, 255],
    );
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/flour.png"),
        16,
        16,
        [250, 250, 250, 255],
    );
    // Alpha 128: the sprite classifier marks it translucent.
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/glass.png"),
        16,
        16,
        [255, 255, 255, 128],
    );
    write(
        root,
        "assets/spt/items/pan.json",
        &format!(
            r#"{{"model":{{"type":"minecraft:special","base":"spt:item/base","model":{{"type":"spt:pan","handle":{handle}}}}}}}"#
        ),
    );
    write(
        root,
        "assets/spt/items/cube.json",
        r#"{"model":{"type":"minecraft:model","model":"spt:item/a"}}"#,
    );
    // Cube A tinted white or grey by vanilla's constant tint source.
    for (name, tint) in [("white_cube", WHITE), ("grey_cube", GREY)] {
        write(
            root,
            &format!("assets/spt/items/{name}.json"),
            &format!(
                r#"{{"model":{{"type":"minecraft:model","model":"spt:item/a","tints":[{{"type":"minecraft:constant","value":{tint}}}]}}}}"#
            ),
        );
    }
    write(
        root,
        "assets/spt/items/stranger.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:unregistered"}}}"#,
    );
    // The base has no geometry of its own: its display transforms differ by context, its
    // particle is flour and its GUI light is flat.
    write(
        root,
        "assets/spt/models/item/base.json",
        r#"{"gui_light":"front","textures":{"particle":"spt:item/flour"},"display":{
            "ground":{"scale":[0.5,0.5,0.5]},
            "fixed":{"rotation":[0,180,0]},
            "head":{"translation":[0,13,7]},
            "thirdperson_righthand":{"rotation":[75,45,0],"translation":[0,2.5,0]},
            "gui":{"rotation":[30,225,0],"scale":[0.625,0.625,0.625]}}}"#,
    );
    write(root, "assets/spt/models/item/a.json", &cubes(1));
    write(root, "assets/spt/models/item/b.json", &cubes(b_cubes));
    let data = crate::test_resources::item_data(
        root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let root = ResourceRoot::directory(root.to_path_buf());
    ItemRenderer::load_tables(
        &root,
        crate::model_library::ParsedModels::load(root.source()).into_item_library(),
        true,
        &data,
        None,
        ItemProviderTables {
            special: Some(factories),
            ..Default::default()
        },
    )
    .unwrap()
}

fn stack(item: &str, count: i32) -> StackView {
    let mut stack = StackView::new(item, count);
    stack
        .patch
        .insert("minecraft:item_model".into(), Some(serde_json::json!(item)));
    stack
}

#[test]
fn each_special_draw_reads_effective_components_and_current_context() {
    let root = tempfile::tempdir().unwrap();
    let mut renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    Arc::make_mut(&mut Arc::make_mut(&mut renderer.models).prototypes).insert(
        "spt:pan".into(),
        BTreeMap::from([("spt:tint".into(), serde_json::json!(GREY))]),
    );
    let env = PanEnv(FakeHost::new());
    let mut pan = stack("spt:pan", 30);
    let level = |game_time| LevelState {
        game_time,
        ..Default::default()
    };
    let draw = |pan: &StackView, display, time| {
        let state = renderer.resolve(pan, display, None, Some(&level(time)), 0, &env);
        quads(&renderer, &state)
            .into_iter()
            .map(|quad| quad.color)
            .collect::<Vec<_>>()
    };

    // Same stack, same view, and no intervening frame: both time and display matter.
    assert_eq!(
        draw(&pan, DisplayContext::Ground, 41),
        vec![GREY as u32; 12]
    );
    assert_eq!(draw(&pan, DisplayContext::Ground, 42), vec![GREY as u32; 6]);
    assert_eq!(draw(&pan, DisplayContext::Gui, 41), vec![GREY as u32; 6]);
    pan.patch.insert(
        "spt:tint".into(),
        Some(serde_json::json!(0xFFAA_1100_u32 as i32)),
    );
    assert_eq!(draw(&pan, DisplayContext::Gui, 41), vec![0xFFAA_1100; 6]);
    pan.patch.insert("spt:tint".into(), None);
    assert_eq!(draw(&pan, DisplayContext::Gui, 41), vec![u32::MAX; 6]);
}

fn is_missing_layer(renderer: &ItemRenderer, layer: &LayerRender) -> bool {
    matches!(&layer.geometry, LayerGeometry::Quads(model)
        if Arc::ptr_eq(model, &renderer.bake(MISSING_MODEL)))
}

fn assert_refused_special_between_siblings(renderer: &ItemRenderer, state: &ItemRenderState) {
    let [_, replacement, _] = state.layers.as_slice() else {
        panic!("the failed layer leaked geometry or erased a sibling");
    };
    assert!(is_missing_layer(renderer, replacement));
    // The fixture atlas omits the missing sprite: only the two ordinary siblings draw.
    let colors: Vec<_> = quads(renderer, state)
        .into_iter()
        .map(|quad| quad.color)
        .collect();
    assert_eq!(colors.len(), 18);
    assert_eq!(colors[..6], [GREY as u32; 6]);
    assert_eq!(colors[6..], [WHITE as u32; 12]);
}

fn special_draw_reports(level: log::Level) -> usize {
    crate::test_log::records()
        .into_iter()
        .filter(|(record_level, message)| {
            *record_level == level && message.contains("special_test")
        })
        .count()
}

#[test]
fn a_refused_special_draw_is_reported_each_time_without_disabling_the_renderer() {
    let root = tempfile::tempdir().unwrap();
    write_composite(root.path());
    let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    let env = PanEnv(FakeHost::new());
    let draw =
        |stack: &StackView| renderer.resolve(stack, DisplayContext::None, None, None, 0, &env);
    let mut unreadable = stack("spt:unreadable", 1);
    unreadable.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("spt:composite")),
    );
    crate::test_log::install();
    for bad in [
        stack("spt:composite", 4), // Err after submitting geometry, twice on the same baked node.
        stack("spt:composite", 4),
        stack("spt:composite", 16), // A collector refusal the callback didn't propagate.
        unreadable.clone(),
        unreadable,
    ] {
        crate::test_log::clear();
        assert_refused_special_between_siblings(&renderer, &draw(&bad));
        assert_eq!(special_draw_reports(log::Level::Warn), 1);
    }
    let working = draw(&stack("spt:composite", 1));
    assert_eq!(quads(&renderer, &working).len(), 24);
}

#[test]
fn a_panicking_special_draw_disables_only_its_registration_across_reload() {
    let root = tempfile::tempdir().unwrap();
    write_composite(root.path());
    for (item, kind) in [("alias", "pan"), ("fault", "fault")] {
        write(
            root.path(),
            &format!("assets/spt/items/{item}.json"),
            &format!(
                r#"{{"model":{{"type":"minecraft:special","base":"spt:item/base","model":{{"type":"spt:{kind}","handle":1}}}}}}"#,
            ),
        );
    }
    let mut factories = pan_factories(|_| {});
    factories.insert(
        mod_api::Identifier::parse("spt:fault").unwrap(),
        owned_factory(|config| config.decode::<PanConfig>(), Pan::bake),
    );
    let env = PanEnv(FakeHost::new());
    let draw = |renderer: &ItemRenderer, item, count| {
        renderer.resolve(
            &stack(item, count),
            DisplayContext::None,
            None,
            None,
            0,
            &env,
        )
    };
    let first = renderer(root.path(), 2, 2, &factories);
    assert_eq!(quads(&first, &draw(&first, "spt:alias", 1)).len(), 6);

    crate::test_log::install();
    crate::test_log::clear();
    assert_refused_special_between_siblings(&first, &draw(&first, "spt:composite", 15));
    assert_eq!(special_draw_reports(log::Level::Error), 1);

    let reloaded = renderer(root.path(), 2, 2, &factories);
    for generation in [&first, &reloaded] {
        // Both nodes using the pan registration are disabled; another registration from the
        // same mod can still draw. No new panic is reported for an already disabled callback.
        assert_refused_special_between_siblings(generation, &draw(generation, "spt:composite", 1));
        assert!(matches!(draw(generation, "spt:alias", 1).layers.as_slice(),
            [layer] if is_missing_layer(generation, layer)));
        assert_eq!(
            quads(generation, &draw(generation, "spt:fault", 1)).len(),
            6
        );
        assert_refused_special_between_siblings(generation, &draw(generation, "spt:composite", 15));
        assert_eq!(special_draw_reports(log::Level::Error), 1);
    }
}

#[test]
fn nested_stacks_allow_over_64_and_key_cycles_by_stack_and_display() {
    let root = tempfile::tempdir().unwrap();
    let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    let env = PanEnv(FakeHost::new());
    let resolve =
        |count, display| renderer.resolve(&stack("spt:pan", count), display, None, None, 0, &env);
    let missing = renderer.bake(MISSING_MODEL);
    let is_missing = |state: &ItemRenderState| {
        matches!(
            state.layers.as_slice(),
            [layer] if matches!(&layer.geometry, LayerGeometry::Quads(baked) if Arc::ptr_eq(baked, &missing))
        )
    };

    // Over 64 nested stack expansions succeed both flat and across nesting levels.
    assert_eq!(
        quads(&renderer, &resolve(21, DisplayContext::Gui)).len(),
        65 * 6
    );
    assert_eq!(
        quads(&renderer, &resolve(22, DisplayContext::Gui)).len(),
        12 + 64 * 6
    );

    // The same stack in the same display context is a cycle and falls back to the missing model.
    assert!(is_missing(&resolve(9, DisplayContext::Gui)));

    // Delegating the exact same stack under a different display context succeeds.
    let delegated = resolve(26, DisplayContext::Fixed);
    assert_eq!(quads(&renderer, &delegated).len(), 6);
    assert_eq!(delegated.layers[0].display, DisplayContext::Gui);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn nonrepeating_nested_stacks_refuse_only_the_recursive_draw() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let root = tempfile::tempdir().unwrap();
            let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
            let env = PanEnv(FakeHost::new());
            let resolve = |count| {
                renderer.resolve(&stack("spt:pan", count), DisplayContext::Gui, None, None, 0, &env)
            };
            let failed = resolve(1000);
            let missing = renderer.bake(MISSING_MODEL);
            assert!(matches!(
                failed.layers.as_slice(),
                [layer] if matches!(&layer.geometry, LayerGeometry::Quads(baked) if Arc::ptr_eq(baked, &missing))
            ));
            assert_eq!(quads(&renderer, &resolve(1)).len(), 6);
        })
        .unwrap()
        .join()
        .unwrap();
}

fn sprites() -> HashMap<String, Rect> {
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    HashMap::from([
        ("spt:item/grey".to_owned(), rect),
        ("spt:item/flour".to_owned(), rect),
    ])
}

fn quads(renderer: &ItemRenderer, state: &ItemRenderState) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    state.submit(renderer, &sprites(), &NoHost, Mat4::IDENTITY, &mut quads);
    quads
}

#[test]
fn an_invalid_winning_registered_model_does_not_load_a_lower_pack() {
    let item_path = "assets/spt/items/pan.json".to_owned();
    let lower = Resource::bytes(
        br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:pan","handle":2}}}"#.to_vec(),
    );
    let winner = Resource::bytes(
        br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:pan","handle":null}}}"#.to_vec(),
    );
    let files = BTreeMap::from([
        (item_path.clone(), winner.clone()),
        ("assets/spt/models/item/a.json".to_owned(), Resource::bytes(b"{}".to_vec())),
        ("assets/spt/models/item/b.json".to_owned(), Resource::bytes(b"{}".to_vec())),
        ("assets/spt/items/other.json".to_owned(), lower.clone()),
        ("assets/spt/items/decode-panic.json".to_owned(), Resource::bytes(
            br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:decode-panic"}}}"#.to_vec(),
        )),
        ("assets/spt/items/bake-panic.json".to_owned(), Resource::bytes(
            br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:bake-panic"}}}"#.to_vec(),
        )),
    ]);
    let pack = ResourceSource::indexed_with_layers(
        files,
        BTreeMap::from([(item_path, vec![lower, winner])]),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path();
    write(directory, "items.json", "{}");
    let item_data = crate::test_resources::item_data(
        directory,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let root = ResourceRoot::new(directory.to_owned(), directory.to_owned(), pack);
    let mut factories = pan_factories(|_| {});
    factories.extend([
        (
            mod_api::Identifier::parse("spt:decode-panic").unwrap(),
            owned_factory(
                |_| -> Result<Pan, String> { panic!("decoder panicked") },
                |pan, _| Ok(pan),
            ),
        ),
        (
            mod_api::Identifier::parse("spt:bake-panic").unwrap(),
            owned_factory(
                |_| Ok(()),
                |(), _| -> Result<Pan, String> { panic!("baker panicked") },
            ),
        ),
    ]);
    let renderer = ItemRenderer::load_tables(
        &root,
        crate::model_library::ParsedModels::load(root.source()).into_item_library(),
        true,
        &item_data,
        None,
        ItemProviderTables {
            special: Some(&factories),
            ..Default::default()
        },
    )
    .unwrap();
    let definitions = &renderer.models.definitions;
    assert_eq!(
        definitions.keys().map(String::as_str).collect::<Vec<_>>(),
        ["spt:other"]
    );
    assert!(matches!(
        &definitions["spt:other"].model,
        sunrise_assets::item_model::ItemModelNode::RegisteredSpecial { .. },
    ));
}

#[test]
fn a_disabled_override_never_reveals_the_builtin_decoder() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ITEM: &str = r#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"minecraft:trident"}}}"#;
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let factories = BTreeMap::from([(
        mod_api::Identifier::parse("minecraft:trident").unwrap(),
        owned_factory(
            {
                let calls = calls.clone();
                move |_| -> Result<Pan, String> {
                    calls.fetch_add(1, Ordering::Relaxed);
                    panic!("override decoder panicked")
                }
            },
            |pan, _| Ok(pan),
        ),
    )]);
    let first = renderer(temp.path(), 2, 2, &factories);
    let providers = registered_providers::ItemProviderTables {
        special: Some(&factories),
        ..Default::default()
    };
    assert!(first.decode_definition(ITEM, &Default::default()).is_ok());
    let failure = first.decode_definition(ITEM, &providers).unwrap_err();
    assert!(failure.to_string().contains("override decoder panicked"));

    let reloaded = renderer(temp.path(), 2, 2, &factories);
    for generation in [&first, &reloaded] {
        let failure = generation.decode_definition(ITEM, &providers).unwrap_err();
        assert!(failure.to_string().contains("owner is disabled"));
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn a_generation_bakes_typed_config_once_and_draws_use_the_baked_instance() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let factories = pan_factories({
        let calls = Arc::clone(&calls);
        move |handle| calls.lock().unwrap().push(handle)
    });
    let draw = |renderer: &ItemRenderer| {
        let env = PanEnv(FakeHost::new());
        let state = renderer.resolve(
            &stack("spt:pan", 1),
            DisplayContext::None,
            None,
            None,
            0,
            &env,
        );
        assert_eq!(quads(renderer, &state).len(), 6);
        let (lo, hi) = state.bounding_box();
        (lo.x, hi.x)
    };
    let first = renderer(temp.path(), 2, 2, &factories);
    assert_eq!(*calls.lock().unwrap(), [2]);
    assert_eq!(draw(&first), (-0.25, 0.0));
    assert_eq!(draw(&first), (-0.25, 0.0));
    assert_eq!(*calls.lock().unwrap(), [2]);

    let second = renderer(temp.path(), 2, 4, &factories);
    assert_eq!(*calls.lock().unwrap(), [2, 4]);
    assert_eq!(draw(&second), (0.0, 0.25));
    assert_eq!(draw(&first), (-0.25, 0.0));
    assert_eq!(draw(&second), (0.0, 0.25));
    assert_eq!(*calls.lock().unwrap(), [2, 4]);
}

#[test]
fn special_geometry_leases_resolve_pack_sprites_and_reject_invalid_quads() {
    let temp = tempfile::tempdir().unwrap();
    let leases = Arc::new(Mutex::new(None));
    let bake = {
        let leases = Arc::clone(&leases);
        move |pan, context: &dyn mod_api::SpecialModelBakeContext| {
            let positions = [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ];
            let mut quad = SpecialModelQuad::new(
                positions,
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                "spt:item/grey",
            );
            quad.tint_index = Some(2);
            quad.light_emission = 12;
            quad.force_translucent = true;
            let resource = context.bake_model("spt:item/b")?;
            let custom = context.bake_geometry(&[quad.clone()])?;
            let mut degenerate = quad.clone();
            degenerate.positions = [[0.0; 3]; 4];
            let mut outside_uv = quad.clone();
            outside_uv.uv = [[-1.0, 2.5], [1.5, -0.25], [2.0, 3.0], [-0.5, 1.25]];
            let mut missing_sprite = quad.clone();
            missing_sprite.sprite = "spt:item/not-present".into();
            let fallbacks = context.bake_geometry(&[degenerate, outside_uv, missing_sprite])?;
            let invalid: [(&str, fn(&mut SpecialModelQuad)); 4] = [
                ("non-finite position", |q| q.positions[0][0] = f32::NAN),
                ("non-finite UV", |q| q.uv[0][0] = f32::INFINITY),
                ("light", |q| q.light_emission = 16),
                ("tint", |q| q.tint_index = Some(u32::MAX)),
            ];
            for (message, invalidate) in invalid {
                let mut invalid_quad = quad.clone();
                invalidate(&mut invalid_quad);
                match context.bake_geometry(&[invalid_quad]) {
                    Err(error) if error.contains(message) => {}
                    _ => return Err(format!("did not reject invalid {message}")),
                }
            }
            *leases.lock().unwrap() = Some((resource, custom.clone(), fallbacks));
            Ok(Pan {
                b: custom,
                ..Pan::bake(pan, context)?
            })
        }
    };
    let factories = BTreeMap::from([(
        mod_api::Identifier::parse("spt:pan").unwrap(),
        owned_factory(|config| config.decode::<PanConfig>(), bake),
    )]);
    let renderer = renderer(temp.path(), 2, 2, &factories);
    assert!(renderer.models.definitions.contains_key("spt:pan"));
    let (resource, custom, fallbacks) = leases.lock().unwrap().take().unwrap();
    let resource = resource.downcast_ref::<BakedSpecialGeometry>().unwrap();
    assert_eq!(resource.model.quads.len(), 12);
    let custom = custom.downcast_ref::<BakedSpecialGeometry>().unwrap();
    assert_eq!(
        custom.extents.as_slice(),
        [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y]
    );
    let [quad] = custom.model.quads.as_slice() else {
        panic!("expected one custom quad")
    };
    assert_eq!(quad.facing, Direction::South);
    assert_eq!(quad.material.sprite, "spt:item/grey");
    assert!(quad.material.force_translucent);
    assert_eq!(quad.tint_index, 2);
    assert_eq!(quad.light_emission, 12);

    let fallbacks = fallbacks.downcast_ref::<BakedSpecialGeometry>().unwrap();
    let [degenerate, outside_uv, missing_sprite] = fallbacks.model.quads.as_slice() else {
        panic!("expected three fallback quads");
    };
    assert_eq!(degenerate.facing, Direction::Up);
    assert_eq!(
        outside_uv.uv,
        [[-1.0, 2.5], [1.5, -0.25], [2.0, 3.0], [-0.5, 1.25]]
    );
    assert_eq!(
        missing_sprite.material.sprite,
        sunrise_assets::item_bake::MISSING_SPRITE
    );
    assert!(missing_sprite.material.force_translucent);

    let mut stack = stack("spt:pan", 2);
    stack.patch.insert(
        "minecraft:enchantment_glint_override".into(),
        Some(serde_json::json!(true)),
    );
    let state = renderer.resolve(
        &stack,
        DisplayContext::Gui,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    let quads = quads(&renderer, &state);
    let [quad] = quads.as_slice() else {
        panic!("expected the submitted custom quad");
    };
    assert_eq!(quad.light_emission, 12);
    assert_eq!(
        quad.color,
        u32::MAX,
        "the indexed palette has no entry for tint 2"
    );
    assert_eq!(quad.foil, Foil::Standard);
    assert!(quad.translucent);
}

#[test]
fn child_poses_leave_following_submissions_in_their_own_scopes() {
    let root = tempfile::tempdir().unwrap();
    let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    let state = renderer.resolve(
        &stack("spt:pan", 25),
        DisplayContext::None,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    let quads = quads(&renderer, &state);
    assert_eq!(quads.len(), 18);
    // The 1/4-wide cube, centered by -1/2 after the local pose: translated by 1/4 and
    // doubled in the child, only translated in its parent, then unposed at the root.
    for (cube, expected) in
        quads
            .as_chunks::<6>()
            .0
            .iter()
            .zip([(-0.25, 0.25), (-0.25, 0.0), (-0.5, -0.25)])
    {
        let bounds = cube
            .iter()
            .flat_map(|quad| quad.pos)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), pos| {
                (lo.min(pos.x), hi.max(pos.x))
            });
        assert_eq!(bounds, expected);
    }
}

#[test]
fn a_bundle_extracts_the_selected_stacks_argument_and_foil() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "assets/spt/items/bundle.json",
        r#"{"model":{"type":"bundle/selected_item"}}"#,
    );
    let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    let mut bundle = stack("spt:bundle", 1);
    bundle.bundle_selected = 1;
    bundle
        .patch
        .insert("spt:tint".into(), Some(serde_json::json!(GREY)));
    bundle.patch.insert(
        "minecraft:bundle_contents".into(),
        Some(serde_json::json!([
            {"id": "spt:pan", "count": 1, "components": {"minecraft:item_model": "spt:pan"}},
            {"id": "spt:pan", "count": 2, "components": {
                "minecraft:item_model": "spt:pan",
                "minecraft:enchantment_glint_override": true,
                "spt:tint": 0xFFAA_1100_u32 as i32
            }}
        ])),
    );
    let state = renderer.resolve(
        &bundle,
        DisplayContext::Gui,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    // Two pans draw B, not A. The tint only comes from a pan's own component and foil only
    // from the selected stack's override; the outer bundle and first entry are not foiled.
    let drawn: Vec<_> = quads(&renderer, &state)
        .into_iter()
        .map(|quad| (quad.color, quad.foil))
        .collect();
    assert_eq!(drawn, [(0xFFAA_1100, Foil::Standard); 12]);
}

#[test]
#[ignore = "daily"]
fn a_registered_special_model_draws_its_renderers_resource_models() {
    let root = std::env::temp_dir().join(format!("sunrise-special-parts-{}", std::process::id()));
    write_composite(&root);
    write(
        &root,
        "assets/spt/items/fault.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:fault","handle":2}}}"#,
    );
    let mut factories = pan_factories(|_| {});
    factories.insert(
        mod_api::Identifier::parse("spt:fault").unwrap(),
        owned_factory(|config| config.decode::<PanConfig>(), Pan::bake),
    );
    let renderer = renderer(&root, 2, 2, &factories);
    let env = PanEnv(FakeHost::new());
    let resolve = |item: &str, count, display| {
        renderer.resolve(&stack(item, count), display, None, None, 0, &env)
    };

    // A or B, in the inventory and in hand, tinted and with the base's particle and GUI light.
    for display in [DisplayContext::Gui, DisplayContext::FirstPersonRightHand] {
        let one = resolve("spt:pan", 1, display);
        let two = resolve("spt:pan", 2, display);
        assert_eq!(quads(&renderer, &one).len(), 6, "{display:?}");
        assert_eq!(quads(&renderer, &two).len(), 12, "{display:?}");
        assert!(
            quads(&renderer, &two)
                .iter()
                .all(|q| q.color == 0xFFCC_2211)
        );
        assert_eq!(two.layers[0].particle.as_ref(), "spt:item/flour");
        assert!(Arc::ptr_eq(
            &one.layers[0].particle,
            &two.layers[0].particle
        ));
        assert!(!two.uses_block_light());
        assert!(
            two.animated,
            "a mod's drawing is never kept as a static GUI item"
        );
    }

    // B on the ground, in a frame, on a head and in another player's hand: the base's
    // transform for each context, so each draws B somewhere else.
    let contexts = [
        DisplayContext::Ground,
        DisplayContext::Fixed,
        DisplayContext::Head,
        DisplayContext::ThirdPersonRightHand,
    ];
    let matrices: Vec<Mat4> = contexts
        .iter()
        .map(|&display| {
            let state = resolve("spt:pan", 2, display);
            assert_eq!(quads(&renderer, &state).len(), 12, "{display:?}");
            state.layers[0].matrix
        })
        .collect();
    for (i, a) in matrices.iter().enumerate() {
        for b in &matrices[i + 1..] {
            assert_ne!(a, b);
        }
    }
    // A part's pose is about the model's corner, as its elements are: A's cube spans x 0 to 1/4
    // there, moved a quarter block or doubled; the drawn item then sits half a block back.
    let x = |count| {
        let (lo, hi) = resolve("spt:pan", count, DisplayContext::None).bounding_box();
        (lo.x, hi.x)
    };
    assert_eq!(x(1), (-0.25, 0.0));
    assert_eq!(x(5), (-0.5, 0.0));

    // An unreadable nested stack discards the geometry already submitted, without poisoning
    // the parent. The next valid stack of that type still draws.
    let missing = renderer.bake(MISSING_MODEL);
    let is_missing_layer = |layer: &LayerRender| matches!(&layer.geometry, LayerGeometry::Quads(baked) if Arc::ptr_eq(baked, &missing));
    let is_missing = |state: &ItemRenderState| matches!(state.layers.as_slice(), [layer] if is_missing_layer(layer));
    assert!(is_missing(&resolve("spt:pan", 3, DisplayContext::Gui)));
    // A stack part draws the inner item through the resolver, under its own display context and
    // tints and foil, not the part's: the cube's six glinting grey quads beside B's twelve
    // tinted ones. A stack part that names the stack being drawn is a cycle, refused whole.
    let nested = resolve("spt:pan", 8, DisplayContext::Fixed);
    let draws: Vec<_> = quads(&renderer, &nested)
        .iter()
        .map(|q| (q.color, q.foil))
        .collect();
    assert_eq!(nested.layers[0].display, DisplayContext::Gui);
    assert_eq!(nested.layers[1].display, DisplayContext::Fixed);
    assert_eq!(draws[..6], [(GREY as u32, Foil::Standard); 6]);
    assert_eq!(draws[6..], [(0xFFCC_2211, Foil::None); 12]);
    assert!(nested.animated);
    assert!(is_missing(&resolve("spt:pan", 9, DisplayContext::Gui)));
    // A refusal below a first level refuses the answers above it whole, their valid sibling
    // models too; there is no nested-stack count cap, and only `(stack, display)` cycles refuse.
    assert!(is_missing(&resolve("spt:pan", 10, DisplayContext::Gui)));
    assert!(is_missing(&resolve("spt:pan", 11, DisplayContext::Gui)));
    let composite = resolve("spt:composite", 12, DisplayContext::Gui);
    let [before, replacement, after] = composite.layers.as_slice() else {
        panic!("the failed parent and child must leave just one replacement between siblings");
    };
    assert!(is_missing_layer(replacement));
    assert!(matches!(&before.geometry, LayerGeometry::Quads(baked) if baked.quads.len() == 6));
    assert!(matches!(&after.geometry, LayerGeometry::Quads(baked) if baked.quads.len() == 12));
    assert_eq!(before.tints.at(0), GREY as u32);
    assert_eq!(after.tints.at(0), WHITE as u32);
    assert_eq!(
        quads(&renderer, &resolve("spt:pan", 1, DisplayContext::Gui)).len(),
        6
    );
    assert!(is_missing(&resolve("spt:pan", 14, DisplayContext::Gui)));
    // A nested refusal aborts only the current layer; the same type can draw a later item.
    assert_eq!(
        quads(&renderer, &resolve("spt:fault", 1, DisplayContext::Gui)).len(),
        6
    );
    let quads_of = |count| quads(&renderer, &resolve("spt:pan", count, DisplayContext::Gui)).len();
    assert_eq!(quads_of(20), 64 * 6);
    assert_eq!(quads_of(21), 65 * 6);
    assert_eq!(quads_of(22), 12 + 64 * 6);
    assert_eq!(quads_of(23), 12 + 63 * 6);
    assert_eq!(
        quads(&renderer, &resolve("spt:pan", 26, DisplayContext::Fixed)).len(),
        6
    );
    assert!(!is_missing(&resolve("spt:pan", 1, DisplayContext::Gui)));
    assert_eq!(
        quads(&renderer, &resolve("spt:cube", 1, DisplayContext::Gui)).len(),
        6
    );

    // A special type no mod registered isn't loaded: its item draws the missing model.
    assert!(is_missing(&resolve("spt:stranger", 1, DisplayContext::Gui)));

    // A reload that makes B three cubes draws 18 quads for the same stack.
    let reloaded = super::special_parts_tests::renderer(&root, 3, 2, &factories);
    let state = reloaded.resolve(
        &stack("spt:pan", 2),
        DisplayContext::Gui,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    assert_eq!(quads(&reloaded, &state).len(), 18);
    fs::remove_dir_all(root).unwrap();
}

/// An old special item renderer state across a reload. A part's quads are CPU vertices submitted
/// every frame, not prepared GPU models, so the
/// state's own `Arc<BakedItemModel>` is the only lease and there is no retirement step: a state
/// resolved before a reload still draws its 12 quads after it, the old baked model is released
/// exactly once, when the last state holding it drops, and a new resolve draws the reload's 18.
#[test]
#[ignore = "daily"]
fn an_old_state_draws_across_a_reload_and_releases_its_models_once() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-special-parts-lease-{}",
        std::process::id()
    ));
    let factories = pan_factories(|_| {});
    let before = renderer(&root, 2, 2, &factories);
    let resolve = |renderer: &ItemRenderer| {
        renderer.resolve(
            &stack("spt:pan", 2),
            DisplayContext::Gui,
            None,
            None,
            0,
            &PanEnv(FakeHost::new()),
        )
    };
    let (old, also_old) = (resolve(&before), resolve(&before));
    let LayerGeometry::Quads(b) = &old.layers[0].geometry else {
        panic!("not quads: {:?}", old.layers[0].geometry);
    };
    let b = Arc::downgrade(b);
    let after = renderer(&root, 3, 2, &factories);
    drop(before);
    assert_eq!(quads(&after, &old).len(), 12);
    assert_eq!(quads(&after, &resolve(&after)).len(), 18);
    assert_eq!(b.strong_count(), 2, "the two old states hold the old B");
    drop(old);
    assert_eq!(b.strong_count(), 1);
    assert_eq!(quads(&after, &also_old).len(), 12);
    drop(also_old);
    assert_eq!(b.strong_count(), 0, "the old B outlived its last state");
    fs::remove_dir_all(root).unwrap();
}

/// Row 11 of the matrix, a part's tint. It is packed vanilla-space ARGB, as an item model's
/// tints are: white and an encoded 0.5 reach the vertices as the colour vanilla's constant tint
/// source gives the same cube, not decoded, so the frame decodes them once (vanilla's power 2.2:
/// shader values 1 and about 0.21763764, pinned by `gfx::linear_blend_tests`). A value above 1 has
/// no input.
#[test]
#[ignore = "daily"]
fn a_parts_tint_reaches_the_vertices_as_a_vanilla_tint_does() {
    let root =
        std::env::temp_dir().join(format!("sunrise-special-parts-tint-{}", std::process::id()));
    let factories = pan_factories(|_| {});
    let renderer = renderer(&root, 2, 2, &factories);
    let env = PanEnv(FakeHost::new());
    let colours = |item: &str, count| {
        let state = renderer.resolve(
            &stack(item, count),
            DisplayContext::Gui,
            None,
            None,
            0,
            &env,
        );
        quads(&renderer, &state)
            .iter()
            .map(|q| q.color)
            .collect::<Vec<_>>()
    };
    for (count, vanilla, argb) in [(6, "spt:white_cube", WHITE), (7, "spt:grey_cube", GREY)] {
        let part = colours("spt:pan", count);
        assert_eq!(part, colours(vanilla, 1), "{vanilla}");
        assert_eq!(part, vec![argb as u32; 6], "{vanilla}");
    }
    fs::remove_dir_all(root).unwrap();
}

/// Registration and resource lookup both use `Identifier::parse`: bare paths default to
/// `minecraft:` and a valid empty path (`example:`) reaches its registered factory.
#[test]
fn special_model_lookup_accepts_bare_path_and_empty_path() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "assets/spt/items/bare.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"skillet","handle":1}}}"#,
    );
    write(
        root.path(),
        "assets/spt/items/empty_path.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"example:","handle":2}}}"#,
    );
    let factory = || owned_factory(|config| config.decode::<PanConfig>(), Pan::bake);
    let factories = BTreeMap::from([
        (mod_api::Identifier::parse("skillet").unwrap(), factory()),
        (mod_api::Identifier::parse("example:").unwrap(), factory()),
    ]);
    let renderer = renderer(root.path(), 2, 2, &factories);
    let env = PanEnv(FakeHost::new());
    for item in ["spt:bare", "spt:empty_path"] {
        let state = renderer.resolve(&stack(item, 1), DisplayContext::Gui, None, None, 0, &env);
        assert_eq!(quads(&renderer, &state).len(), 6, "{item}");
    }
}

/// Gap 33: reusing one baked lease with distinct per-submission packed light (including a smooth
/// byte) and `LightmapOnly`, followed by an inherited neighbour with baked `light_emission`,
/// emits exact vertex light coordinates across frame times.
#[test]
fn special_model_per_submission_packed_light_and_lightmap_only_reach_world_sink_exactly() {
    use mod_api::{SpecialModelLighting, SpecialModelQuad, SpecialModelSubmitOptions};

    struct LitParts(SpecialModelGeometry);
    impl SpecialModelRenderer for LitParts {
        type Argument = ();

        fn extract_argument(&self, _: &dyn ClientHost, _: &SpecialModelStack<'_>) {}

        fn submit(
            &self,
            _: &(),
            context: &SpecialModelContext,
            collector: &mut SpecialModelCollector<'_>,
        ) -> Result<(), String> {
            let smooth_block: u32 = if context.game_time == Some(20) {
                0x45
            } else {
                0x37
            };
            let packed_a = (0xA0u32 << 16) | smooth_block;
            let packed_b = (0xF0u32 << 16) | 0x10u32;
            collector.submit_model(
                &self.0,
                SpecialModelTints::Untinted,
                SpecialModelSubmitOptions::default()
                    .with_packed_light(packed_a)
                    .with_lighting(SpecialModelLighting::LightmapOnly),
            )?;
            collector.submit_model(
                &self.0,
                SpecialModelTints::Untinted,
                SpecialModelSubmitOptions::default()
                    .with_packed_light(packed_b)
                    .with_lighting(SpecialModelLighting::Diffuse),
            )?;
            collector.submit_model(
                &self.0,
                SpecialModelTints::Untinted,
                SpecialModelSubmitOptions::default(),
            )
        }
    }

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "assets/spt/items/lit.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:lit"}}}"#,
    );
    let factory = owned_factory(
        |_| Ok(()),
        |(), bake| {
            let mut quad = SpecialModelQuad::new(
                [
                    [0.0, 0.0, 0.5],
                    [0.0, 1.0, 0.5],
                    [1.0, 1.0, 0.5],
                    [1.0, 0.0, 0.5],
                ],
                [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
                "spt:item/grey",
            );
            quad.light_emission = 12;
            Ok(LitParts(bake.bake_geometry(&[quad])?))
        },
    );
    let factories = BTreeMap::from([(mod_api::Identifier::parse("spt:lit").unwrap(), factory)]);
    let renderer = renderer(root.path(), 2, 2, &factories);
    let sprites = HashMap::from([(
        "spt:item/grey".to_owned(),
        crate::hud::Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        },
    )]);
    let draw_light = 0x50u32 | (0x20u32 << 8);
    let lights = Some((
        Vec3::new(0.2, 1.0, -0.7).normalize(),
        Vec3::new(-0.2, 1.0, 0.7).normalize(),
    ));
    let env = PanEnv(FakeHost::new());

    for (game_time, smooth_block) in [(10i64, 0x37u32), (20i64, 0x45u32)] {
        let level = LevelState {
            game_time,
            ..Default::default()
        };
        let state = renderer.resolve(
            &stack("spt:lit", 1),
            DisplayContext::ThirdPersonRightHand,
            None,
            Some(&level),
            0,
            &env,
        );
        let mut verts = Vec::new();
        let mut sink = WorldSink {
            out: &mut verts,
            light: draw_light,
            lights,
            glint: None,
            tier: 0,
        };
        state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut sink);
        assert_eq!(verts.len(), 12, "3 quads * 4 vertices");
        // Quad 0: explicit smooth packed light (no baked emission floor) + LightmapOnly (unshaded).
        assert_eq!(verts[0].light, 0xA0 | (smooth_block << 8) | (1 << 16));
        assert_eq!(verts[0].color, u32::MAX);
        // Quad 1: explicit packed light (no baked emission floor) + Diffuse.
        assert_eq!(verts[4].light, 0xF0 | (0x10 << 8) | (1 << 16));
        // Quad 2: inherited draw sky (0x50) + baked emission floor (12 * 16 = 0xC0).
        assert_eq!(verts[8].light, 0x50 | (0xC0 << 8));
    }
}

/// Gap 33: submitting `ItemTranslucent` then `ItemCutout` (reverse phase order) on the same
/// baked lease with a partially transparent sprite routes each submission to its explicit
/// material in `WorldSink`, independent of the sprite translucency classifier.
#[test]
fn special_model_explicit_render_type_overrides_sprite_translucency() {
    use mod_api::{SpecialModelQuad, SpecialModelRenderType, SpecialModelSubmitOptions};

    struct TwoSheets(SpecialModelGeometry);
    impl SpecialModelRenderer for TwoSheets {
        type Argument = ();

        fn extract_argument(&self, _: &dyn ClientHost, _: &SpecialModelStack<'_>) {}

        fn submit(
            &self,
            _: &(),
            _: &SpecialModelContext,
            collector: &mut SpecialModelCollector<'_>,
        ) -> Result<(), String> {
            // Submit translucent first, then cutout (reverse phase order).
            collector.submit_model(
                &self.0,
                SpecialModelTints::Untinted,
                SpecialModelSubmitOptions::default()
                    .with_render_type(SpecialModelRenderType::ItemTranslucent),
            )?;
            collector.submit_model(
                &self.0,
                SpecialModelTints::Untinted,
                SpecialModelSubmitOptions::default()
                    .with_render_type(SpecialModelRenderType::ItemCutout),
            )
        }
    }

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "assets/spt/items/sheets.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:sheets"}}}"#,
    );
    let factory = owned_factory(
        |_| Ok(()),
        |(), bake| {
            let quad = SpecialModelQuad::new(
                [
                    [0.0, 0.0, 0.5],
                    [0.0, 1.0, 0.5],
                    [1.0, 1.0, 0.5],
                    [1.0, 0.0, 0.5],
                ],
                [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
                "spt:item/glass",
            );
            Ok(TwoSheets(bake.bake_geometry(&[quad])?))
        },
    );
    let factories = BTreeMap::from([(mod_api::Identifier::parse("spt:sheets").unwrap(), factory)]);
    let renderer = renderer(root.path(), 2, 2, &factories);
    let sprites = HashMap::from([(
        "spt:item/glass".to_owned(),
        crate::hud::Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        },
    )]);
    let env = PanEnv(FakeHost::new());
    let state = renderer.resolve(
        &stack("spt:sheets", 1),
        DisplayContext::Ground,
        None,
        None,
        0,
        &env,
    );
    let mut verts = Vec::new();
    let mut sink = WorldSink {
        out: &mut verts,
        light: 0xF0 | (0xF0 << 8),
        lights: None,
        glint: None,
        tier: 0,
    };
    state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut sink);
    assert_eq!(verts.len(), 8);
    assert_eq!(
        verts[0].layer & 0xFFFF,
        crate::gfx::TRANSLUCENT_ITEM_OVERLAY,
        "explicit ItemTranslucent must route to translucent item batch"
    );
    assert_eq!(
        verts[4].layer & 0xFFFF,
        0,
        "explicit ItemCutout must route to solid/cutout item batch even on a translucent sprite"
    );
}
