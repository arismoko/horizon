use crate::game::{ClientGame, stock_layer_mask::StockLayerMask};
use std::{collections::HashMap, sync::Arc};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_client::trims::{TrimOverride, TrimRegistry};
use sunrise_client::{
    cit::{Hand, Kind, Rule},
    mesher::Vertex,
    mobs::{
        Part,
        render::{WornLayer, WornModel},
        skin::Paint,
    },
    trims::TrimTexture,
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::armor_stands_common::{
    TYPE_ID, skull_kind, swap_with_equipment_slot,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    entities::ArmorStand,
    mobs::{
        Mob,
        anim::Poses,
        finalize::{self, EquipmentRead},
    },
};

/// `EquipmentLayerRenderer.getColorForLayer`: a dyeable layer takes the stack's
/// `minecraft:dyed_color`, else its optional undyed colour, else zero to skip it.
/// Nondyeable layers stay white. The result uses the renderer's byte order (R and B swapped).
pub(in crate::game) fn layer_color(
    dyeable: bool,
    undyed: Option<u32>,
    stack: Option<&ItemStack>,
) -> u32 {
    let dyed = stack
        .and_then(|s| s.data())
        .and_then(|d| d.dyed_color)
        .map(|c| c as u32 & 0xFF_FFFF);
    if !dyeable {
        return 0xFFFF_FFFF;
    }
    let Some(c) = dyed.or(undyed) else {
        return 0;
    };
    0xFF00_0000 | (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
}

/// Armor stand models and the humanoid layers of every equipment asset.
#[derive(Default)]
pub struct ArmorStandAssets {
    /// Shared (`Arc`) so the GPU path (`mobs::skin`) keeps one stored copy of each.
    pub main: Option<Arc<Part>>,
    pub small: Option<Arc<Part>>,
    /// (slot model suffix, small) -> model.
    pub layers: HashMap<(&'static str, bool), Arc<Part>>,
    /// The player's armour models by slot (helmet, chestplate, leggings, boots).
    pub player: HashMap<&'static str, Arc<Part>>,
    /// Asset id -> layer kind -> ordered textures with dyeability and optional undyed colour.
    pub(in crate::game) equipment: super::equipment_piece::EquipmentAssets,
    pub extensions: super::client_item_extensions::ArmorExtensions,
    /// asset id -> `trim_overrides`.
    pub trim_overrides: HashMap<String, Vec<TrimOverride>>,
    pub trims: TrimRegistry,
    /// The resources the trim textures are made from.
    pub root: ResourceSource,
}
#[cfg(test)]
impl ArmorStandAssets {
    pub(in crate::game) fn clone_for_tests(&self) -> Self {
        Self {
            main: self.main.clone(),
            small: self.small.clone(),
            layers: self.layers.clone(),
            player: self.player.clone(),
            equipment: self.equipment.clone(),
            extensions: Default::default(),
            trim_overrides: self.trim_overrides.clone(),
            trims: self.trims.clone(),
            root: self.root.clone(),
        }
    }
}
impl ArmorStandAssets {
    pub fn load(
        root: &ResourceSource,
        document: Option<&sunrise_client::mobs::EntityModelDocument>,
    ) -> Self {
        let load = |key: &str| document.and_then(|document| document.model(key));
        let mut layers = HashMap::new();
        let mut player = HashMap::new();
        for slot in ["helmet", "chestplate", "leggings", "boots"] {
            if let Some(part) = load(&format!("minecraft:player#{slot}")) {
                player.insert(slot, Arc::new(part));
            }
            for (small, model) in [(false, "armor_stand"), (true, "armor_stand_small")] {
                if let Some(part) = load(&format!("minecraft:{model}#{slot}")) {
                    layers.insert((slot, small), Arc::new(part));
                }
            }
        }
        for (key, name) in [
            ("minecraft:elytra#main", "elytra"),
            ("minecraft:elytra_baby#main", "elytra_baby"),
        ] {
            if let Some(part) = load(key) {
                player.insert(name, Arc::new(part));
            }
        }
        let mut equipment = HashMap::new();
        let mut trim_overrides = HashMap::new();
        // EquipmentAssetManager reads resolved resources in every namespace.
        for namespace in root.namespaces("assets") {
            let prefix = format!("assets/{namespace}/equipment/");
            for path in root.list(&prefix) {
                let Some(name) = path
                    .strip_prefix(&prefix)
                    .and_then(|path| path.strip_suffix(".json"))
                else {
                    continue;
                };
                let Ok(v) = root
                    .read(&path)
                    .map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes))
                else {
                    continue;
                };
                let Ok(v) = v else { continue };
                let id = format!("{namespace}:{name}");
                let mut kinds = HashMap::new();
                for kind in [
                    "humanoid",
                    "humanoid_leggings",
                    "humanoid_baby",
                    "wings",
                    "horse_body",
                    "nautilus_body",
                ] {
                    // Every layer of the kind: leather's overlay is layer 1.
                    let layers: Vec<super::equipment_piece::EquipmentAssetLayer> = v["layers"]
                        [kind]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|layer| {
                            let tex = layer["texture"].as_str().unwrap_or(&id);
                            let (tex_namespace, tex) =
                                tex.split_once(':').unwrap_or(("minecraft", tex));
                            let sprite = match tex_namespace {
                                "minecraft" => format!("entity/equipment/{kind}/{tex}"),
                                other => format!("{other}:entity/equipment/{kind}/{tex}"),
                            };
                            let undyed = layer["dyeable"]["color_when_undyed"]
                                .as_i64()
                                .map(|c| c as u32 & 0xFF_FFFF);
                            super::equipment_piece::EquipmentAssetLayer {
                                texture: sprite,
                                dyeable: layer
                                    .get("dyeable")
                                    .is_some_and(serde_json::Value::is_object),
                                color_when_undyed: undyed,
                                use_player_texture: layer["use_player_texture"]
                                    .as_bool()
                                    .unwrap_or(false),
                            }
                        })
                        .collect();
                    if !layers.is_empty() {
                        kinds.insert(kind.to_owned(), layers);
                    }
                }
                trim_overrides.insert(id.clone(), TrimOverride::list(&v));
                equipment.insert(id, kinds);
            }
        }
        Self {
            main: load("minecraft:armor_stand#main").map(Arc::new),
            small: load("minecraft:armor_stand_small#main").map(Arc::new),
            layers,
            player,
            equipment,
            extensions: Default::default(),
            trim_overrides,
            trims: TrimRegistry::load(root),
            root: root.to_owned(),
        }
    }
}
/// The stand's slot (`ArmorStand::SLOTS` order) for an item whose
/// `equippable` slot is `equip` (`SmithingScreen.updateArmorStandPreview`):
/// worn in a head, chest, legs or feet slot, else (no `equippable`, or any
/// other slot: main hand, off hand, body, saddle) held in the left hand.
pub(in crate::game) fn preview_slot(equip: Option<&str>) -> usize {
    match equip {
        Some("feet") => 2,
        Some("legs") => 3,
        Some("chest") => 4,
        Some("head") => 5,
        _ => 1,
    }
}
/// The `Pose` rotations on the parts of an armor stand model or its armor
/// (`ArmorStandArmorModel.setupAnim`); the body also turns the sticks.
pub(in crate::game) fn apply_pose(poses: &mut Poses, pose: &[[f32; 3]; 6]) {
    let parts: [&[&str]; 6] = [
        &["head"],
        &[
            "body",
            "right_body_stick",
            "left_body_stick",
            "shoulder_stick",
        ],
        &["left_arm"],
        &["right_arm"],
        &["left_leg"],
        &["right_leg"],
    ];
    for (names, rotation) in parts.into_iter().zip(pose) {
        for name in names {
            if poses.has(name) {
                let part = poses.get(name);
                part[3] = rotation[0].to_radians();
                part[4] = rotation[1].to_radians();
                part[5] = rotation[2].to_radians();
            }
        }
    }
}

