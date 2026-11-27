//! Shared eligibility and material resolution for worn humanoid equipment.
use super::stock_layer_mask::StockLayerMask;
use super::*;
use mod_api::{ArmorLayerPass, ArmorLayerSlot, StockLayer};
use sunrise_client::mobs::render::EquipmentModel;
use sunrise_sim::mobs::finalize::EquipmentRead;

pub(in crate::game) use sunrise_client::mobs::render::ArmorSlot;

pub(in crate::game) fn stock_armor_layer(slot: ArmorSlot, pass: ArmorLayerPass) -> StockLayer {
    let slot = match slot {
        ArmorSlot::Head => ArmorLayerSlot::Head,
        ArmorSlot::Chest => ArmorLayerSlot::Chest,
        ArmorSlot::Legs => ArmorLayerSlot::Legs,
        ArmorSlot::Feet => ArmorLayerSlot::Feet,
    };
    StockLayer::Armor(slot, pass)
}

/// The model layers the mob's humanoid equipment renderer submits, in submission order.
pub(in crate::game) const MOB_ARMOR: [(
    sunrise_core::equipment_slot::EquipmentSlot,
    &str,
    &str,
    ArmorSlot,
); 4] = [
    (
        sunrise_core::equipment_slot::EquipmentSlot::Chest,
        "chestplate",
        "chest",
        ArmorSlot::Chest,
    ),
    (
        sunrise_core::equipment_slot::EquipmentSlot::Legs,
        "leggings",
        "legs",
        ArmorSlot::Legs,
    ),
    (
        sunrise_core::equipment_slot::EquipmentSlot::Feet,
        "boots",
        "feet",
        ArmorSlot::Feet,
    ),
    (
        sunrise_core::equipment_slot::EquipmentSlot::Head,
        "helmet",
        "head",
        ArmorSlot::Head,
    ),
];

pub(in crate::game) fn has_mob_armor_layer(mob: &Mob) -> bool {
    !sunrise_client::mobs::render::avatar(mob)
        && !sunrise_sim::mobs::equine::is_equine(mob.species())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum EquipmentPass {
    Draw,
    TextureInventory,
}

pub(in crate::game) struct EquipmentPiece<'a> {
    pub asset: String,
    pub slot: ArmorSlot,
    pub kind: &'static str,
    pub layers: &'a [EquipmentAssetLayer],
    pub overlay: u32,
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn equipment_piece<'a>(
        &'a self,
        stack: &EquipmentRead<'_>,
        slot: &str,
        leggings: bool,
        baby: bool,
    ) -> Option<EquipmentPiece<'a>> {
        let base = stack.base();
        let equippable = sunrise_sim::item_sync::equippable(base, self.common.res.items())?;
        let glint = self
            .common
            .res
            .items()
            .enchantments
            .props
            .get(&base.item)
            .and_then(|p| p.glint);
        let overlay = if stack.has_foil(glint) {
            sunrise_client::mobs::render::ARMOR_GLINT
        } else {
            0
        };
        resolve_piece(
            equippable,
            slot,
            leggings,
            baby,
            &self.client.armor_stand_assets.0.equipment,
            overlay,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) struct EquipmentAssetLayer {
    pub texture: String,
    pub dyeable: bool,
    pub use_player_texture: bool,
    pub color_when_undyed: Option<u32>,
}
pub(in crate::game) type EquipmentAssets =
    HashMap<String, HashMap<String, Vec<EquipmentAssetLayer>>>;

fn resolve_piece<'a>(
    equippable: sunrise_sim::item_sync::Equippable,
    slot: &str,
    leggings: bool,
    baby: bool,
    equipment: &'a EquipmentAssets,
    overlay: u32,
) -> Option<EquipmentPiece<'a>> {
    if equippable.slot != slot {
        return None;
    }
    let asset = equippable.asset_id?;
    let kind = if baby {
        "humanoid_baby"
    } else if leggings {
        "humanoid_leggings"
    } else {
        "humanoid"
    };
    let layers = equipment.get(&asset)?.get(kind)?;
    if layers.is_empty() {
        return None;
    }
    let slot = match slot {
        "head" => ArmorSlot::Head,
        "chest" => ArmorSlot::Chest,
        "legs" => ArmorSlot::Legs,
        "feet" => ArmorSlot::Feet,
        _ => return None,
    };
    Some(EquipmentPiece {
        asset,
        slot,
        kind,
        layers,
        overlay,
    })
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn mob_equipment_layers(
        &self,
        mob: &Mob,
        pass: EquipmentPass,
        hidden_layers: StockLayerMask,
    ) -> Vec<sunrise_client::mobs::render::Layer> {
        use sunrise_client::mobs::render;
        use sunrise_sim::mobs::finalize;
        if render::avatar(mob) {
            return Vec::new();
        }
        let mut out = Vec::new();
        if has_mob_armor_layer(mob)
            && sunrise_items::equipment::ARMOR
                .iter()
                .any(|&slot| mob.extra.slot_stack(slot).is_some())
        {
            for (slot, layer, worn, _) in MOB_ARMOR {
                let Some(read) = finalize::equipment_slot(mob, slot) else {
                    continue;
                };
                let Some(piece) =
                    self.equipment_piece(&read, worn, layer == "leggings", mob.is_baby())
                else {
                    continue;
                };
                let Some(key) =
                    render::equipment_model_key(mob, layer, self.client_res().client_mob_models())
                else {
                    continue;
                };
                // The mob renderer resolves CEM textures before applying their emissive overlay.
                // Do not submit the default texture's glow a second time here.
                for material in self
                    .equipment_materials(&piece, &read, pass)
                    .into_iter()
                    .filter(|m| !m.glows)
                {
                    let overlay = material.visible_overlay(piece.slot, hidden_layers);
                    let mut layer = render::Layer::equipment(
                        &key,
                        material.binding.name,
                        material.color,
                        overlay.unwrap_or_default(),
                    );
                    // Keep the material's place for CEM's first-layer texture selection, even
                    // when the material will not be submitted.
                    layer.equipment_hidden = overlay.is_none();
                    layer.equipment_model = material.model.map(|model| (model.clone(), piece.slot));
                    layer.equipment_uv = material.final_texture.then_some(material.binding.uv);
                    out.push(layer);
                }
            }
        }
        out.extend(self.body_equipment_layers(mob));
        out
    }

    /// `SimpleEquipmentLayer` for the body armour of a horse, an undead horse or a nautilus: the
    /// `horse_body` or `nautilus_body` layers of the equipment asset the stack's `equippable`
    /// names (`EquipmentLayerRenderer`), in any namespace.
    fn body_equipment_layers(&self, mob: &Mob) -> Vec<sunrise_client::mobs::render::Layer> {
        use sunrise_client::mobs::render;
        use sunrise_sim::mobs::Species as S;
        let (model, kind) = match mob.species() {
            S::Horse => ("horse_armor#main", "horse_body"),
            S::ZombieHorse | S::SkeletonHorse => ("undead_horse_armor#main", "horse_body"),
            S::Nautilus | S::ZombieNautilus if !mob.is_baby() => {
                ("nautilus_armor#main", "nautilus_body")
            }
            _ => return Vec::new(),
        };
        let Some(stack) = mob.extra.body_stack() else {
            return Vec::new();
        };
        let Some(equippable) = sunrise_sim::item_sync::equippable(stack, self.common.res.items())
        else {
            return Vec::new();
        };
        let Some(layers) = equippable
            .asset_id
            .as_ref()
            .filter(|_| equippable.slot == "body")
            .and_then(|asset| self.client.armor_stand_assets.0.equipment.get(asset))
            .and_then(|kinds| kinds.get(kind))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for layer in layers {
            let color = crate::game::client_side::armor_stands_client::layer_color(
                layer.dyeable,
                layer.color_when_undyed,
                Some(stack),
            );
            if color == 0 {
                continue;
            }
            let Some((name, _)) = self
                .client
                .gui
                .atlas
                .sprites
                .get_key_value(layer.texture.as_str())
            else {
                continue;
            };
            out.push(render::Layer::equipment(model, name, color, 0));
        }
        out
    }
}