impl ArmorStandAssets {
    /// Trim data of reloaded resources (the models stay).
    pub fn apply_resources(&mut self, fresh: Self) {
        self.equipment = fresh.equipment;
        self.extensions = fresh.extensions;
        self.trim_overrides = fresh.trim_overrides;
        self.trims.reloaded(fresh.trims);
        self.root = fresh.root;
    }
    /// `TrimTextureKey`: the trim texture of `stack` on its humanoid
    /// layer (`humanoid` or `humanoid_leggings`), if it has a trim.
    pub(in crate::game::client_side) fn trim_texture(
        &self,
        asset: &str,
        kind: &str,
        trim: &sunrise_items::item_data::Trim,
    ) -> Option<TrimTexture> {
        let (material, pattern) = sunrise_items::item_data::trim::named(trim)?;
        let overrides = self
            .trim_overrides
            .get(asset)
            .map_or(&[][..], Vec::as_slice);
        TrimTexture::resolve(&self.trims, overrides, material, pattern, kind)
    }
}

impl ClientGame<'_> {
    /// The layers of the armour items `worn` (with their stacks when known,
    /// for trims and CIT armour rules), head to feet.
    fn armor_layers(
        &self,
        worn: [Option<EquipmentRead<'_>>; 4],
        hidden_layers: StockLayerMask,
    ) -> Vec<WornLayer<'_>> {
        let assets = &self.client.armor_stand_assets.0;
        let mut out = Vec::new();
        for (item, (layer, slot)) in worn.into_iter().zip([
            ("helmet", "head"),
            ("chestplate", "chest"),
            ("leggings", "legs"),
            ("boots", "feet"),
        ]) {
            let Some(stack) = item else {
                continue;
            };
            let Some(piece) = self.equipment_piece(&stack, slot, layer == "leggings", false) else {
                continue;
            };
            let Some(part) = assets.player.get(layer) else {
                continue;
            };
            for material in self.equipment_materials(
                &piece,
                &stack,
                super::equipment_piece::EquipmentPass::Draw,
            ) {
                let Some(overlay) = material.visible_overlay(piece.slot, hidden_layers) else {
                    continue;
                };
                out.push((
                    material.model.map_or(WornModel::Vanilla(part), |model| {
                        WornModel::Override(model, piece.slot)
                    }),
                    material.binding.uv,
                    material.color,
                    overlay,
                    material.glows,
                ));
            }
        }
        out
    }
    /// ArmorStandRenderer: the stand with its default pose, then armor layers.
    pub(in crate::game::client_side) fn armor_stand_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        // The stands' models posed on the GPU path (`mobs::skin`), when it runs this frame.
        let skinning = self.skinning();
        for a in self.common.armor_stands[run]
            .iter()
            .filter(|a| !self.hides_as_camera(a.id))
        {
            // ArmorStand.shouldRenderAtSqrDistance: 256 times the box's mean
            // edge (4 for a marker's empty box), whatever the entity distance.
            // LivingEntityRenderer grows the culling box by 0.5 for a dragon head.
            let (width, height) = match (a.marker, a.small) {
                (true, _) => (0.0, 0.0),
                (false, true) => (0.25, 0.9875),
                (false, false) => (0.5, 1.975),
            };
            let edge = (width + height + width) / 3.0 * 4.0;
            // 26.3 compares the size with RenderSystem.DEFAULT_DEPTH_CLEAR_VALUE,
            // which is 0.0 (reverse Z; the bytecode is `dconst_0`): an empty
            // marker box falls back to 4, so a visible marker stand draws.
            // Holograms stay hidden through the invisible flag instead.
            let range = if edge == 0.0 { 4.0 } else { edge } * 64.0;
            let dragon = a.equipment[5]
                .as_ref()
                .is_some_and(|i| &*i.item == "minecraft:dragon_head");
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(a.previous),
                super::entity_culling::at(a.position),
                alpha,
                width,
                height,
                range,
                if dragon { 0.5 } else { 0.0 },
            ) {
                continue;
            }
            let culling = self.entity_cull_decision(a, alpha);
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(
                entity_start,
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(a.id),
            );
            // EntityRenderer: drawn between last tick's position and this one's.
            let at = a.render_position(alpha);
            let p = super::entity_culling::at(at) - camera;
            // setupRotations: 180 - body yaw, plus the wobble for 5 ticks after a hit.
            let wiggle = (self.common.world.game_time - a.last_hit) as f32 + alpha as f32;
            let wobble = if wiggle < 5.0 {
                (wiggle / 1.5 * std::f32::consts::PI).sin() * 3.0
            } else {
                0.0
            };
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y(
                    (180.0 - a.render_yaw(alpha as f32) + wobble).to_radians(),
                )
                * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
            // `EntityRenderer.getPackedLightCoords`: the block
            // `ArmorStand.getLightProbePosition` gives - the brightest cell of a
            // marker's own box, else the stand's eye - at 15 block light while
            // it burns.
            let probe = a.light_probe(alpha, |x, y, z| self.common.world.light(x, y, z));
            let light = self.common.world.light(probe[0], probe[1], probe[2]);
            let packed = a.render_light(light);
            let skinned = if skinning {
                let mut models = self.client.skin_models.0.borrow_mut();
                let mut frame = self.client.skin_frame.borrow_mut();
                let mark = frame.mark();
                let mut skin = sunrise_client::mobs::skin::Skin::new(&mut models, &mut frame);
                self.armor_stand_vertices(
                    a,
                    Some(alpha as f32),
                    base,
                    packed,
                    Some(&mut skin),
                    out,
                );
                Some((mark, skin.bounds))
            } else {
                self.armor_stand_vertices(a, Some(alpha as f32), base, packed, None, out);
                None
            };
            if !self.tracked_entity_glowing(a.id) {
                self.cull_skinned(out, entity_start, skinned);
            }
        }
    }
    /// `ItemInHandLayer` and `CustomHeadLayer` of an armour stand: the hand
    /// items in the third-person contexts, and a non-armour head item on the
    /// head (`translateToHead`: down a quarter, turned, 0.625 scale). With
    /// `skin`, items that can are posed on the GPU with the stand.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::client_side) fn armor_stand_items(
        &self,
        a: &ArmorStand,
        model: &Part,
        base: glam::Mat4,
        poses: &Poses,
        world_partial: Option<f32>,
        light: u32,
        mut skin: Option<&mut sunrise_client::mobs::skin::Skin>,
        out: &mut Vec<Vertex>,
    ) {
        let lights = sunrise_client::block_entity_render::diffuse_lights(self.nether_lighting());
        let mut submit = |state: &sunrise_client::item_render::ItemRenderState,
                          pose: glam::Mat4,
                          out: &mut Vec<Vertex>| {
            let on_gpu = skin
                .as_deref_mut()
                .is_some_and(|skin| self.skin_item(state, pose, light, lights, skin));
            if !on_gpu {
                self.submit_item_world(state, pose, light, out);
            }
        };
        use sunrise_assets::item_model::DisplayContext;
        use sunrise_client::mobs::render::part_matrix;
        let owner = super::item_draw::owner_at(
            [a.position.x, a.position.y, a.position.z],
            a.yaw,
            "minecraft:armor_stand",
        );
        // `useBabyOffset`: small stands use the baby hand offsets.
        let (ox, oy, oz) = if a.small {
            (0.0, 1.0, -4.5)
        } else {
            (1.0, 2.0, -10.0)
        };
        for (slot, left, context) in [
            (0usize, false, DisplayContext::ThirdPersonRightHand),
            (1, true, DisplayContext::ThirdPersonLeftHand),
        ] {
            let Some(stack) = &a.equipment[slot] else {
                continue;
            };
            let arm = if left { "left_arm" } else { "right_arm" };
            let Some(hand) = part_matrix(model, base, poses, arm, 0.0) else {
                continue;
            };
            let pose = hand
                * glam::Mat4::from_rotation_x((-90.0f32).to_radians())
                * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                * glam::Mat4::from_translation(glam::Vec3::new(
                    if left { -ox } else { ox } / 16.0,
                    oy / 16.0,
                    oz / 16.0,
                ));
            let seed = a.id as i32 + if left { 1 } else { 2 };
            // OptiFine CIT `hand=off` rules: slot 1 is the off hand.
            let mut state = if left {
                self.resolve_item_in_hand(stack, context, Some(&owner), seed, Hand::Off)
            } else {
                self.resolve_item(stack, context, Some(&owner), seed)
            };
            if let Some(partial_tick) = world_partial {
                let hand = if left {
                    mod_api::Hand::Off
                } else {
                    mod_api::Hand::Main
                };
                self.tint_held_item(&mut state, stack, a.id, hand, partial_tick);
            }
            submit(&state, pose, out);
        }
        // `CustomHeadLayer`: a worn skull draws the skull model.
        if let Some(stack) = &a.equipment[5]
            && let Some(kind) =
                skull_kind(&self.client.client_pack.item_renderer.models, &stack.item)
            && let Some(head) = part_matrix(model, base, poses, "head", 0.0)
        {
            let pose = head * glam::Mat4::from_scale(glam::Vec3::splat(1.1875));
            let special = sunrise_assets::item_model::SpecialModel {
                kind: if kind == "player" {
                    sunrise_assets::item_model::SpecialKind::PlayerHead
                } else {
                    sunrise_assets::item_model::SpecialKind::Head {
                        kind: kind.to_owned(),
                        texture: None,
                        animation: 0.0,
                    }
                },
            };
            let mut sink = sunrise_client::item_render::WorldSink {
                out,
                light,
                lights: Some(sunrise_client::block_entity_render::diffuse_lights(
                    self.nether_lighting(),
                )),
                glint: self.glint(),
                tier: 0,
            };
            let profile = stack
                .extra
                .as_deref()
                .and_then(|d| {
                    d.preserved_components()
                        .iter()
                        .find(|(k, _)| k == "minecraft:profile")
                })
                .map(|(_, t)| self.client.skin_profiles.borrow_mut().head_profile_nbt(t))
                .unwrap_or_else(|| std::sync::Arc::new(serde_json::Value::Null));
            self.client.client_pack.item_renderer.submit_special(
                &special,
                &profile,
                false,
                pose,
                &self.client.gui.atlas.sprites,
                &self.item_host(),
                &mut sink,
            );
            return;
        }
        // A head item that is not worn armour.
        if let Some(stack) = &a.equipment[5]
            && !sunrise_sim::item_sync::equippable(stack, self.common.res.items())
                .is_some_and(|e| e.asset_id.is_some() && e.slot == "head")
            && let Some(head) = part_matrix(model, base, poses, "head", 0.0)
        {
            let pose = head
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.25, 0.0))
                * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                * glam::Mat4::from_scale(glam::Vec3::new(0.625, -0.625, -0.625));
            // `updateForLiving`: seeded by the entity id plus HEAD's ordinal.
            let state =
                self.resolve_item(stack, DisplayContext::Head, Some(&owner), a.id as i32 + 5);
            submit(&state, pose, out);
        }
    }
    /// `ArmorStandRenderer.submit` for stand `a` under `base` (its
    /// `setupRotations`): the body, the hand and head items, the armour and
    /// the wings, at packed light `packed`. `world_partial` is absent in the GUI preview.
    /// With `skin`, supported meshes are posed on the GPU.
    pub(in crate::game::client_side) fn armor_stand_vertices(
        &self,
        a: &ArmorStand,
        world_partial: Option<f32>,
        base: glam::Mat4,
        packed: u32,
        mut skin: Option<&mut sunrise_client::mobs::skin::Skin>,
        out: &mut Vec<Vertex>,
    ) {
        use sunrise_client::mobs::render::put;
        let assets = &self.client.armor_stand_assets.0;
        let sprites = &self.client.gui.atlas.sprites;
        let uv_of = |name: &str| sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // A texture's OptiFine emissive overlay. The lock is read per lookup and let go:
        // `wings_geometry` below reads it too.
        let glow_of = |texture: &str| {
            let emissive = self
                .client_res()
                .client_mob_data()
                .emissive
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            emissive.overlay(texture).and_then(uv_of)
        };
        let Some(wood) = uv_of("entity/armorstand/armorstand") else {
            return;
        };
        let Some(model) = (if a.small { &assets.small } else { &assets.main }) else {
            return;
        };
        let mut poses = Poses::new(model);
        let partial = world_partial.unwrap_or(0.0);
        self.setup_armor_stand_pose(a, partial, &mut poses);
        // Item layers pose from the hidden arms too (`visible` only skips drawing).
        let posed = poses.clone();
        let mut hide = |name: &str| {
            if poses.has(name) {
                let pose = poses.get(name);
                pose[6] = 0.0;
                pose[7] = 0.0;
                pose[8] = 0.0;
            }
        };
        if !a.show_arms {
            hide("left_arm");
            hide("right_arm");
        }
        if a.no_base_plate {
            hide("base_plate");
        }
        // LivingEntityRenderer.submit: an invisible stand has no body render type, but its
        // layers (held items, custom head, armour) are submitted all the same.
        if !a.invisible {
            put(
                &mut skin,
                model,
                base,
                &poses,
                wood,
                0.0,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            if let Some(glow) = glow_of("entity/armorstand/armorstand") {
                let full = sunrise_client::mobs::render::FULL_BRIGHT;
                put(
                    &mut skin,
                    model,
                    base,
                    &poses,
                    glow,
                    0.0,
                    0xFFFF_FFFF,
                    full,
                    0,
                    out,
                );
            }
        }
        self.armor_stand_items(
            a,
            model,
            base,
            &posed,
            world_partial,
            packed,
            skin.as_deref_mut(),
            out,
        );
        // HumanoidArmorLayer: head, chest, legs, feet. One pose map for every layer
        // (`reset` keeps its capacity).
        let mut layer_poses = Poses::default();
        for (slot, layer, worn) in [
            (5usize, "helmet", "head"),
            (4, "chestplate", "chest"),
            (3, "leggings", "legs"),
            (2, "boots", "feet"),
        ] {
            let Some(stack) = &a.equipment[slot] else {
                continue;
            };
            let stack = EquipmentRead::resident(stack);
            let Some(piece) = self.equipment_piece(&stack, worn, layer == "leggings", false) else {
                continue;
            };
            let Some(part) = assets.layers.get(&(layer, a.small)) else {
                continue;
            };
            // ArmorStandArmorModel.setupAnim runs humanoid setup independently.
            layer_poses.reset(part);
            self.setup_armor_stand_pose(a, partial, &mut layer_poses);
            for material in self.equipment_materials(
                &piece,
                &stack,
                super::equipment_piece::EquipmentPass::Draw,
            ) {
                let model = material.model.map_or(WornModel::Vanilla(part), |model| {
                    WornModel::Override(model, piece.slot)
                });
                model.submit(
                    &mut skin,
                    base,
                    &layer_poses,
                    Paint {
                        uv: material.binding.uv,
                        scroll: 0.0,
                        color: material.color,
                        light: if material.glows {
                            sunrise_client::mobs::render::FULL_BRIGHT
                        } else {
                            packed
                        },
                        overlay: material.overlay,
                    },
                    out,
                );
            }
        }
        // WingsLayer: ArmorStandRenderer adds it after the armour layers. A stand
        // never crouches or glides, so its wings rest folded.
        self.wings_geometry(
            a.equipment[4].as_ref().map(|s| &*s.item),
            a.equipment[4].as_ref(),
            None,
            a.small,
            base,
            sunrise_sim::mobs::ElytraAnimation::RESTING,
            false,
            packed,
            self.wings_overlay(a.equipment[4].as_ref()),
            out,
        );
    }
    /// `ArmorStandArmorModel.setupAnim`: humanoid preparation and posing first, then saved rotations.
    pub(in crate::game) fn setup_armor_stand_pose(
        &self,
        a: &ArmorStand,
        partial: f32,
        poses: &mut Poses,
    ) {
        use sunrise_sim::mobs::anim::{self, ArmPose};
        let state = anim::State {
            pos: 0.0,
            speed: 0.0,
            age: self.common.world.game_time as f32 + partial,
            y_rot: 0.0,
            x_rot: a.pitch,
            swing: 0.0,
            baby: a.small,
            partial,
            observer: None,
            data: None,
        };
        let mut humanoid = anim::Hum::new(ArmPose::Empty, ArmPose::Empty);
        humanoid.passenger = a.vehicle.is_some();
        anim::humanoid_with_setup(poses, &state, &humanoid, |poses| {
            if !mod_api::__private::listened(mod_api::HookId::HumanoidSetupAnim) {
                return;
            }
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let stacks = [0, 1].map(|slot| {
                a.equipment[slot]
                    .as_ref()
                    .and_then(|stack| stack.try_into_api().ok())
            });
            let view = mod_api::HumanoidRenderState {
                main_hand: stacks[0].as_ref(),
                off_hand: stacks[1].as_ref(),
                main_arm_left: false,
                is_using_item: false,
                use_hand: None,
                ticks_using_item: 0,
                use_animation: mod_api::UseAnimation::None,
                is_swinging: false,
                main_arm_pose: mod_api::ArmPose::Empty,
                off_arm_pose: mod_api::ArmPose::Empty,
            };
            self.setup_humanoid_parts(a.id, TYPE_ID, &view, partial, poses);
        });
        apply_pose(poses, &a.pose);
    }
    /// Client prediction twin of `equip_from_hand`: swaps armor slot without server stats/drops.
    pub(in crate::game) fn client_equip_from_hand(&mut self, stack: &ItemStack) -> bool {
        let Some(info) = self.items().get(&stack.item) else {
            return false;
        };
        let components = self
            .common
            .pack
            .item_models
            .components(&sunrise_sim::item_models::stack_view(stack));
        let Some(equippable) = components.get("minecraft:equippable") else {
            return false;
        };
        let actual = stack
            .data()
            .and_then(|data| data.other("minecraft:equippable"));
        let supported = actual.is_none_or(|tag| {
            matches!(tag, sunrise_core::nbt::Tag::Compound(map)
                if map.keys().all(|key| matches!(key.to_string_lossy().as_str(), "slot" | "swappable")))
                && matches!(tag.get("slot"), Some(sunrise_core::nbt::Tag::String(_)))
                && tag.get("swappable").is_none_or(|flag| {
                    sunrise_items::item_data::component_bool(flag).is_some()
                })
        });
        let slot = if supported {
            equippable.get("slot").and_then(serde_json::Value::as_str)
        } else {
            info.equip_slot.as_deref()
        };
        let (index, slot) = match slot {
            Some("head") => (0, "head"),
            Some("chest") => (1, "chest"),
            Some("legs") => (2, "legs"),
            Some("feet") => (3, "feet"),
            _ => return false,
        };
        let swappable = if !supported {
            info.swappable
        } else if let Some(actual) = actual {
            actual
                .get("swappable")
                .and_then(sunrise_items::item_data::component_bool)
                .unwrap_or(true)
        } else {
            equippable
                .get("swappable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
        };
        if !swappable {
            return false;
        }
        let current = self.common.armor[index].clone();
        if current.as_ref().is_some_and(|c| {
            (self.stations().enchantments.prevents_armor_change(c) && !self.common.creative)
                || c.same_item_same_components(stack)
        }) {
            return true;
        }
        let swap = swap_with_equipment_slot(stack, current, self.common.creative);
        self.common.armor[index] = Some(swap.worn.clone());
        self.player_equipped(slot, Some(&swap.worn));
        *self.common.held_mut() = swap.hand;
        if let Some(old) = swap.to_inventory {
            let _ = self.add_stack_to_inventory(old);
        }
        self.swing_arm();
        true
    }
    /// OptiFine CIT: the winning armour or elytra rule (`kind`) of the worn `stack`. No lookup
    /// without CIT rules.
    pub(in crate::game::client_side) fn cit_worn_rule(
        &self,
        kind: Kind,
        stack: &ItemStack,
    ) -> Option<&Rule> {
        let rules = &self.client.client_pack.item_renderer.cit.as_ref()?.rules;
        let item_max = self.items().get(&stack.item).map_or(0, |i| i.max_damage);
        sunrise_client::item_render::cit::worn_rule(rules, kind, stack, item_max)
            .and_then(|i| rules.rules.get(i))
    }
    /// `SmithingScreen`'s armour stand: `updateArmorStandPreview` puts the
    /// result on a stand without its base plate and with its arms, turned to
    /// 210 degrees, and `extractEntityInInventory` draws it at scale 25 in
    /// the box (x0, y0) to (x1, y1), moved up 1 and tilted by `ARMOR_STAND_ANGLE`.
    pub(in crate::game::client_side) fn draw_smithing_armor_stand(
        &self,
        batch: &mut sunrise_client::hud::HudBatch,
        result: Option<&ItemStack>,
        [x0, y0, x1, y1]: [f32; 4],
    ) {
        let mut stand = ArmorStand::new(0, Vec3::ZERO, 210.0);
        stand.no_base_plate = true;
        stand.show_arms = true;
        if let Some(stack) = result {
            let equip = self
                .common
                .res
                .items()
                .get(&stack.item)
                .and_then(|i| i.equip_slot.clone());
            stand.equipment[preview_slot(equip.as_deref())] = Some(stack.clone());
        }
        // LivingEntityRenderer: setupRotations (180 - body yaw), the flip and
        // the 1.501 drop, at full light.
        let base = glam::Mat4::from_rotation_y((180.0 - stand.yaw).to_radians())
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
        let mut verts = Vec::new();
        self.armor_stand_vertices(&stand, None, base, 240 | 240 << 8, None, &mut verts);
        // ARMOR_STAND_ANGLE: rotationXYZ(0.43633232, 0, PI).
        let gui =
            glam::Mat4::from_translation(glam::Vec3::new((x0 + x1) / 2.0, (y0 + y1) / 2.0, 0.0))
                * glam::Mat4::from_scale(glam::Vec3::new(25.0, 25.0, -25.0))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.0, 0.0))
                * glam::Mat4::from_rotation_x(0.436_332_32)
                * glam::Mat4::from_rotation_z(std::f32::consts::PI);
        let glints =
            super::perspective::preview_glint_quads(&verts, gui, 0..0, self.client.glint_frame);
        let quads = super::perspective::player_preview_quads(
            verts,
            gui,
            None,
            &self.client.gui.atlas.sprites,
        );
        super::perspective::emit_preview(batch, quads, glints, [x0, y0, x1, y1]);
    }
    /// `HumanoidArmorLayer` for the player: (model, texture rect, colour,
    /// overlay) of each worn piece, head to feet.
    pub(in crate::game::client_side) fn player_armor_layers(
        &self,
        hidden_layers: StockLayerMask,
    ) -> Vec<WornLayer<'_>> {
        self.stack_armor_layers(&self.common.armor, hidden_layers)
    }
    /// `PalettedTextureManager.getOrPrepare` for the trims drawn this frame
    /// (the player's armour and the armour stands'): each generated texture
    /// goes into an atlas trim slot, a free one, else the least recently
    /// drawn. Reuse the completed preparation while the requests and atlas stay the same.
    pub(in crate::game::client_side) fn prepare_trims(&mut self) {
        let assets = &self.client.armor_stand_assets.0;
        let items = self.items();
        let mut wanted: Vec<TrimTexture> = Vec::new();
        use sunrise_core::equipment_slot::EquipmentSlot::{Chest, Feet, Head, Legs};
        let layers = ["helmet", "chestplate", "leggings", "boots"];
        let stacks = self
            .common
            .armor
            .iter()
            .map(|s| s.as_ref().map(EquipmentRead::resident))
            .zip(layers.into_iter().cycle())
            .chain(self.common.mobs.iter().flat_map(|m| {
                [Head, Chest, Legs, Feet]
                    .map(|slot| finalize::equipment_slot(m, slot))
                    .into_iter()
                    .zip(layers)
            }))
            .chain(self.common.armor_stands.iter().flat_map(|a| {
                [5usize, 4, 3, 2]
                    .map(|slot| a.equipment[slot].as_ref().map(EquipmentRead::resident))
                    .into_iter()
                    .zip(layers)
            }));
        // The smithing screen's preview stand wears the result slot's item.
        let preview = self
            .common
            .station
            .as_ref()
            .filter(|s| s.kind == super::stations::Kind::Smithing)
            .and_then(|s| s.result.as_ref())
            .map(|stack| {
                let equip = items.get(&stack.item).and_then(|i| i.equip_slot.as_deref());
                let layer = match preview_slot(equip) {
                    2 => "boots",
                    3 => "leggings",
                    4 => "chestplate",
                    5 => "helmet",
                    _ => "",
                };
                (Some(EquipmentRead::resident(stack)), layer)
            });
        for (stack, layer) in stacks.chain(preview) {
            let Some(stack) = stack else {
                continue;
            };
            let Some(trim) = stack.trim() else {
                continue;
            };
            let Some(asset) =
                sunrise_sim::item_sync::equippable(stack.base(), items).and_then(|e| e.asset_id)
            else {
                continue;
            };
            let kind = if layer == "leggings" {
                "humanoid_leggings"
            } else {
                "humanoid"
            };
            if let Some(t) = assets.trim_texture(&asset, kind, trim)
                && !wanted.contains(&t)
            {
                wanted.push(t);
            }
        }
        self.client.trim_slots.prepare(
            wanted,
            &assets.root,
            &mut self.client.gui.atlas,
            &mut self.client.map_patches,
        );
    }
    /// `HumanoidArmorLayer` for worn stacks (head to feet), trims included.
    pub(in crate::game::client_side) fn stack_armor_layers(
        &self,
        armor: &[Option<ItemStack>; 4],
        hidden_layers: StockLayerMask,
    ) -> Vec<WornLayer<'_>> {
        self.armor_layers(
            armor
                .each_ref()
                .map(|s| s.as_ref().map(EquipmentRead::resident)),
            hidden_layers,
        )
    }
    /// `HumanoidArmorLayer` for another player's model: the armour it wears
    /// (`extra.equipment`, head to feet).
    pub(in crate::game::client_side) fn worn_armor_layers(
        &self,
        m: &Mob,
        hidden_layers: StockLayerMask,
    ) -> Vec<WornLayer<'_>> {
        use sunrise_core::equipment_slot::EquipmentSlot::{Chest, Feet, Head, Legs};
        self.armor_layers(
            [Head, Chest, Legs, Feet].map(|slot| finalize::equipment_slot(m, slot)),
            hidden_layers,
        )
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt2 as _;