#[derive(Clone, Copy)]
pub(in crate::game) struct MaterialBinding<'a> {
    pub name: &'a str,
    pub uv: [f32; 4],
}
pub(in crate::game) struct EquipmentMaterial<'a> {
    pub binding: MaterialBinding<'a>,
    pub color: u32,
    pub overlay: u32,
    pub glows: bool,
    is_trim: bool,
    pub model: Option<&'a Arc<EquipmentModel>>,
    pub final_texture: bool,
}

impl EquipmentMaterial<'_> {
    /// The visibility edit is applied after material and foil eligibility are resolved, so
    /// hiding one pass cannot move the foil or a texture override to another material.
    pub(in crate::game) fn visible_overlay(
        &self,
        slot: ArmorSlot,
        hidden: StockLayerMask,
    ) -> Option<u32> {
        let pass = if self.is_trim {
            ArmorLayerPass::Trim
        } else {
            ArmorLayerPass::Base
        };
        let foil =
            self.overlay != 0 && !hidden.contains(stock_armor_layer(slot, ArmorLayerPass::Glint));
        match (hidden.contains(stock_armor_layer(slot, pass)), foil) {
            (false, true) => Some(self.overlay),
            (false, false) => Some(0),
            (true, true) => Some(sunrise_client_render::gfx::ARMOR_GLINT_ONLY_OVERLAY),
            (true, false) => None,
        }
    }
}

pub(in crate::game) struct ResolvedEquipmentLayer<'a> {
    pub binding: MaterialBinding<'a>,
    pub color: u32,
    pub model: Option<&'a Arc<EquipmentModel>>,
    pub final_texture: bool,
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn equipment_materials<'a>(
        &'a self,
        piece: &EquipmentPiece<'_>,
        read: &EquipmentRead<'_>,
        pass: EquipmentPass,
    ) -> Vec<EquipmentMaterial<'a>> {
        let stack = read.base();
        let sprites = &self.client.gui.atlas.sprites;
        let bind = |name: &str| {
            sprites
                .get_key_value(name)
                .map(|(name, r)| MaterialBinding {
                    name,
                    uv: [r.u0, r.v0, r.u1, r.v1],
                })
        };
        let cit = self.cit_worn_rule(sunrise_optifine::cit::Kind::Armor, stack);
        let trim = read
            .trim()
            .and_then(|trim| {
                self.client
                    .armor_stand_assets
                    .0
                    .trim_texture(&piece.asset, piece.kind, trim)
            })
            .and_then(|t| bind(&t.sprite()));
        let emissive = self
            .client_res()
            .client_mob_data()
            .emissive
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let texture = |name: &str| {
            sunrise_client::cit_armor::worn_layer(cit, &piece.asset, piece.kind, name, bind)
        };
        let glow = |name: &str| emissive.overlay(name).and_then(bind);
        if pass == EquipmentPass::Draw
            && let Some(materials) =
                self.extended_equipment_materials(piece, read, trim, texture, glow)
        {
            return materials;
        }
        piece.materials(Some(stack), trim, texture, glow)
    }
}

impl EquipmentPiece<'_> {
    // Only resolved atlas bindings reach submissions. Missing textures do not consume foil.
    #[cfg(feature = "client")]
    fn materials<'a>(
        &self,
        stack: Option<&ItemStack>,
        trim: Option<MaterialBinding<'a>>,
        mut texture: impl FnMut(&str) -> Option<MaterialBinding<'a>>,
        emissive: impl FnMut(&str) -> Option<MaterialBinding<'a>>,
    ) -> Vec<EquipmentMaterial<'a>> {
        self.resolved_materials(
            trim,
            |_, layer| {
                let color = crate::game::client_side::armor_stands_client::layer_color(
                    layer.dyeable,
                    layer.color_when_undyed,
                    stack,
                );
                if color == 0 {
                    return None;
                }
                Some(ResolvedEquipmentLayer {
                    binding: texture(&layer.texture)?,
                    color,
                    model: None,
                    final_texture: false,
                })
            },
            emissive,
        )
    }

    pub(in crate::game::client_side) fn resolved_materials<'a>(
        &self,
        trim: Option<MaterialBinding<'a>>,
        mut resolve: impl FnMut(usize, &EquipmentAssetLayer) -> Option<ResolvedEquipmentLayer<'a>>,
        mut emissive: impl FnMut(&str) -> Option<MaterialBinding<'a>>,
    ) -> Vec<EquipmentMaterial<'a>> {
        let trim = trim.filter(|_| self.kind != "humanoid_baby");
        let mut foil = if trim.is_some() { 0 } else { self.overlay };
        let mut last_model = None;
        let mut out = Vec::new();
        for (index, layer) in self.layers.iter().enumerate() {
            let Some(ResolvedEquipmentLayer {
                binding,
                color,
                model,
                final_texture,
            }) = resolve(index, layer)
            else {
                continue;
            };
            last_model = model;
            let name = if final_texture {
                binding.name
            } else {
                &layer.texture
            };
            out.push(EquipmentMaterial {
                binding,
                color,
                overlay: foil,
                glows: false,
                is_trim: false,
                model,
                final_texture,
            });
            foil = 0;
            if let Some(binding) = emissive(name) {
                out.push(EquipmentMaterial {
                    binding,
                    color,
                    overlay: 0,
                    glows: true,
                    is_trim: false,
                    model,
                    final_texture,
                });
            }
        }
        if let Some(binding) = trim {
            out.push(EquipmentMaterial {
                binding,
                color: 0xFFFF_FFFF,
                overlay: self.overlay,
                glows: false,
                is_trim: true,
                model: last_model,
                final_texture: false,
            });
        }
        out
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client::mobs::{Part, render};
    use sunrise_sim::{item_sync::Equippable, mobs::anim::Poses};

    pub(super) fn asset_layer(
        texture: &str,
        dyeable: bool,
        color_when_undyed: Option<u32>,
    ) -> EquipmentAssetLayer {
        EquipmentAssetLayer {
            texture: texture.into(),
            dyeable,
            use_player_texture: false,
            color_when_undyed,
        }
    }

    fn equippable(slot: &str, asset: Option<&str>) -> Equippable {
        Equippable {
            slot: slot.into(),
            asset_id: asset.map(str::to_owned),
            camera_overlay: None,
        }
    }

    #[test]
    fn asset_slot_and_baby_layer_gate() {
        let layers = vec![
            asset_layer("test:outer", false, None),
            asset_layer("test:inner", true, Some(0xA06540)),
        ];
        let equipment = HashMap::from([(
            "test:chest".into(),
            HashMap::from([
                ("humanoid".into(), layers.clone()),
                ("humanoid_leggings".into(), vec![]),
                (
                    "humanoid_baby".into(),
                    vec![asset_layer("test:baby", false, None)],
                ),
            ]),
        )]);
        let resolve = |slot, leggings, baby| {
            resolve_piece(
                equippable("chest", Some("test:chest")),
                slot,
                leggings,
                baby,
                &equipment,
                36,
            )
        };
        assert!(resolve("head", false, false).is_none());
        assert!(resolve("chest", true, false).is_none());
        assert!(
            resolve_piece(
                equippable("chest", None),
                "chest",
                false,
                false,
                &equipment,
                0
            )
            .is_none()
        );
        assert!(
            resolve_piece(
                equippable("chest", Some("test:unknown")),
                "chest",
                false,
                false,
                &equipment,
                0
            )
            .is_none()
        );
        assert_eq!(resolve("chest", false, false).unwrap().layers, layers);
        assert_eq!(
            resolve("chest", true, true).unwrap().layers[0].texture,
            "test:baby"
        );
    }

    // Pin the actual vertex frame, not only material names: the player and stand
    // still submit the same model, UV, colour and foil after the shared gate.
    #[test]
    fn player_and_stand_material_frames_are_identical() {
        let model = Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![sunrise_client::mobs::Polygon {
                normal: [0.0, 0.0, 1.0],
                vertices: [
                    [0.0, 0.0, 0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0, 1.0, 1.0],
                    [0.0, 1.0, 0.0, 0.0, 1.0],
                ],
                cull_back: false,
            }],
            children: Vec::new(),
            layer: None,
        };
        let poses = Poses::new(&model);
        let equipment = HashMap::from([(
            "test:leather".into(),
            HashMap::from([(
                "humanoid".into(),
                vec![
                    asset_layer("test:base", true, Some(0xA06540)),
                    asset_layer("test:overlay", false, None),
                ],
            )]),
        )]);
        let piece = resolve_piece(
            equippable("chest", Some("test:leather")),
            "chest",
            false,
            false,
            &equipment,
            0,
        )
        .unwrap();
        for base in [
            glam::Mat4::IDENTITY,
            glam::Mat4::from_scale(glam::Vec3::splat(0.5)),
        ] {
            let mut expected = Vec::new();
            let mut actual = Vec::new();
            for (uv, tint, foil) in [
                ([0.0, 0.0, 0.5, 0.5], 0xFF4065A0, 0),
                ([0.5, 0.5, 1.0, 1.0], 0xFFFFFFFF, 0),
            ] {
                render::draw(&model, base, &poses, uv, tint, 0xF0F0, foil, &mut expected);
            }
            for material in piece.materials(
                None,
                None,
                |name| match name {
                    "test:base" => Some(MaterialBinding {
                        name: "test:base",
                        uv: [0.0, 0.0, 0.5, 0.5],
                    }),
                    "test:overlay" => Some(MaterialBinding {
                        name: "test:overlay",
                        uv: [0.5, 0.5, 1.0, 1.0],
                    }),
                    _ => None,
                },
                |_| None,
            ) {
                render::draw(
                    &model,
                    base,
                    &poses,
                    material.binding.uv,
                    material.color,
                    0xF0F0,
                    material.overlay,
                    &mut actual,
                );
            }
            assert!(!actual.is_empty());
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&actual),
                bytemuck::cast_slice::<_, u8>(&expected)
            );
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
mod material_tests {
    use super::*;
    use crate::game::client_side::equipment_piece::tests::asset_layer;