#[cfg(test)]
pub(in crate::game) mod setup_anim_tests {
    use super::*;
    use crate::game::server_side::armor_stands::setup_anim_tests::HEARD;
    use crate::game::server_side::armor_stands::setup_anim_tests::Listener;
    use std::sync::atomic::Ordering;

    /// The real stand adapter exposes humanoid riding/head preparation, then
    /// preserves position edits while saved rotations replace listener rotations.
    #[test]
    #[ignore = "daily"]
    fn armor_stand_prepares_humanoid_then_overrides_only_rotations() {
        let mods = vec![mod_api::ModEntry::new("pose", Box::new(Listener))].into();
        mod_platform::testing::with_mods(mods, || {
            assert!(mod_api::__private::listened(
                mod_api::HookId::HumanoidSetupAnim
            ));
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let mut stand = ArmorStand::new(73, Vec3::ZERO, 0.0);
                stand.show_arms = true;
                stand.pitch = 30.0;
                stand.vehicle = Some(9);
                stand.pose = [[10.0, 20.0, 30.0]; 6];
                let model = game.client.armor_stand_assets.0.main.as_ref().unwrap();
                let mut poses = Poses::new(model);
                game.setup_armor_stand_pose(&stand, 0.5, &mut poses);
                for name in [
                    "head",
                    "body",
                    "left_arm",
                    "right_arm",
                    "left_leg",
                    "right_leg",
                ] {
                    let pose = poses.pose_of(name).unwrap();
                    assert_eq!(&pose[..3], &[7.0, 8.0, 9.0]);
                    assert_eq!(
                        &pose[3..6],
                        &[
                            10.0_f32.to_radians(),
                            20.0_f32.to_radians(),
                            30.0_f32.to_radians()
                        ]
                    );
                }
                let mut verts = Vec::new();
                game.armor_stand_vertices(
                    &stand,
                    Some(0.5),
                    glam::Mat4::IDENTITY,
                    0xf0f0,
                    None,
                    &mut verts,
                );
                assert!(!verts.is_empty());
            });
        });
        assert_eq!(HEARD.load(Ordering::Relaxed), 2);
    }
}