    /// `EquipmentLayerRenderer` for a horse: a mod's body armour draws the `horse_body` layer of
    /// the equipment asset its `equippable` names, in the asset's own namespace.
    #[test]
    #[ignore = "daily"]
    fn a_horse_in_mod_body_armour_draws_its_namespaced_horse_body_layer() {
        use sunrise_core::nbt::Tag;
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let texture = "example:entity/equipment/horse_body/obsidian";
            game.client.armor_stand_assets.0.equipment.insert(
                "example:obsidian".into(),
                HashMap::from([("horse_body".into(), vec![asset_layer(texture, false, None)])]),
            );
            game.client.gui.atlas.sprites.insert(
                texture.into(),
                sunrise_client::hud::Rect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    w: 64,
                    h: 64,
                },
            );
            let kind = game
                .common
                .res
                .mob_types()
                .get("minecraft:horse")
                .unwrap()
                .clone();
            let mut horse = Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
            let mut armor = ItemStack::new("example:obsidian_horse_armor", 1);
            armor.data_mut().set_other(
                "minecraft:equippable",
                Some(Tag::compound([
                    ("slot", Tag::String("body".into())),
                    ("asset_id", Tag::String("example:obsidian".into())),
                ])),
            );
            horse.extra.set_body_stack(Some(armor));
            let layers =
                game.mob_equipment_layers(&horse, EquipmentPass::Draw, StockLayerMask::EMPTY);
            assert_eq!(layers.len(), 1);
            assert_eq!(&*layers[0].model, "horse_armor#main");
            assert_eq!(&*layers[0].texture, texture);
        });
    }

    #[test]
    fn armor_visibility_removes_only_the_chosen_pass_in_submission_order() {
        use sunrise_client::mobs::render::ARMOR_GLINT;
        use sunrise_client_render::gfx::ARMOR_GLINT_ONLY_OVERLAY;

        let layers = [
            asset_layer("base", false, None),
            asset_layer("overlay", false, None),
        ];
        let piece = EquipmentPiece {
            asset: "test:armor".into(),
            slot: ArmorSlot::Chest,
            kind: "humanoid",
            layers: &layers,
            overlay: ARMOR_GLINT,
        };
        let bind = |name| MaterialBinding {
            name,
            uv: [0.0, 0.0, 1.0, 1.0],
        };
        let materials = piece.materials(
            None,
            Some(bind("trim")),
            |name| match name {
                "base" => Some(bind("base")),
                "overlay" => Some(bind("overlay")),
                _ => None,
            },
            |_| None,
        );
        for (hide, expected) in [
            (
                None,
                &[("base", 0), ("overlay", 0), ("trim", ARMOR_GLINT)][..],
            ),
            (Some(ArmorLayerPass::Base), &[("trim", ARMOR_GLINT)]),
            (
                Some(ArmorLayerPass::Glint),
                &[("base", 0), ("overlay", 0), ("trim", 0)],
            ),
            (
                Some(ArmorLayerPass::Trim),
                &[
                    ("base", 0),
                    ("overlay", 0),
                    ("trim", ARMOR_GLINT_ONLY_OVERLAY),
                ],
            ),
        ] {
            let mut hidden = StockLayerMask::EMPTY;
            if let Some(pass) = hide {
                hidden.insert(StockLayer::Armor(ArmorLayerSlot::Chest, pass));
            }
            let actual = materials
                .iter()
                .filter_map(|m| Some((m.binding.name, m.visible_overlay(piece.slot, hidden)?)))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{hide:?}");
        }
    }

    #[test]
    fn dyeable_without_default_uses_dye_or_skips_before_foil() {
        let layers = vec![
            asset_layer("dyed_only", true, None),
            asset_layer("outer", false, None),
        ];
        let piece = EquipmentPiece {
            asset: "test:dyed".into(),
            slot: ArmorSlot::Chest,
            kind: "humanoid",
            layers: &layers,
            overlay: 36,
        };
        let mut stack = ItemStack::new("test:equipment", 1);
        let draw = |stack: &ItemStack| {
            piece
                .materials(
                    Some(stack),
                    None,
                    |name| {
                        let name = match name {
                            "dyed_only" => "dyed_only",
                            "outer" => "outer",
                            _ => return None,
                        };
                        Some(MaterialBinding {
                            name,
                            uv: [0.0, 0.0, 1.0, 1.0],
                        })
                    },
                    |_| None,
                )
                .into_iter()
                .map(|m| (m.binding.name, m.color, m.overlay))
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(&stack), [("outer", 0xFFFFFFFF, 36)]);
        stack.data_mut().dyed_color = Some(0x123456);
        assert_eq!(
            draw(&stack),
            [("dyed_only", 0xFF563412, 36), ("outer", 0xFFFFFFFF, 0)]
        );
    }

    #[test]
    fn ordered_materials_first_drawn_foil_trim_and_babies() {
        let layers = vec![
            asset_layer("missing", false, None),
            asset_layer("inner", false, None),
            asset_layer("outer", false, None),
        ];
        let mut piece = EquipmentPiece {
            asset: "test:custom".into(),
            slot: ArmorSlot::Chest,
            kind: "humanoid",
            layers: &layers,
            overlay: 36,
        };
        let stack = ItemStack::new("test:not_an_armor_name", 1);
        let draw = |piece: &EquipmentPiece<'_>, trim: Option<&'static str>| {
            let bind = |name| MaterialBinding {
                name,
                uv: [0.0, 0.0, 1.0, 1.0],
            };
            piece
                .materials(
                    Some(&stack),
                    trim.map(bind),
                    |name| match name {
                        "inner" => Some(bind("inner")),
                        "outer" => Some(bind("outer")),
                        _ => None,
                    },
                    |_| None,
                )
                .into_iter()
                .map(|l| (l.binding.name.to_owned(), l.overlay))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            draw(&piece, None),
            [("inner".into(), 36), ("outer".into(), 0)]
        );
        assert_eq!(
            draw(&piece, Some("trim")),
            [
                ("inner".into(), 0),
                ("outer".into(), 0),
                ("trim".into(), 36)
            ]
        );
        piece.kind = "humanoid_baby";
        assert_eq!(
            draw(&piece, Some("trim")),
            [("inner".into(), 36), ("outer".into(), 0)]
        );
    }
}
