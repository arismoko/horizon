//! Mob rendering (`LivingEntityRenderer.submit` and the render layers):
//! variant/baby model and texture selection, setupRotations, per-species
//! scale, and extra layers (sheep wool, spider eyes, drowned/stray outer
//! layers, villager overlays, collars...).
use super::{
    Mob, ModelNormals, Part, Species,
    anim::{self, Poses},
    living::wrap,
    skin::Paint,
};
use crate::mesher::Vertex;
use crate::mobs_render::{ClientMobData, ClientMobModels, ClientMobRender, ClientMobVisuals};
use std::borrow::Cow;
use std::f32::consts::PI;
use sunrise_core::geometry::trig::Mth;

mod frame_model;
use frame_model::FrameBody;
pub use frame_model::{BakedFrameModel, BakedFrameModels, BakedFrameNode, FrameModelPose};

mod equipment;
pub use equipment::{ArmorSlot, EquipmentModel, WornLayer, WornModel};

// The invisible body a viewer still sees (`Entity.isInvisibleTo`), beside this file.
#[path = "invisible_body.rs"]
mod invisible_body;
pub use invisible_body::render_seen_invisible;

/// Whether a mixin build installed a patched copy of the free function `$function` of this
/// module. The prebuilt game's staging pass answers it from the function's replacement slot; a
/// build without replacement slots has no patched functions.
macro_rules! replaced {
    ($function:ident) => {
        false
    };
}

/// Immutable resources and the optional frame-local gameplay animation observer.
/// Ordinary tools and probes use `ClientMobRender` directly, with no callbacks.
pub trait RenderDataSource {
    fn mob_tables(&self) -> ClientMobRender<'_>;
    fn mob_data(&self) -> &ClientMobData {
        self.mob_tables().data
    }
    /// Equipment asset layers, resolved by the resource owner after the slot gate.
    fn equipment_layers(&self, _mob: &Mob) -> Vec<Layer> {
        Vec::new()
    }
    /// Default material inventory for random-texture selection, without frame callbacks.
    fn equipment_inventory(&self, mob: &Mob) -> Vec<Layer> {
        self.equipment_layers(mob)
    }
    fn humanoid_setup(&self) -> Option<&dyn anim::HumanoidSetup> {
        None
    }
    /// The synced or world variant tables for registry-backed entity textures.
    fn variants(&self) -> Option<&super::variants::VariantTables> {
        None
    }
    /// The observer of `living_model_posed`, present only while a mod listens, so a draw with
    /// none builds nothing and asks nothing.
    fn model_posed(&self) -> Option<&dyn ModelPosed> {
        None
    }
    /// `PiglinAi.isLovedItem`: whether `item` is in `#minecraft:piglin_loved` of the tags the
    /// game plays with (on a client, the server's). False for a source without item tags.
    fn piglin_loves(&self, _item: &str) -> bool {
        false
    }
    /// `ItemStack.has(DataComponents.TOOL)` by the game's item data
    /// (`AbstractPiglin.isHoldingMeleeWeapon`). False for a source without item data.
    fn has_tool(&self, _stack: &crate::items::ItemStack) -> bool {
        false
    }
}

impl anim::AnimDataSource for &dyn RenderDataSource {
    fn humanoid_setup(&self) -> Option<&dyn anim::HumanoidSetup> {
        RenderDataSource::humanoid_setup(*self)
    }

    fn piglin_loves(&self, item: &str) -> bool {
        RenderDataSource::piglin_loves(*self, item)
    }

    fn has_tool(&self, stack: &crate::items::ItemStack) -> bool {
        RenderDataSource::has_tool(*self, stack)
    }
}

impl std::fmt::Debug for dyn RenderDataSource + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RenderDataSource")
    }
}

/// What a draw tells the game at the RETURN of `EntityModel.setupAnim` for each model it poses.
pub trait ModelPosed {
    /// `layer` is the model's `ModelLayerLocation` (`minecraft:zombie#chestplate`); `camera` is
    /// the draw's camera position (`None` for a draw placed without one, such as a screen's);
    /// `model` lends the posed hierarchy.
    fn posed(
        &self,
        mob: &Mob,
        partial: f32,
        camera: Option<glam::DVec3>,
        layer: &str,
        model: &mut dyn mod_api::FrameModelPoseEdit,
    );
}

/// The `living_model_posed` hook, for the entity `entity` drawn with the model `layer` (its
/// `ModelLayerLocation`, `namespace:path#layer`). It runs inside the draw, so it takes the
/// loop context of the client and no `Host`.
#[inline(never)]
pub fn native_living_model_posed(
    context: &sunrise_world::native_load_context::LoopContext,
    entity: &mod_api::EntityRenderState,
    layer: &str,
    model: &mut dyn mod_api::FrameModelPoseEdit,
) {
    if !mod_api::__private::listened(mod_api::HookId::LivingModelPosed) {
        return;
    }
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_living_model_posed(&reader.context(), entity, layer, model);
}

/// The RETURN of `EntityModel.setupAnim` for the model `key` (a body, or an armour or equipment
/// model that runs its own setup): the `living_model_posed` listeners edit `poses`, which the
/// draw then uses. The model's `ModelLayerLocation` is its key under `minecraft:` unless a mod
/// registered it under its own namespace; the default body is `<entity>#main`.
fn model_posed(
    m: &Mob,
    partial: f32,
    camera: Option<glam::DVec3>,
    data: &dyn RenderDataSource,
    key: &str,
    layout: Option<&BakedFrameModel>,
    poses: &mut Poses,
) {
    let (Some(observer), Some(layout)) = (data.model_posed(), layout) else {
        return;
    };
    // Interned: a listener is asked for every posed model of every draw.
    let location: Cow<'_, str> = if key == "main" {
        interned(format_args!("{}#main", m.kind.id))
    } else if !key.contains('#') {
        interned(format_args!("minecraft:{key}#main"))
    } else if key
        .split_once('#')
        .is_some_and(|(model, _)| model.contains(':'))
    {
        Cow::Borrowed(key)
    } else {
        interned(format_args!("minecraft:{key}"))
    };
    observer.posed(
        m,
        partial,
        camera,
        &location,
        &mut frame_model::PosedEdit::new(layout, poses),
    );
}

/// A texture lookup into the shared atlas (name without namespace -> uv rect).
pub type Atlas<'a> = &'a dyn Fn(&str) -> Option<[f32; 4]>;

/// The overlay code of an enchanted armour layer's glint, past the ones
/// `gfx/entity_batch.rs` gives materials (13 is the boat water mask, which
/// draws depth only: armour with 13 vanished; 35 is text pulled toward the
/// camera). It marks the `ARMOR_ENTITY_GLINT_TEXTURING` sample: entity.wgsl
/// transforms the model UV and reads `enchanted_glint_armor`.
pub const ARMOR_GLINT: u32 = 36;

/// `RenderTypes.breezeWind`: the texture of the breeze's wind layer scrolls
/// by 0.02 of the texture per tick.
pub const BREEZE_WIND_SCROLL: f32 = 0.02;
/// `WindChargeRenderer.xOffset`: the wind charge's texture scrolls by 0.03 of
/// the texture per tick.
pub const WIND_CHARGE_SCROLL: f32 = 0.03;
/// The offset of a scrolling texture: `RenderTypes.breezeWind` takes the
/// texture's age times the speed, modulo one.
pub fn scroll_offset(age: f32, speed: f32) -> f32 {
    (age * speed) % 1.0
}
/// The vertex's U with the scroll: the texture matrix of
/// `RenderTypes.breezeWind` moves the texture under the model, so the U
/// moves by the offset and wraps around the texture.
fn scroll_u(u: f32, scroll: f32) -> f32 {
    if scroll == 0.0 { u } else { (u + scroll) % 1.0 }
}
/// Full sky and block light in a vertex's `light`.
pub const FULL_BRIGHT: u32 = 240 | 240 << 8;
/// How brightly the eyes render type's layers (spiders', endermen's and
/// phantoms' eyes, the breeze's, the creaking's, the copper golem's) shine in
/// HDR output, as `mesher::hdr_tier`.
const EYES_TIER: u8 = crate::mesher::BRIGHT;

/// Tags quads as drawn by a render type vanilla gives an `OitPipelineSet`
/// (`gfx::OIT_ENTITY`): improved transparency draws them in its stages.
fn mark_oit(quads: &mut [Vertex]) {
    for vertex in quads {
        vertex.layer |= crate::gfx::OIT_ENTITY;
    }
}
/// Whether the species' model is built with `RenderTypes::entityTranslucent`
/// (`Model.renderType`, which `LivingEntityRenderer.getRenderType` uses for the
/// body): the allay, the piglins, the breeze, the vex, the player model and the
/// sulfur cube.
fn translucent_body(m: &Mob) -> bool {
    matches!(
        m.species(),
        Species::Allay
            | Species::Piglin
            | Species::PiglinBrute
            | Species::ZombifiedPiglin
            | Species::Breeze
            | Species::Vex
            | Species::SulfurCube
    ) || avatar(m)
}
/// Whether a layer's render type has an `OitPipelineSet`: `RenderTypes.eyes` and
/// `entityTranslucentEmissive` (the full bright layers), `breezeWind`, and
/// `entityTranslucent` (the slime's outer cube, the sulfur cube's inner cube
/// and the horse's markings).
fn translucent_layer(m: &Mob, layer: &Layer) -> bool {
    layer.full_bright
        || layer.model == "breeze#wind"
        || layer.model == "slime#outer"
        || matches!(m.species(), Species::SulfurCube)
        || layer.texture.starts_with("entity/horse/horse_markings")
}

/// Whether a layer is the eyes render type's (`RenderTypes.eyes`: full bright, no
/// normal, at `EYES_TIER`) on a mob whose other draws are plain: spiders',
/// endermen's, phantoms', the creaking's and the copper golem's. The GPU path
/// poses them as [`super::skin::Skin`] `FLAT` draws when improved transparency
/// is off ([`super::skin::SkinModels::tagged`]).
fn eyes_layer(m: &Mob, layer: &Layer) -> bool {
    layer.full_bright
        && layer.scroll == 0.0
        && matches!(
            m.species(),
            Species::Spider
                | Species::CaveSpider
                | Species::Enderman
                | Species::Phantom
                | Species::Creaking
                | Species::CopperGolem
        )
}

/// One draw of a model with a texture.
pub struct Layer {
    /// Final sprite UVs bound at reload; the texture name still identifies its emissive sprite.
    pub equipment_uv: Option<[f32; 4]>,
    /// Equipment owns its overlay; other layers inherit the body overlay.
    pub equipment_overlay: Option<u32>,
    /// A suppressed submission still participates in first-material CEM texture selection.
    pub equipment_hidden: bool,
    /// Reload-baked, slot-clipped replacement; never geometry supplied by a frame.
    pub equipment_model: Option<(std::sync::Arc<EquipmentModel>, ArmorSlot)>,
    pub model: Cow<'static, str>,
    pub texture: Cow<'static, str>,
    pub color: u32,
    pub full_bright: bool,
    /// `RenderTypes.breezeWind`: the texture scrolls with the entity's age.
    pub scroll: f32,
}
/// DyeColor.getTextureDiffuseColor (RGB) and dye names.
pub use sunrise_level::dyes::{DYE_DIFFUSE as DYE, DYE_NAMES};
/// ABGR vertex colour from an RGB value.
pub fn abgr(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    0xFF00_0000 | b << 16 | g << 8 | r
}

/// `EquipmentLayerRenderer.getColorForLayer`: a dyeable layer takes the
/// stack's dye colour (`minecraft:dyed_color`), the colour it has when
/// undyed otherwise; a layer that is not dyeable is not tinted.
pub fn armor_tint(undyed: Option<u32>, stack: Option<&crate::items::ItemStack>) -> u32 {
    let Some(undyed) = undyed else {
        return 0xFFFF_FFFF;
    };
    let dyed = stack
        .and_then(|s| s.extra.as_ref())
        .and_then(|e| e.dyed_color)
        .map(|d| d as u32)
        .unwrap_or(undyed);
    abgr(dyed)
}
pub use crate::pack_palette::{diffuse as dye, sheep_wool as sheep_wool_rgb};
/// `ColorLerper.getLerpedColor(ColorLerper.Type.SHEEP, ageInTicks)`: the 16
/// wool colours in DyeColor order, one per 25 ticks, lerped in between. The
/// colour a `jeb_` sheep's wool takes.
pub fn jeb_wool_rgb(age: f32) -> u32 {
    // ageInTicks is the tick count plus the partial tick: never negative.
    let i = age.floor();
    let ticks = i as u64;
    let j = ticks / 25;
    let f = ((ticks % 25) as f32 + (age - i)) / 25.0;
    let lo = sheep_wool_rgb((j % 16) as u8);
    let hi = sheep_wool_rgb(((j + 1) % 16) as u8);
    let mix = |shift: u32| {
        let (lo, hi) = ((lo >> shift & 255) as f32, (hi >> shift & 255) as f32);
        ((lo + (hi - lo) * f) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}
pub use super::villager::VILLAGER_TYPES;
/// The saddle layer texture.
fn saddle_texture(s: Species) -> &'static str {
    match s {
        Species::Donkey => "entity/equipment/donkey_saddle/saddle",
        Species::Mule => "entity/equipment/mule_saddle/saddle",
        Species::SkeletonHorse => "entity/equipment/skeleton_horse_saddle/saddle",
        Species::ZombieHorse => "entity/equipment/zombie_horse_saddle/saddle",
        Species::Camel => "entity/equipment/camel_saddle/saddle",
        Species::CamelHusk => "entity/equipment/camel_husk_saddle/saddle",
        _ => "entity/equipment/horse_saddle/saddle",
    }
}

fn baby_or<'a>(
    m: &Mob,
    visuals: &ClientMobVisuals,
    adult: impl Into<Cow<'a, str>>,
    baby: impl Into<Cow<'a, str>>,
) -> Cow<'a, str> {
    let baby = baby.into();
    if m.is_baby() && visuals.layer(&baby).is_some() {
        baby
    } else {
        adult.into()
    }
}
#[path = "villager_client.rs"]
pub mod villager_client;
use villager_client::interned;

/// `AvatarRenderer` entities: a mannequin draws exactly like a player —
/// the player model, its resolved skin and the same scale.
pub fn avatar(m: &Mob) -> bool {
    matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin")
}
/// `AvatarRenderer.shouldRenderLayers`: a spectating player or mannequin
/// draws no layers at all (armour, held items, head item, cape, wings,
/// parrots, ears, stuck arrows), only its translucent head.
pub fn layers_shown(m: &Mob) -> bool {
    !(avatar(m) && m.extra.spectator)
}
/// The model layer key and texture for the mob's body.
pub fn body<'a>(m: &'a Mob, visuals: &'a ClientMobVisuals) -> (Cow<'static, str>, Cow<'a, str>) {
    body_with_variants(m, visuals, None)
}

/// [`body`] with the synced or world variant tables for registry-backed textures.
pub fn body_with_variants<'a>(
    m: &'a Mob,
    visuals: &'a ClientMobVisuals,
    variants: Option<&super::variants::VariantTables>,
) -> (Cow<'static, str>, Cow<'a, str>) {
    use Species as S;
    let baby = m.is_baby();
    let b = |s: &str| -> Cow<'static, str> {
        if baby {
            interned(format_args!("{s}_baby"))
        } else {
            interned(format_args!("{s}"))
        }
    };
    if avatar(m) {
        // AvatarRenderer: PlayerModel's slim layer for a slim skin.
        return (
            if m.slim { "player_slim#main" } else { "main" }.into(),
            m.skin.as_deref().unwrap_or(&visuals.texture).into(),
        );
    }
    match m.species() {
        S::Llama | S::TraderLlama => {
            let name = if m.species() == S::Llama {
                "llama"
            } else {
                "trader_llama"
            };
            (
                baby_or(
                    m,
                    visuals,
                    interned(format_args!("{name}#main")),
                    interned(format_args!("{name}_baby#main")),
                ),
                super::llama::texture(m).into(),
            )
        }
        s if super::equine::is_equine(s) => {
            let name = super::equine::model_name(s);
            (
                baby_or(
                    m,
                    visuals,
                    interned(format_args!("{name}#main")),
                    interned(format_args!("{name}_baby#main")),
                ),
                super::equine::texture(m).into(),
            )
        }
        S::Wolf => (
            baby_or(m, visuals, "wolf#main", "wolf_baby#main"),
            super::tame::texture(m, variants).into(),
        ),
        S::Panda => (
            baby_or(m, visuals, "panda#main", "panda_baby#main"),
            super::panda::texture(m).into(),
        ),
        S::HappyGhast => (
            baby_or(m, visuals, "happy_ghast#main", "happy_ghast_baby#main"),
            if baby {
                "entity/ghast/happy_ghast_baby".into()
            } else {
                "entity/ghast/happy_ghast".into()
            },
        ),
        S::Nautilus | S::ZombieNautilus => (
            super::nautilus::model(m, variants, visuals.layer("nautilus_baby#main").is_some())
                .into(),
            super::nautilus::texture(m, variants).into(),
        ),
        S::SulfurCube => {
            let (model, texture) = super::sulfur_cube::body(m);
            (model.into(), texture.into())
        }
        S::Sniffer => (
            baby_or(m, visuals, "sniffer#main", "sniffer_baby#main"),
            if baby {
                "entity/sniffer/snifflet".into()
            } else {
                "entity/sniffer/sniffer".into()
            },
        ),
        S::Shulker => ("shulker#main".into(), super::shulker::texture(m).into()),
        S::Vex => ("vex#main".into(), super::vex::texture(m).into()),
        S::Wither => ("wither#main".into(), super::wither::texture(m).into()),
        S::Ghast => (
            "ghast#main".into(),
            if m.extra.flag {
                "entity/ghast/ghast_shooting".into()
            } else {
                "entity/ghast/ghast".into()
            },
        ),
        S::Cat => (
            baby_or(m, visuals, "cat#main", "cat_baby#main"),
            super::tame::cat_texture(m, variants).into(),
        ),
        S::Frog => {
            let texture = variants
                .and_then(|v| {
                    let r = m.frog_variant(&v.frog)?;
                    Some(v.frog.get(r)?.texture().to_owned())
                })
                .unwrap_or_default();
            ("frog#main".into(), Cow::Owned(texture))
        }
        // RabbitRenderer.getTextureLocation: the Toast name tag, else the
        // variant (the killer bunny's id is 99).
        S::Rabbit => {
            let name = if m.custom_name.as_deref() == Some("Toast") {
                "toast"
            } else {
                match m.extra.variant {
                    1 => "white",
                    2 => "black",
                    3 => "white_splotched",
                    4 => "gold",
                    5 => "salt",
                    99 => "caerbannog",
                    _ => "brown",
                }
            };
            (
                baby_or(m, visuals, "rabbit#main", "rabbit_baby#main"),
                b(&interned(format_args!("entity/rabbit/rabbit_{name}"))),
            )
        }
        // ParrotRenderer.getVariantTexture.
        S::Parrot => {
            let name = ["red_blue", "blue", "green", "yellow_blue", "grey"]
                [usize::from(m.extra.variant).min(4)];
            (
                m.kind.model_name().into(),
                interned(format_args!("entity/parrot/parrot_{name}")),
            )
        }
        // AxolotlRenderer.getTextureLocation.
        S::Axolotl => {
            let name =
                ["lucy", "wild", "gold", "cyan", "blue"][usize::from(m.extra.variant).min(4)];
            (
                baby_or(m, visuals, "axolotl#main", "axolotl_baby#main"),
                b(&interned(format_args!("entity/axolotl/axolotl_{name}"))),
            )
        }
        // SalmonRenderer.submit: the model of the salmon's size.
        S::Salmon => {
            let model = match m.extra.variant {
                0 => "salmon_small#main",
                2 => "salmon_large#main",
                _ => "salmon#main",
            };
            (model.into(), "entity/fish/salmon".into())
        }
        // The baby models and textures of AgeableMobRenderer's other
        // subclasses.
        S::Goat => (
            baby_or(m, visuals, "goat#main", "goat_baby#main"),
            b(&visuals.texture),
        ),
        S::PolarBear => (
            baby_or(m, visuals, "polar_bear#main", "polar_bear_baby#main"),
            b(&visuals.texture),
        ),
        S::Ocelot => (
            baby_or(m, visuals, "ocelot#main", "ocelot_baby#main"),
            b(&visuals.texture),
        ),
        S::Turtle => (
            baby_or(m, visuals, "turtle#main", "turtle_baby#main"),
            b(&visuals.texture),
        ),
        S::Armadillo => (
            baby_or(m, visuals, "armadillo#main", "armadillo_baby#main"),
            b(&visuals.texture),
        ),
        // FoxRenderer.getTextureLocation: snow variant, sleeping, baby.
        S::Fox => (
            baby_or(m, visuals, "fox#main", "fox_baby#main"),
            interned(format_args!(
                "entity/fox/fox{}{}{}",
                if m.extra.variant == 0 { "" } else { "_snow" },
                if super::fox::flag(m, super::fox::SLEEPING) {
                    "_sleep"
                } else {
                    ""
                },
                if baby { "_baby" } else { "" },
            )),
        ),
        S::Pig => {
            if let Some(table) = variants.map(|v| &v.pig)
                && !table.is_empty()
            {
                if let Some(entry) = m.pig_variant(table).and_then(|r| table.get(r)) {
                    (
                        entry.model_key(baby).into(),
                        entry.texture(baby).to_owned().into(),
                    )
                } else {
                    (
                        if baby { "pig_baby#main" } else { "pig#main" }.into(),
                        Cow::Borrowed(""),
                    )
                }
            } else {
                (
                    if baby { "pig_baby#main" } else { "pig#main" }.into(),
                    b("entity/pig/pig_temperate"),
                )
            }
        }
        S::Cow => {
            if let Some(table) = variants.map(|v| &v.cow)
                && !table.is_empty()
            {
                if let Some(entry) = m.cow_variant(table).and_then(|r| table.get(r)) {
                    (
                        entry.model_key(baby).into(),
                        entry.texture(baby).to_owned().into(),
                    )
                } else {
                    (
                        if baby { "cow_baby#main" } else { "cow#main" }.into(),
                        Cow::Borrowed(""),
                    )
                }
            } else {
                (
                    if baby { "cow_baby#main" } else { "cow#main" }.into(),
                    b("entity/cow/cow_temperate"),
                )
            }
        }
        S::Mooshroom => (
            baby_or(m, visuals, "mooshroom#main", "mooshroom_baby#main"),
            b(if m.extra.variant == 1 {
                "entity/cow/mooshroom_brown"
            } else {
                "entity/cow/mooshroom_red"
            }),
        ),
        S::Chicken => {
            if let Some(table) = variants.map(|v| &v.chicken)
                && !table.is_empty()
            {
                if let Some(entry) = m.chicken_variant(table).and_then(|r| table.get(r)) {
                    (
                        entry.model_key(baby).into(),
                        entry.texture(baby).to_owned().into(),
                    )
                } else {
                    (
                        if baby {
                            "chicken_baby#main"
                        } else {
                            "chicken#main"
                        }
                        .into(),
                        Cow::Borrowed(""),
                    )
                }
            } else {
                (
                    if baby {
                        "chicken_baby#main"
                    } else {
                        "chicken#main"
                    }
                    .into(),
                    b("entity/chicken/chicken_temperate"),
                )
            }
        }
        S::Sheep => (
            baby_or(m, visuals, "sheep#main", "sheep_baby#main"),
            b("entity/sheep/sheep"),
        ),
        S::Zombie => (
            baby_or(m, visuals, "zombie#main", "zombie_baby#main"),
            b("entity/zombie/zombie"),
        ),
        S::Husk => (
            baby_or(m, visuals, "husk#main", "husk_baby#main"),
            b("entity/zombie/husk"),
        ),
        S::Drowned => (
            baby_or(m, visuals, "drowned#main", "drowned_baby#main"),
            b("entity/zombie/drowned"),
        ),
        S::ZombieVillager => (
            baby_or(
                m,
                visuals,
                "zombie_villager#main",
                "zombie_villager_baby#main",
            ),
            b("entity/zombie_villager/zombie_villager"),
        ),
        S::Villager => (
            baby_or(m, visuals, "villager#main", "villager_baby#main"),
            b("entity/villager/villager"),
        ),
        S::Piglin => (
            baby_or(m, visuals, "piglin#main", "piglin_baby#main"),
            b("entity/piglin/piglin"),
        ),
        S::ZombifiedPiglin => (
            baby_or(
                m,
                visuals,
                "zombified_piglin#main",
                "zombified_piglin_baby#main",
            ),
            b("entity/piglin/zombified_piglin"),
        ),
        S::Hoglin => (
            baby_or(m, visuals, "hoglin#main", "hoglin_baby#main"),
            b("entity/hoglin/hoglin"),
        ),
        S::Zoglin => (
            baby_or(m, visuals, "zoglin#main", "zoglin_baby#main"),
            b("entity/hoglin/zoglin"),
        ),
        S::Strider => (
            baby_or(m, visuals, "strider#main", "strider_baby#main"),
            b(if m.extra.flag {
                "entity/strider/strider_cold"
            } else {
                "entity/strider/strider"
            }),
        ),
        // BeeRenderer.getTextureLocation: angry, carrying nectar, or both.
        S::Bee => {
            let angry = super::bee::get(m).is_some_and(|b| b.anger_end > 0);
            let nectar = super::bee::has_nectar(m);
            let texture = match (angry, nectar) {
                (true, true) => "entity/bee/bee_angry_nectar",
                (true, false) => "entity/bee/bee_angry",
                (false, true) => "entity/bee/bee_nectar",
                (false, false) => "entity/bee/bee",
            };
            let layer = baby_or(m, visuals, "bee#main", "bee_baby#main");
            let texture = if layer.starts_with("bee_baby") {
                b(texture)
            } else {
                texture.into()
            };
            (layer, texture)
        }
        S::Squid => (
            baby_or(m, visuals, "squid#main", "squid_baby#main"),
            b("entity/squid/squid"),
        ),
        S::GlowSquid => (
            baby_or(m, visuals, "glow_squid#main", "glow_squid_baby#main"),
            b("entity/squid/glow_squid"),
        ),
        S::Dolphin => (
            baby_or(m, visuals, "dolphin#main", "dolphin_baby#main"),
            b("entity/dolphin/dolphin"),
        ),
        S::Pufferfish => {
            let model = match m.extra.size {
                0 => "pufferfish_small#main",
                1 => "pufferfish_medium#main",
                _ => "pufferfish_big#main",
            };
            (model.into(), "entity/fish/pufferfish".into())
        }
        S::TropicalFish => {
            if m.extra.variant < 6 {
                (
                    "tropical_fish_small#main".into(),
                    "entity/fish/tropical_a".into(),
                )
            } else {
                (
                    "tropical_fish_large#main".into(),
                    "entity/fish/tropical_b".into(),
                )
            }
        }
        // CopperGolemOxidationLevels: the texture of the weathering.
        S::CopperGolem => (
            m.kind.model_name().into(),
            super::copper_golem::texture(m).into(),
        ),
        _ => (m.kind.model_name().into(), visuals.texture.as_str().into()),
    }
}
/// LivingEntityRenderer.getModelTint (tropical fish base colour).
pub fn body_color(m: &Mob, alpha: f32) -> u32 {
    match m.species() {
        Species::Wolf => super::tame::tint(m, alpha),
        Species::TropicalFish => abgr(dye(m.extra.variant2 as usize)),
        // LivingEntityRenderer.submit: a spectator is invisible
        // (Player.updateInvisibilityStatus) but not to another spectator or
        // itself, so its body is forced translucent (654311423, alpha 38).
        _ if avatar(m) && m.extra.spectator => 0x26FF_FFFF,
        _ => 0xFFFF_FFFF,
    }
}
/// Extra render layers drawn over the body with the same pose. `age` is the
/// render state's `ageInTicks` (`Mob.age` plus the partial tick).
pub fn layers(m: &Mob, age: f32) -> Vec<Layer> {
    use Species as S;
    let mut v = Vec::new();
    if avatar(m) {
        return v;
    }
    let white = 0xFFFF_FFFF;
    fn push(
        v: &mut Vec<Layer>,
        model: impl Into<Cow<'static, str>>,
        texture: impl Into<Cow<'static, str>>,
        color: u32,
        full_bright: bool,
    ) {
        v.push(Layer {
            equipment_overlay: None,
            equipment_hidden: false,
            equipment_model: None,
            equipment_uv: None,
            model: model.into(),
            texture: texture.into(),
            color,
            full_bright,
            scroll: 0.0,
        })
    }
    match m.species() {
        // WolfArmorLayer, then WolfCollarLayer.
        S::Wolf if m.extra.tame => {
            let (model, texture) = if m.is_baby() {
                ("wolf_baby#main", "entity/wolf/wolf_collar_baby")
            } else {
                ("wolf#main", "entity/wolf/wolf_collar")
            };
            // The armor, then the cracks of its damage (`Crackiness.WOLF_ARMOR`).
            if !m.is_baby() && m.extra.body_id() == Some("minecraft:wolf_armor") {
                let left = (super::WOLF_ARMOR_DURABILITY - m.extra.body_damage()) as f32
                    / super::WOLF_ARMOR_DURABILITY as f32;
                push(
                    &mut v,
                    "wolf_armor#main",
                    "entity/equipment/wolf_body/armadillo_scute",
                    white,
                    false,
                );
                let crack = [(0.32, "high"), (0.69, "medium"), (0.95, "low")]
                    .iter()
                    .find(|(t, _)| left < *t);
                if let Some((_, level)) = crack {
                    push(
                        &mut v,
                        "wolf_armor#main",
                        interned(format_args!("entity/wolf/wolf_armor_crackiness_{level}")),
                        white,
                        false,
                    );
                }
            }
            push(
                &mut v,
                model,
                texture,
                abgr(dye(m.extra.color as usize)),
                false,
            );
        }
        // SimpleEquipmentLayer: the harness over the body.
        S::HappyGhast => {
            let harness = m
                .extra
                .body_id()
                .and_then(super::happy_ghast::harness_texture);
            if !m.is_baby()
                && let Some(texture) = &harness
            {
                push(
                    &mut v,
                    "happy_ghast_harness#main",
                    texture.clone(),
                    white,
                    false,
                );
            }
            // RopesLayer: the ropes over a harnessed body while the ghast
            // holds a quad lead (`isLeashHolder`), the baby's model for a baby.
            if harness.is_some() && super::happy_ghast::is_leash_holder(m) {
                let model = if m.is_baby() {
                    "happy_ghast_baby_ropes#main"
                } else {
                    "happy_ghast_ropes#main"
                };
                push(
                    &mut v,
                    model,
                    "entity/ghast/happy_ghast_ropes",
                    white,
                    false,
                );
            }
        }
        // BreezeWindLayer and BreezeEyesLayer. The wind layer's texture
        // scrolls (`RenderTypes.breezeWind`): the offset is taken inside the
        // sprite, so the atlas does not have to wrap.
        S::Breeze => {
            v.push(Layer {
                equipment_overlay: None,
                equipment_hidden: false,
                equipment_model: None,
                equipment_uv: None,
                model: "breeze#wind".into(),
                texture: "entity/breeze/breeze_wind".into(),
                color: white,
                full_bright: false,
                scroll: scroll_offset(age, BREEZE_WIND_SCROLL),
            });
            push(
                &mut v,
                "breeze#eyes",
                "entity/breeze/breeze_eyes",
                white,
                true,
            );
        }
        // LlamaDecorLayer: the carpet, or the trader's blanket.
        S::Llama | S::TraderLlama => {
            if let Some((model, texture)) = super::llama::decor(m) {
                push(&mut v, model, texture, white, false);
            }
        }
        // HorseMarkingLayer and the saddle; the body armour is an equipment layer.
        s if super::equine::is_equine(s) => {
            let name = super::equine::model_name(s);
            let body = if m.is_baby() {
                interned(format_args!("{name}_baby#main"))
            } else {
                interned(format_args!("{name}#main"))
            };
            if let Some(tex) = super::equine::markings(m) {
                push(&mut v, body, tex, white, false);
            }
            if m.extra.saddled() && !m.is_baby() {
                push(
                    &mut v,
                    interned(format_args!("{name}#saddle")),
                    saddle_texture(s),
                    white,
                    false,
                );
            }
        }
        // SimpleEquipmentLayer: pig and strider saddles.
        S::Pig if m.extra.saddled() && !m.is_baby() => push(
            &mut v,
            "pig#saddle",
            "entity/equipment/pig_saddle/saddle",
            white,
            false,
        ),
        S::Strider if m.extra.saddled() && !m.is_baby() => push(
            &mut v,
            "strider#saddle",
            "entity/equipment/strider_saddle/saddle",
            white,
            false,
        ),
        // CatCollarLayer.
        S::Cat if m.extra.tame => {
            let (model, texture) = if m.is_baby() {
                ("cat_baby#collar", "entity/cat/cat_collar_baby")
            } else {
                ("cat#collar", "entity/cat/cat_collar")
            };
            push(
                &mut v,
                model,
                texture,
                abgr(dye(m.extra.color as usize)),
                false,
            );
        }
        // IronGolemCrackinessLayer.
        S::IronGolem => {
            if let Some(texture) = super::golem::crackiness(m) {
                push(&mut v, "iron_golem#main", texture, white, false);
            }
        }
        // NautilusRenderer: the saddle SimpleEquipmentLayer; the armour is an equipment layer.
        S::Nautilus | S::ZombieNautilus if !m.is_baby() => {
            if m.extra.saddled() {
                push(
                    &mut v,
                    "nautilus#saddle",
                    "entity/equipment/nautilus_saddle/saddle",
                    white,
                    false,
                );
            }
        }
        // SulfurCubeInnerLayer.
        S::SulfurCube => {
            if let Some((model, texture)) = super::sulfur_cube::inner(m) {
                push(&mut v, model, texture, white, false);
            }
        }
        // SlimeOuterLayer: the translucent outer cube.
        S::Slime => push(&mut v, "slime#outer", "entity/slime/slime", white, false),
        // SheepWoolUndercoatLayer has no isSheared guard: a sheared dyed sheep
        // keeps it, while only SheepWoolLayer stops.
        S::Sheep => {
            // MobRenderer.checkMagicName(entity, "jeb_"): an exact custom name.
            let jeb = m.custom_name.as_deref() == Some("jeb_");
            let color = abgr(if jeb {
                jeb_wool_rgb(age)
            } else {
                sheep_wool_rgb(m.extra.color)
            });
            if !m.is_baby() && (jeb || m.extra.color != 0) {
                push(
                    &mut v,
                    "sheep#wool_undercoat",
                    "entity/sheep/sheep_wool_undercoat",
                    color,
                    false,
                );
            }
            // SheepWoolLayer: none once sheared.
            if !m.extra.sheared {
                let (model, texture) = if m.is_baby() {
                    ("sheep_baby#wool", "entity/sheep/sheep_wool_baby")
                } else {
                    ("sheep#wool", "entity/sheep/sheep_wool")
                };
                push(&mut v, model, texture, color, false);
            }
        }
        S::Creaking if super::creaking::eyes_glowing(m) => push(
            &mut v,
            "creaking#eyes",
            "entity/creaking/creaking_eyes",
            white,
            true,
        ),
        // The copper golem's eyes glow: CopperGolemRenderer's
        // LivingEntityEmissiveLayer (eyes render type) draws on a model baked
        // from ModelLayers.COPPER_GOLEM, the body's, not COPPER_GOLEM_EYES.
        S::CopperGolem => {
            let eyes = super::copper_golem::eye_texture(m);
            push(&mut v, m.kind.model_name(), eyes, white, true);
        }
        S::Spider | S::CaveSpider => push(
            &mut v,
            m.kind.model_name(),
            "entity/spider/spider_eyes",
            white,
            true,
        ),
        S::Enderman => push(
            &mut v,
            "enderman#main",
            "entity/enderman/enderman_eyes",
            white,
            true,
        ),
        S::Phantom => push(
            &mut v,
            "phantom#main",
            "entity/phantom/phantom_eyes",
            white,
            true,
        ),
        S::Drowned => {
            if m.is_baby() {
                push(
                    &mut v,
                    "drowned_baby#outer",
                    "entity/zombie/drowned_outer_layer_baby",
                    white,
                    false,
                );
            } else {
                push(
                    &mut v,
                    "drowned#outer",
                    "entity/zombie/drowned_outer_layer",
                    white,
                    false,
                );
            }
        }
        S::Stray => push(
            &mut v,
            "stray#outer",
            "entity/skeleton/stray_overlay",
            white,
            false,
        ),
        S::Bogged if !m.extra.sheared => push(
            &mut v,
            "bogged#outer",
            "entity/skeleton/bogged_overlay",
            white,
            false,
        ),
        S::TropicalFish => {
            // TropicalFishPatternLayer: the pattern tinted with the pattern colour.
            let (model, letter, n) = if m.extra.variant < 6 {
                ("tropical_fish_small#pattern", "a", m.extra.variant + 1)
            } else {
                ("tropical_fish_large#pattern", "b", m.extra.variant - 5)
            };
            push(
                &mut v,
                model,
                interned(format_args!("entity/fish/tropical_{letter}_pattern_{n}")),
                abgr(dye(m.extra.variant3 as usize)),
                false,
            );
        }
        S::Villager | S::ZombieVillager => {
            let base = if m.species() == S::Villager {
                "villager"
            } else {
                "zombie_villager"
            };
            let model = if m.is_baby() {
                interned(format_args!("{base}_baby#main"))
            } else {
                interned(format_args!("{base}#main"))
            };
            let t = VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()];
            let folder = if m.species() == S::Villager {
                "entity/villager"
            } else {
                "entity/zombie_villager"
            };
            // VillagerProfessionLayer: a baby's type texture is in `baby/`.
            let group = if m.is_baby() { "baby" } else { "type" };
            push(
                &mut v,
                model.clone(),
                interned(format_args!("{folder}/{group}/{t}")),
                white,
                false,
            );
            let nitwit = m.extra.profession.index() == Some(11);
            if !m.is_baby()
                && let Some(texture) = villager_client::texture(
                    &m.extra.profession,
                    folder == "entity/zombie_villager",
                )
            {
                push(&mut v, model.clone(), texture, white, false);
                if !nitwit {
                    // `LEVEL_LOCATIONS` clamps the level to 1..=5.
                    let level = ["stone", "iron", "gold", "emerald", "diamond"]
                        [(m.extra.villager_level.max(1) as usize - 1).min(4)];
                    push(
                        &mut v,
                        model,
                        interned(format_args!("{folder}/profession_level/{level}")),
                        white,
                        false,
                    );
                }
            }
        }
        _ => {}
    }
    v
}
/// Whether this renderer has the generic head item route (`CustomHeadLayer`).
pub fn has_head_item_layer(m: &Mob) -> bool {
    let species = m.species();
    species != Species::SnowGolem
        && !super::species::body_slot(species)
        && !super::equine::is_equine(species)
}

/// The existing humanoid equipment model, without rebuilding its key each frame.
pub fn equipment_model_key(
    mob: &Mob,
    layer: &str,
    models: &ClientMobModels,
) -> Option<Cow<'static, str>> {
    let visuals = models.get(&mob.kind.id)?;
    let (model, _) = body(mob, visuals);
    let base = model.strip_suffix("#main")?;
    let key = interned(format_args!("{base}#{layer}"));
    visuals.layer(&key).map(|_| key)
}

impl Layer {
    /// A resolved equipment pass; names are interned with the other mob material keys.
    pub fn equipment(model: &str, texture: &str, color: u32, overlay: u32) -> Self {
        Self {
            model: interned(format_args!("{model}")),
            texture: interned(format_args!("{texture}")),
            color,
            equipment_overlay: Some(overlay),
            equipment_hidden: false,
            equipment_model: None,
            equipment_uv: None,
            full_bright: false,
            scroll: 0.0,
        }
    }
}
/// The mob's root transform (LivingEntityRenderer.submit + setupRotations).
pub fn base_transform(m: &Mob, alpha: f32, camera: glam::DVec3) -> glam::Mat4 {
    let a = f64::from(alpha);
    let p = glam::DVec3::new(
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ) - camera
        + enderman_render_offset(m, alpha)
        + super::shulker::render_offset(m, alpha);
    base_transform_at(m, alpha, glam::Mat4::from_translation(p.as_vec3()))
}
/// `EndermanRenderer.getRenderOffset`: a creepy (angry) enderman is moved by
/// a gaussian 0.02 blocks sideways in x and z every frame. Vanilla draws from
/// the renderer's own random; this one is seeded by the mob and the frame's
/// age, so the body, its carried block and its eyes agree within a frame.
fn enderman_render_offset(m: &Mob, alpha: f32) -> glam::DVec3 {
    use sunrise_core::random::{LegacyRandom, RandomSource};
    if m.species() != Species::Enderman || !m.extra.flag {
        return glam::DVec3::ZERO;
    }
    let frame = (m.age as f32 + alpha).to_bits();
    let mut random = LegacyRandom::new((i64::from(m.id) << 32) ^ i64::from(frame));
    // `0.02 * state.scale`.
    let d = 0.02 * f64::from(m.model_scale());
    let x = random.next_gaussian() * d;
    let z = random.next_gaussian() * d;
    glam::DVec3::new(x, 0.0, z)
}
/// `LivingEntityRenderer.setupRotations` and the rest of the model transform, from the
/// entity's origin pose (`EntityRenderDispatcher.submit`, including mobs drawn in spawners).
#[sunrise_mod_api_macros::replaceable]
pub fn base_transform_at(m: &Mob, alpha: f32, origin: glam::Mat4) -> glam::Mat4 {
    let mut body = anim::body_rot(m, alpha);
    if shaking(m) {
        // LivingEntityRenderer.setupRotations: a rapid wobble.
        let ticks = (m.age as f32 + alpha).floor();
        body += (f64::from(ticks * 3.25).cos() * std::f64::consts::PI * 0.4) as f32;
    }
    // A sleeping villager, or another player lying in their bed.
    let sleep = (m.species() == Species::Villager || avatar(m))
        .then(|| super::villager::sleep_pose(m))
        .flatten();
    // `LivingEntityRenderer.submit` moves a sleeper toward the pillow before
    // it scales the model, so the bed offset is not scaled and the pose after
    // it is. Keep the two apart.
    let (sleep_offset, rotation) = match sleep {
        Some((angle, step)) => {
            // The eye height of the standing pose (LivingEntityRenderer
            // extracts `getEyeHeight(Pose.STANDING)` for a bed).
            let eye = if avatar(m) {
                // `getEyeHeight(Pose.STANDING)` on an avatar is the standing
                // box's eye height, `minecraft:scale` included.
                m.kind.eye_height as f32 * m.model_scale()
            } else {
                m.awake_dims().2 as f32
            };
            let head = eye - 0.1;
            (
                glam::Mat4::from_translation(glam::Vec3::new(
                    -step[0] * head,
                    0.0,
                    -step[1] * head,
                )),
                // setupRotations for Pose.SLEEPING.
                glam::Mat4::from_rotation_y(angle.to_radians())
                    * glam::Mat4::from_rotation_z(90.0_f32.to_radians())
                    * glam::Mat4::from_rotation_y(270.0_f32.to_radians()),
            )
        }
        None => (
            glam::Mat4::IDENTITY,
            super::aquatic::rotations(m, alpha, body)
                .unwrap_or_else(|| glam::Mat4::from_rotation_y((180.0 - body).to_radians())),
        ),
    };
    // `LivingEntityRenderer.submit`: `poseStack.scale(state.scale, ...)` for
    // every living entity, after the sleeping offset and before
    // setupRotations, so a pose translation scales with the model but the bed
    // it lies in does not.
    let entity_scale = m.model_scale();
    let mut base = origin
        * sleep_offset
        * glam::Mat4::from_scale(glam::Vec3::splat(entity_scale))
        * pufferfish_bob(m, alpha)
        * rotation;
    if m.death_time > 0 && !super::creaking::tearing_down(m) {
        let f = ((m.death_time as f32 + alpha - 1.0) / 20.0 * 1.6)
            .sqrt()
            .min(1.0);
        base *= glam::Mat4::from_rotation_z(f * flip_degrees(m).to_radians());
    } else if m.extra.auto_spin_attack {
        // Riptide: lying along the look direction, spinning 75 degrees a tick.
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        base *= glam::Mat4::from_rotation_x((-90.0 - pitch).to_radians())
            * glam::Mat4::from_rotation_y(((m.age as f32 + alpha) * -75.0).to_radians());
    } else if anim::upside_down(m) && m.pose() != mod_api::Pose::Sleeping {
        // The box height already includes entity scale. Undo the pose's scale
        // for the whole offset, including the 0.1-block clearance.
        let lift = (m.height() + 0.1) / entity_scale;
        base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, lift, 0.0))
            * glam::Mat4::from_rotation_z(PI);
    }
    // IronGolemRenderer.setupRotations, after LivingEntityRenderer's: the body
    // tilts side to side while it walks.
    if m.species() == Species::IronGolem {
        let s = anim::state(m, alpha);
        if s.speed >= 0.01 {
            let wp = s.pos + 6.0;
            let triangle_wave = ((wp % 13.0 - 6.5).abs() - 3.25) / 3.25;
            base *= glam::Mat4::from_rotation_z((6.5 * triangle_wave).to_radians());
        }
    }
    // AvatarRenderer.setupRotations, after LivingEntityRenderer's.
    if m.extra.fall_flying {
        let ticks = m.extra.fall_fly_ticks as f32 + alpha;
        let scale = (ticks * ticks / 100.0).min(1.0);
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        if !m.extra.auto_spin_attack {
            base *= glam::Mat4::from_rotation_x((scale * (-90.0 - pitch)).to_radians());
        }
        let yaw = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
        let (look_x, look_z) = (-(yaw.to_radians()).msin(), (yaw.to_radians()).mcos());
        let v = m.extra.previous_velocity.lerp(m.velocity, f64::from(alpha));
        let view_scale = (pitch.to_radians()).mcos();
        let (look_x, look_z) = (look_x * view_scale, look_z * view_scale);
        let view_length = f64::from(look_x * look_x + look_z * look_z);
        let speed = v.x * v.x + v.z * v.z;
        if speed > f64::from(1.0e-5f32) && view_length > f64::from(1.0e-5f32) {
            let dot =
                (v.x * f64::from(look_x) + v.z * f64::from(look_z)) / (speed * view_length).sqrt();
            let sign = (v.x * f64::from(look_z) - v.z * f64::from(look_x)).signum();
            base *= glam::Mat4::from_rotation_y(
                (sign * sunrise_core::jmath::acos(dot.abs().min(1.0))) as f32,
            );
        }
    } else if m.species() == Species::Drowned {
        // DrownedRenderer.setupRotations: tip toward the look pitch about
        // the middle of the box while swimming.
        let amount = m.extra.swim.amount(alpha);
        if amount > 0.0 {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            let middle = glam::Vec3::new(0.0, m.height() / 2.0, 0.0);
            base *= glam::Mat4::from_translation(middle)
                * glam::Mat4::from_rotation_x((amount * (-10.0 - pitch)).to_radians())
                * glam::Mat4::from_translation(-middle);
        }
    } else if m.extra.swim.amount(alpha) > 0.0 {
        // AvatarRenderer.setupRotations: tip forward with the swim amount,
        // following the pitch in water, then lie along the swimming box.
        let amount = m.extra.swim.amount(alpha);
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        let target = if m.extra.swim.in_water {
            -90.0 - pitch
        } else {
            -90.0
        };
        base *= glam::Mat4::from_rotation_x((amount * target).to_radians());
        if m.extra.swim.visually {
            base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.0, 0.3));
        }
    }
    base *= species_rotations(m, alpha);
    base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
    base *= species_scale(m, alpha);
    base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0))
}
/// `PufferfishRenderer.setupRotations`: `translate(0, cos(ageInTicks * 0.05) * 0.08, 0)`
/// before the usual rotations, so a pufferfish bobs up and down.
fn pufferfish_bob(m: &Mob, alpha: f32) -> glam::Mat4 {
    if m.species() != Species::Pufferfish {
        return glam::Mat4::IDENTITY;
    }
    glam::Mat4::from_translation(glam::Vec3::new(
        0.0,
        pufferfish_bob_y(m.age as f32 + alpha),
        0.0,
    ))
}
fn pufferfish_bob_y(age_in_ticks: f32) -> f32 {
    (age_in_ticks * 0.05).mcos() * 0.08
}
/// `LivingEntityRenderer.isShaking`: a fully frozen mob (the skeleton's own
/// `isShaking` is its stray conversion only), converting zombies, piglins
/// and hoglins, a zombie villager being cured and a cold strider.
fn shaking(m: &Mob) -> bool {
    if m.is_fully_frozen() && m.species() != Species::Skeleton {
        return true;
    }
    match m.species() {
        Species::Strider => super::strider::suffocating(m),
        Species::ZombieVillager => super::zombie_villager::is_converting(m),
        Species::Zombie | Species::Husk | Species::Skeleton => m.extra.counter2 > 0,
        Species::Piglin | Species::PiglinBrute | Species::Hoglin => {
            !m.extra.flag2 && m.extra.counter2 > 0
        }
        _ => false,
    }
}
fn flip_degrees(m: &Mob) -> f32 {
    match m.species() {
        Species::Spider | Species::CaveSpider | Species::Silverfish | Species::Endermite => 180.0,
        _ => 90.0,
    }
}
fn species_rotations(m: &Mob, alpha: f32) -> glam::Mat4 {
    match m.species() {
        Species::Cat => super::tame::cat_rotations(m, alpha),
        Species::Panda => super::panda::rotations(m, alpha),
        Species::Shulker => super::shulker::rotations(m),
        // FoxRenderer.setupRotations: the body follows a pounce's pitch.
        Species::Fox if super::fox::flag(m, super::fox::POUNCING | super::fox::FACEPLANTED) => {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            glam::Mat4::from_rotation_x((-pitch).to_radians())
        }
        // PhantomRenderer.setupRotations: the body follows its pitch.
        Species::Phantom => {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            glam::Mat4::from_rotation_x(pitch.to_radians())
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Creeper.getSwelling: the fuse's progress, past 1 in its last two ticks.
fn creeper_swelling(m: &Mob, alpha: f32) -> f32 {
    (m.previous_swell as f32 + alpha * (m.swell - m.previous_swell) as f32)
        / (m.max_swell - 2) as f32
}
/// LivingEntityRenderer.getOverlayCoords for the body: the hurt red, or a
/// swelling creeper's white flash. Layers take the red only (progress 0).
fn body_overlay(m: &Mob, alpha: f32) -> u32 {
    let white = if m.species() == Species::Creeper {
        crate::overlay::creeper_progress(creeper_swelling(m, alpha))
    } else {
        0.0
    };
    crate::overlay::living(white, red_overlay(m))
}
/// LivingEntityRenderState.hasRedOverlay: hurt or dying (not a creaking
/// tearing down).
pub fn red_overlay(m: &Mob) -> bool {
    m.hurt_time > 0 || (m.death_time > 0 && !super::creaking::tearing_down(m))
}
fn species_scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    if avatar(m) {
        // AvatarRenderer.scale, shared by the inventory and world renderer.
        // The entity's own `scale` is `LivingEntityRenderer.submit`'s, applied
        // by `base_transform_at` before setupRotations.
        return glam::Mat4::from_scale(glam::Vec3::splat(0.9375));
    }
    match m.species() {
        Species::Wither => {
            glam::Mat4::from_scale(glam::Vec3::splat(super::wither::scale(m, alpha)))
        }
        Species::Creeper => {
            // CreeperRenderer.scale: swelling with a wobble.
            let g = creeper_swelling(m, alpha);
            let wobble = 1.0 + (g * 100.0).msin() * g * 0.01;
            let s = g.clamp(0.0, 1.0);
            let s4 = s * s * s * s;
            let xz = (1.0 + s4 * 0.4) * wobble;
            let y = (1.0 + s4 * 0.1) / wobble;
            glam::Mat4::from_scale(glam::Vec3::new(xz, y, xz))
        }
        Species::Slime | Species::MagmaCube => super::cube::scale(m, alpha),
        Species::SulfurCube => super::sulfur_cube::scale(m, alpha),
        Species::Phantom => {
            // PhantomRenderer.scale: 15% bigger per size, and the model moved
            // down to the box (it is modelled 1.3125 blocks above the feet).
            let s = 1.0 + 0.15 * m.extra.size as f32;
            glam::Mat4::from_scale(glam::Vec3::splat(s))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.3125, 0.1875))
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Draw a part tree with a pose map, texture rect, colour and light.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    draw_scrolled(part, base, poses, uv, 0.0, color, light, overlay, out);
}
/// `draw` for a texture that scrolls: the vertex's U is moved by the offset
/// and wraps inside the texture, as the texture matrix of
/// `RenderTypes.breezeWind` does.
#[allow(clippy::too_many_arguments)]
pub fn draw_scrolled(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let paint = Paint {
        uv,
        scroll,
        color,
        light,
        overlay,
    };
    draw_normals(
        part,
        base,
        ModelNormals::new(base),
        poses,
        paint,
        part.layer.as_deref(),
        compiler(),
        &mut Quads::new(out),
    );
}
/// [`compile_part`]'s signature, for a draw that goes through it.
pub(crate) type CompilePart =
    fn(Option<&str>, &Part, glam::Mat4, ModelNormals, Paint, &Poses, &mut Quads<'_>);
/// The replaced [`compile_part`], when a mod installed one. Asked once per draw, not per part:
/// while no mod replaced it, the parts' polygons are pushed as they always were, without going
/// through it.
pub(crate) fn compiler() -> Option<CompilePart> {
    replaced!(compile_part).then_some(compile_part as CompilePart)
}
/// `ModelPart.render`: the part and its children under their poses. `layer` is the model
/// layer of the tree drawn (its root's), `compile` the replaced [`compile_part`] when a mod
/// installed one.
#[allow(clippy::too_many_arguments)]
fn draw_normals(
    part: &Part,
    base: glam::Mat4,
    normals: ModelNormals,
    poses: &Poses,
    paint: Paint,
    layer: Option<&str>,
    compile: Option<CompilePart>,
    quads: &mut Quads<'_>,
) {
    let p = poses.pose(part);
    let m = posed(base, p);
    if poses.is_hidden(part) || p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
        return;
    }
    let normals = normals.part(p);
    // `skips` is a name lookup: ask once per part, not once per polygon. A part
    // hidden this way still draws its children, so only the polygons are gated.
    if !poses.skips(part) {
        emit_part(compile, layer, part, (m, normals), paint, poses, quads);
    }
    for c in &part.children {
        draw_normals(c, m, normals, poses, paint, layer, compile, quads);
    }
}
/// One part's own geometry, as every draw of a baked model emits it: through the replaced
/// [`compile_part`] when `compile` holds it, its polygons as they are otherwise. A part with
/// neither polygons nor children (an empty leaf) has nothing a replacement could change, and
/// is never handed to it. `posed` is the part's matrix and normals, as [`posed`] and
/// `ModelNormals::part` give them.
pub(crate) fn emit_part(
    compile: Option<CompilePart>,
    layer: Option<&str>,
    part: &Part,
    (matrix, normals): (glam::Mat4, ModelNormals),
    paint: Paint,
    poses: &Poses,
    quads: &mut Quads<'_>,
) {
    match compile {
        Some(compile) if !(part.polygons.is_empty() && part.children.is_empty()) => {
            compile(layer, part, matrix, normals, paint, poses, quads);
        }
        _ => push_polygons(&part.polygons, matrix, normals, paint, quads),
    }
}
/// Where [`compile_part`] puts a part's geometry: whole quads, appended after what the draw
/// already holds, which it can neither read nor remove.
pub struct Quads<'a> {
    out: &'a mut Vec<Vertex>,
}
impl<'a> Quads<'a> {
    /// Quads appended to `out`.
    pub(crate) fn new(out: &'a mut Vec<Vertex>) -> Self {
        Self { out }
    }
    /// Appends one quad, its four vertices in order.
    pub fn push(&mut self, quad: [Vertex; 4]) {
        self.out.extend_from_slice(&quad);
    }
}
/// `ModelPart.compile` (`ModelPart.Cube.compile` per cube): emits one part's own geometry,
/// posed by `matrix` (the part's, as [`posed`] gives it under its parents) with the normals of
/// its pose, painted with `paint`. Its children are drawn after it, each through this function
/// too. Parts hidden, scaled to nothing, with `skipDraw` set or with neither polygons nor
/// children don't come here.
///
/// A native mod replaces this through `[[native.mixin]]` to emit a part's geometry its own
/// way while the rest of the draw stays the game's: a limb bent in two segments, a squashed or
/// stretched part, another mesh. `layer` is the model layer of the tree being drawn
/// (`minecraft:player#main`; None for a model not read from `entity-models.json`) and
/// `part.name` the part. An entity's own state reaches the replacement through `poses`, where
/// the mod's mixin on `anim::setup` stores it ([`Poses::set_data`]); a draw posed without
/// `anim` (an arrow, the first-person arm) passes empty poses. The replacement appends whole
/// quads to `quads` and sees nothing drawn before; it can call [`push_polygons`] for the
/// polygons it leaves as they are. A model is drawn on the GPU without this function unless
/// one of its parts is taken by [`takes_part`], so a mod that replaces this function replaces
/// that one too. Without a replacement, the same vertices in the same order as before this
/// function existed.
#[sunrise_mod_api_macros::replaceable]
pub fn compile_part(
    layer: Option<&str>,
    part: &Part,
    matrix: glam::Mat4,
    normals: ModelNormals,
    paint: Paint,
    poses: &Poses,
    quads: &mut Quads<'_>,
) {
    let _ = (layer, poses);
    push_polygons(&part.polygons, matrix, normals, paint, quads);
}
/// Whether a native mod draws the part `part` of the model layer `layer`
/// (`minecraft:player#main`) itself, in its replacement of [`compile_part`]. A model with a
/// part taken is drawn on the CPU, where [`compile_part`] runs. The mob's other eligible models
/// stay on the GPU, in the same draw order.
///
/// The answer must be fixed per (layer, part) for the whole session: it is asked once per
/// model, the first time the GPU path sees it, and kept. What changes per entity or per frame
/// (a bend that is on or off) belongs in the replacement of [`compile_part`], which can push
/// the part's own polygons when that entity has nothing to change.
#[sunrise_mod_api_macros::replaceable]
pub fn takes_part(layer: &str, part: &str) -> bool {
    let _ = (layer, part);
    false
}
/// Whether a mixin replaced [`takes_part`]: while none has, no model is asked about.
pub(super) fn parts_taken_hooked() -> bool {
    replaced!(takes_part)
}
/// The quads of `polygons` posed by `m`: [`compile_part`]'s own body, which a replacement
/// can call for the parts or polygons it leaves as they are.
pub fn push_polygons(
    polygons: &[super::Polygon],
    m: glam::Mat4,
    normals: ModelNormals,
    paint: Paint,
    quads: &mut Quads<'_>,
) {
    let Paint {
        uv,
        scroll,
        color,
        light,
        overlay,
    } = paint;
    for poly in polygons {
        let normal = poly.vertex_normal(normals);
        quads.push(poly.vertices.map(|v| {
            let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
            // The armour glint scrolls the layer's model UV in the shader;
            // the item path already passed its transformed UV.
            let glint_uv = if matches!(overlay, ARMOR_GLINT | crate::gfx::ARMOR_GLINT_ONLY_OVERLAY)
            {
                [v[3], v[4]]
            } else {
                [0.0, 0.0]
            };
            Vertex {
                normal,
                pos: pos.to_array(),
                uv: [
                    uv[0] + (uv[2] - uv[0]) * scroll_u(v[3], scroll),
                    uv[1] + (uv[3] - uv[1]) * v[4],
                ],
                layer: overlay,
                color,
                light,
                glint_uv,
            }
        }));
    }
}
/// A part's matrix under `base` for its pose (`ModelPart.translateAndRotate`
/// and its scale), as `draw` and the GPU path (`super::skin`) both use it.
pub(super) fn posed(base: glam::Mat4, p: [f32; 9]) -> glam::Mat4 {
    base * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]))
}

/// Keeps the original multiplication order when the local rotation was reused.
pub(super) fn posed_rotated(base: glam::Mat4, p: [f32; 9], rotation: glam::Mat4) -> glam::Mat4 {
    base * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * rotation
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]))
}
/// `draw_scrolled` through the GPU path when the mob takes it (`skin`), on
/// the CPU otherwise.
#[allow(clippy::too_many_arguments)]
pub fn put(
    skin: &mut Option<&mut super::skin::Skin>,
    part: &std::sync::Arc<Part>,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let paint = Paint {
        uv,
        scroll,
        color,
        light,
        overlay,
    };
    // The skin buffer draws ordinary entity materials. Standalone foil needs the additive pass.
    if overlay != crate::gfx::ARMOR_GLINT_ONLY_OVERLAY
        && let Some(skin) = skin
        && skin.draw(part, base, poses, paint)
    {
        return;
    }
    draw_scrolled(part, base, poses, uv, scroll, color, light, overlay, out);
}
/// Render a mob with its variant model, texture and layers.
pub fn render(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    out: &mut Vec<Vertex>,
) {
    render_with(m, alpha, camera, atlas, light, data, &[], out);
}
/// `render` plus extra layers posed like the body (`HumanoidArmorLayer`:
/// the armour model's parts copy the body's): (model, texture rect, colour,
/// overlay: 0 or `ARMOR_GLINT`, as `EquipmentLayerRenderer` never tints them).
#[allow(clippy::too_many_arguments)]
pub fn render_with(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    extra: &[WornLayer<'_>],
    out: &mut Vec<Vertex>,
) {
    render_with_frame(
        m,
        alpha,
        camera,
        atlas,
        light,
        data,
        extra,
        &mut FrameDraw::default(),
        out,
    );
}
/// Inputs and captured output for one render of a living model.
#[derive(Default)]
pub struct FrameDraw<'a> {
    prepared: Option<(FrameBody<'a>, FrameModelPose)>,
    pre_vertices_start: Option<u32>,
    /// Whether the post hooks or mod layers need the final model.
    pub capture_model: bool,
    model: Option<FrameModelPose>,
}

impl<'a> FrameDraw<'a> {
    /// Track the final pose only if a layer or Post hook will read it.
    pub fn new(capture_model: bool) -> Self {
        Self {
            capture_model,
            ..Self::default()
        }
    }

    /// Select this mob's body for Pre, lending its tree and unanimated bind pose. Call only for
    /// draws with a Pre listener, and pass this same draw state on to the renderer. Unbaked
    /// models (e.g. standalone probes without client resources) have no view to lend.
    pub fn prepare(
        &mut self,
        mob: &'a Mob,
        data: &ClientMobData,
        models: &'a ClientMobModels,
    ) -> Option<&mut dyn mod_api::FrameModelEdit> {
        self.prepare_with_source(mob, &ClientMobRender { models, data }, models)
    }

    pub fn prepare_with_source(
        &mut self,
        mob: &'a Mob,
        data: &dyn RenderDataSource,
        models: &'a ClientMobModels,
    ) -> Option<&mut dyn mod_api::FrameModelEdit> {
        let body = FrameBody::select(mob, data.mob_data(), models, data.variants())?;
        let layout = body.layout(data.mob_data())?.clone();
        let mut poses = Poses::pooled();
        poses.reset(body.model());
        poses.prepare_frame_visibility(layout.as_ref());
        let (_, model) = self.prepared.insert((
            body,
            FrameModelPose {
                layout,
                poses,
                bind: true,
            },
        ));
        Some(model)
    }

    /// Keep this draw's Pre geometry ahead of its posed body and layers.
    pub fn mark_pre_vertices(&mut self, start: usize) {
        self.pre_vertices_start = Some(start as u32);
    }

    /// Borrow the captured body model for layer and Post callbacks.
    pub fn model(&self) -> Option<&dyn mod_api::FrameModel> {
        self.model
            .as_ref()
            .map(|model| model as &dyn mod_api::FrameModel)
    }
}

/// [`render_with`], applying Pre edits to the body after Skin Customization and to the
/// layers on its own model, but not to armour or other layers. `frame.capture_model`
/// keeps the pose the draw used for Post hooks and mod layers.
#[allow(clippy::too_many_arguments)]
pub fn render_with_frame(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    extra: &[WornLayer<'_>],
    frame: &mut FrameDraw<'_>,
    out: &mut Vec<Vertex>,
) {
    let paint = Some(body_color(m, alpha));
    // The invisibility effect hides the model (a drinking wandering trader);
    // an illusioner shows its four mirror images instead.
    if m.is_invisible() {
        if m.species() == Species::Illusioner {
            let age = m.age as f32 + alpha;
            let bases = super::illusioner::offsets(m, alpha)
                .into_iter()
                .enumerate()
                .map(|(i, off)| {
                    let w = super::illusioner::wobble(i, age);
                    let shifted =
                        camera - glam::DVec3::new(off[0] + w[0], off[1] + w[1], off[2] + w[2]);
                    base_transform(m, alpha, shifted)
                });
            render_visible(
                m,
                alpha,
                Some(camera),
                bases,
                atlas,
                light,
                data,
                extra,
                paint,
                frame,
                out,
                None,
            );
        } else {
            frame.model = None;
            frame.prepared = None;
        }
        return;
    }
    render_visible(
        m,
        alpha,
        Some(camera),
        [base_transform(m, alpha, camera)],
        atlas,
        light,
        data,
        extra,
        paint,
        frame,
        out,
        None,
    );
}
/// `render` with the model's ordinary draws posed on the GPU when the mob
/// allows it (`super::skin`): the same picture, no vertices on the CPU.
/// `frame` is used as in [`render_with_frame`].
#[allow(clippy::too_many_arguments)]
pub fn render_skinned(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    frame: &mut FrameDraw<'_>,
    out: &mut Vec<Vertex>,
    skin: &mut super::skin::Skin,
) {
    if m.is_invisible() {
        render_with_frame(m, alpha, camera, atlas, light, data, &[], frame, out);
        return;
    }
    render_visible(
        m,
        alpha,
        Some(camera),
        [base_transform(m, alpha, camera)],
        atlas,
        light,
        data,
        &[],
        Some(body_color(m, alpha)),
        frame,
        out,
        Some(skin),
    );
}
/// `CapeLayer.submit` with `PlayerCapeModel`: the player model's body posed
/// as the player's, its cape turned by `ModelPart.rotateBy` with the cape's
/// flap and leans. `base` is the player's root transform (moved out over a
/// chestplate); `uv` the cape texture.
#[allow(clippy::too_many_arguments)]
pub fn cape(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    uv: [f32; 4],
    light: u32,
    data: &dyn RenderDataSource,
    out: &mut Vec<Vertex>,
) {
    let tables = data.mob_tables();
    let Some(model) = tables.models.layer(&m.kind.id, "player#cape") else {
        return;
    };
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let mut poses = Poses::new(model);
    anim::setup(m, &s, &mut poses, &tables.data.animations);
    let a = f64::from(alpha);
    let at = [
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ];
    let body = anim::body_rot(m, alpha);
    // LivingEntityRenderState.fallFlyingScale.
    let ticks = m.extra.fall_fly_ticks as f32 + alpha;
    let gliding = (ticks * ticks / 100.0).clamp(0.0, 1.0);
    let [flap, lean, lean2] = m.extra.avatar.cape(alpha, at, body, gliding);
    let pose = poses.get("cape");
    let by = glam::Quat::from_rotation_y(-PI)
        * glam::Quat::from_rotation_x((6.0 + lean / 2.0 + flap).to_radians())
        * glam::Quat::from_rotation_z((lean2 / 2.0).to_radians())
        * glam::Quat::from_rotation_y((180.0 - lean2 / 2.0).to_radians());
    let now = glam::Quat::from_euler(glam::EulerRot::ZYX, pose[5], pose[4], pose[3]);
    let (z, y, x) = (now * by).to_euler(glam::EulerRot::ZYX);
    (pose[3], pose[4], pose[5]) = (x, y, z);
    // RenderTypes.entitySolid with OverlayTexture.NO_OVERLAY.
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, 0, out);
}
/// `Deadmau5EarsLayer.submit` with `PlayerEarsModel`: the player model's
/// head carrying two 6x6x1 ears (the model's other cubes are cleared),
/// drawn with the player's skin as `RenderTypes.entitySolid`. The caller has
/// checked `showExtraEars`; an invisible avatar draws no ears.
#[allow(clippy::too_many_arguments)]
pub fn ears(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    uv: [f32; 4],
    light: u32,
    data: &dyn RenderDataSource,
    out: &mut Vec<Vertex>,
) {
    if m.is_invisible() {
        return;
    }
    let tables = data.mob_tables();
    let Some(model) = tables.models.layer(&m.kind.id, "player#ears") else {
        return;
    };
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let mut poses = Poses::new(model);
    anim::setup(m, &s, &mut poses, &tables.data.animations);
    // LivingEntityRenderer.getOverlayCoords(state, 0.0F).
    draw(
        model,
        base,
        &poses,
        uv,
        0xFFFF_FFFF,
        light,
        body_overlay(m, alpha),
        out,
    );
}
/// `PlayerModel.bodyParts`: what `getRandomBodyPart` picks from.
const BODY_PARTS: [&str; 6] = [
    "head",
    "body",
    "left_arm",
    "right_arm",
    "left_leg",
    "right_leg",
];

/// The cubes of a part as `ModelPart.cubes` has them: min and max corners in
/// model units. The part keeps six faces per cube, in cube order.
fn cube_bounds(part: &Part) -> Vec<[f32; 6]> {
    let size = if part.polygons.len().is_multiple_of(6) {
        6
    } else {
        part.polygons.len().max(1)
    };
    part.polygons
        .chunks(size)
        .map(|faces| {
            let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
            for v in faces.iter().flat_map(|f| f.vertices) {
                for k in 0..3 {
                    b[k] = b[k].min(v[k]);
                    b[3 + k] = b[3 + k].max(v[k]);
                }
            }
            b
        })
        .collect()
}

/// `StuckInBodyLayer.snapToFace`.
fn snap_to_face(value: f32) -> f32 {
    if value > 0.5 { 1.0 } else { 0.5 }
}

/// `StuckInBodyLayer.submit` (with `submitStuckItem`) for a player: `count`
/// copies of `item` (`ArrowLayer`'s arrow model, `BeeStingerLayer`'s stinger)
/// stuck in random cubes of the posed body, placed from a random source
/// seeded by the entity id `id`. `on_surface` is `PlacementStyle.ON_SURFACE`.
#[allow(clippy::too_many_arguments)]
pub fn stuck_in_body(
    m: &Mob,
    id: i32,
    alpha: f32,
    camera: glam::DVec3,
    data: &dyn RenderDataSource,
    count: i32,
    on_surface: bool,
    item: &Part,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    use sunrise_core::random::{LegacyRandom, RandomSource};
    if count <= 0 {
        return;
    }
    let tables = data.mob_tables();
    let Some(visuals) = tables.models.get(&m.kind.id) else {
        return;
    };
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let (model_key, _) = body_with_variants(m, visuals, data.variants());
    let model = visuals.layer(&model_key).unwrap_or(&visuals.model);
    let mut poses = Poses::new(model);
    anim::setup(m, &s, &mut poses, &tables.data.animations);
    let mut random = LegacyRandom::new(i64::from(id));
    for _ in 0..count {
        let name = BODY_PARTS[random.next_int_bound(BODY_PARTS.len() as i32).unwrap_or(0) as usize];
        let Some(part) = find_part(model, name) else {
            return;
        };
        let cubes = cube_bounds(part);
        let Ok(index) = random.next_int_bound(cubes.len() as i32) else {
            return;
        };
        let cube = cubes[index as usize];
        let Some(mut t) = part_matrix(model, base, &poses, name, 0.0) else {
            return;
        };
        let mut mid = [
            random.next_float(),
            random.next_float(),
            random.next_float(),
        ];
        if on_surface {
            let plane = random.next_int_bound(3).unwrap_or(0) as usize;
            if plane < 3 {
                mid[plane] = snap_to_face(mid[plane]);
            }
        }
        let lerp = |k: usize| cube[k] + mid[k] * (cube[3 + k] - cube[k]);
        t *= glam::Mat4::from_translation(glam::Vec3::new(lerp(0), lerp(1), lerp(2)) / 16.0);
        let (dx, dy, dz) = (
            -(mid[0] * 2.0 - 1.0),
            -(mid[1] * 2.0 - 1.0),
            -(mid[2] * 2.0 - 1.0),
        );
        let dxz = f64::from(dx * dx + dz * dz).sqrt() as f32;
        let degrees = |y: f32, x: f32| {
            (sunrise_core::jmath::atan2(f64::from(y), f64::from(x))
                * f64::from(sunrise_core::geometry::trig::RAD_TO_DEG)) as f32
        };
        let y_rot = degrees(dx, dz);
        let x_rot = degrees(dy, dxz);
        t *= glam::Mat4::from_rotation_y((y_rot - 90.0).to_radians())
            * glam::Mat4::from_rotation_z(x_rot.to_radians());
        super::render_model(item, t, &|p| p.pose, uv, light, 0, out);
    }
}
/// `ParrotOnShoulderLayer.submitOnShoulder`: a parrot of `variant` on the
/// avatar's left or right shoulder, in `ParrotModel.Pose.ON_SHOULDER` with
/// the avatar's walk animation, age and head turn. `base` is the avatar's
/// root transform; `parrot` the parrot's entity type (its model).
#[allow(clippy::too_many_arguments)]
pub fn shoulder_parrot(
    avatar: &Mob,
    parrot: &ClientMobVisuals,
    variant: i32,
    left: bool,
    alpha: f32,
    base: glam::Mat4,
    atlas: Atlas,
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let Some(uv) = atlas(&shoulder_parrot_texture(variant)) else {
        return;
    };
    let model = parrot.layer("parrot#main").unwrap_or(&parrot.model);
    let s = anim::State {
        baby: false,
        ..anim::state(avatar, alpha)
    };
    let mut poses = Poses::new(model);
    // ParrotRenderState.flapAngle stays 0.
    super::body_anim::parrot(
        &mut poses,
        &s,
        super::body_anim::ParrotPose::OnShoulder,
        0.0,
    );
    let y = if avatar.extra.crouching { -1.3 } else { -1.5 };
    let x = if left { 0.4 } else { -0.4 };
    let base = base * glam::Mat4::from_translation(glam::Vec3::new(x, y, 0.0));
    // ParrotModel's cutout render type with OverlayTexture.NO_OVERLAY.
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, 0, out);
}
/// `ParrotRenderer.getVariantTexture` for `Parrot.Variant.byId` (an id out
/// of range is clamped).
fn shoulder_parrot_texture(variant: i32) -> String {
    const NAMES: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "grey"];
    let name = usize::try_from(variant)
        .ok()
        .and_then(|i| NAMES.get(i))
        .unwrap_or(if variant < 0 { &NAMES[0] } else { &NAMES[4] });
    format!("entity/parrot/parrot_{name}")
}
/// `render_with` without the body, for an avatar the viewer can't see:
/// `LivingEntityRenderer.submit` gets no render type for an invisible body
/// (`isBodyVisible`, `isInvisibleTo`), but its layers still draw
/// (`HumanoidArmorLayer`: the armour in `extra`). The captured model, if requested,
/// includes Pre's edits even when the body wasn't submitted.
#[allow(clippy::too_many_arguments)]
pub fn render_layers(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    extra: &[WornLayer<'_>],
    frame: &mut FrameDraw<'_>,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    render_visible(
        m,
        alpha,
        Some(camera),
        [base],
        atlas,
        light,
        data,
        extra,
        None,
        frame,
        out,
        None,
    );
}
/// A mob drawn under `origin` instead of at its position (the spinning mob
/// of `SpawnerRenderer.submitEntityInSpawner`).
pub fn render_posed(
    m: &Mob,
    alpha: f32,
    origin: glam::Mat4,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform_at(m, alpha, origin);
    render_visible(
        m,
        alpha,
        None,
        [base],
        atlas,
        light,
        data,
        &[],
        Some(body_color(m, alpha)),
        &mut FrameDraw::default(),
        out,
        None,
    );
}
/// The custom model's key of a body model: `main` is the wide player model, `kind.model`.
fn cem_key(model_key: &str) -> &str {
    if model_key == "main" {
        "player#main"
    } else {
        model_key
    }
}
/// Every texture [`render_visible`] may look up for `m` now (keep the two in step: a texture
/// drawn but not listed here never gets a random pick): the body's and each layer's, a
/// custom model's own texture beside them (`cem_texture` falls back to the vanilla one when
/// the atlas lacks it, and only the layer a `.jem` was made for takes it, `paints_with_jem`),
/// and the warden's glowing layers (whether they glow this frame or not). Random textures
/// choose a variant for each.
pub fn drawn_textures(m: &Mob, data: &dyn RenderDataSource) -> Vec<String> {
    fn push(names: &mut Vec<String>, custom: Option<&super::cem::CemLayer>, vanilla: String) {
        if let Some(texture) = custom.and_then(|c| c.texture_for(&vanilla)) {
            names.push(texture.to_owned());
        }
        names.push(vanilla);
    }
    let tables = data.mob_tables();
    let Some(visuals) = tables.models.get(&m.kind.id) else {
        return Vec::new();
    };
    let equipment = data.equipment_inventory(m);
    let cem = tables
        .data
        .cem
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (model_key, texture) = body_with_variants(m, visuals, data.variants());
    let cem_key = cem_key(&model_key);
    let mut names = Vec::new();
    push(&mut names, cem.layer(cem_key), texture.into_owned());
    let mut all_layers = layers(m, m.age as f32);
    all_layers.extend(equipment);
    if m.species() == Species::Warden {
        for (model, texture, _) in super::warden::emissive_layers(m, 0.0) {
            all_layers.push(Layer {
                equipment_overlay: None,
                equipment_hidden: false,
                equipment_model: None,
                equipment_uv: None,
                model: model.into(),
                texture: texture.into(),
                color: 0,
                full_bright: true,
                scroll: 0.0,
            });
        }
    }
    let body_keys = [&*model_key, cem_key];
    for (i, layer) in all_layers.iter().enumerate() {
        let custom = cem
            .layer(&layer.model)
            .filter(|_| paints_with_jem(&all_layers, i, body_keys));
        push(&mut names, custom, layer.texture.to_string());
    }
    names
}
/// The `.jem`'s replacement for `vanilla`, only if the atlas has it (a texture the
/// atlas lacks would hide the model).
fn cem_texture<'a>(
    custom: Option<&'a super::cem::CemLayer>,
    vanilla: &str,
    atlas: Atlas<'_>,
) -> Option<&'a str> {
    custom
        .and_then(|c| c.texture_for(vanilla))
        .filter(|t| atlas(t).is_some())
}
/// Whether `layers[i]` is the layer its model's `.jem` was made for, so the `.jem`'s texture
/// paints it: the first drawn with that model (`sheep#wool`, a wolf's armour). Later layers
/// on the same model are overlays (the armour's cracks), like those on the body's model
/// (`body`: eyes, collars, a villager's type and profession), and keep their own textures
/// over the new geometry.
fn paints_with_jem(layers: &[Layer], i: usize, body: [&str; 2]) -> bool {
    let key = &*layers[i].model;
    !body.contains(&key) && layers[..i].iter().all(|l| l.model != key)
}
/// The model a mob's body draws with — the packs' custom one when it has one — posed the way
/// the body draws it (the vanilla animation, then the pack's own), with the `attachments`
/// anchors its bones declare. Everything that anchors to a mob — an item in its hand, a
/// block on its head — walks the same tree the player saw.
fn posed_body(
    m: &Mob,
    alpha: f32,
    data: &dyn RenderDataSource,
) -> Option<(std::sync::Arc<Part>, Poses, Vec<(String, String, [f32; 3])>)> {
    let tables = data.mob_tables();
    let visuals = tables.models.get(&m.kind.id)?;
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let (model_key, _) = body_with_variants(m, visuals, data.variants());
    let cem = tables
        .data
        .cem
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let custom = cem.layer(cem_key(&model_key));
    let model = match custom {
        Some(c) => c.model.clone(),
        None => visuals.layer(&model_key).unwrap_or(&visuals.model).clone(),
    };
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &tables.data.animations);
    if let Some(c) = custom
        && let Some(animation) = &c.animation
    {
        animation.run(m, &s, &mut poses);
    }
    let anchors = custom.map_or(Vec::new(), |c| c.anchors.clone());
    Some((model, poses, anchors))
}

/// Stock item-layer attachments for `m`'s vanilla body model: `(custom_head, main_hand, off_hand)`.
pub fn stock_item_layers(m: &Mob, data: &dyn RenderDataSource) -> (bool, bool, bool) {
    let tables = data.mob_tables();
    let Some(visuals) = tables.models.get(&m.kind.id) else {
        return (false, false, false);
    };
    let (key, _) = body_with_variants(m, visuals, data.variants());
    let model = visuals.layer(&key).unwrap_or(&visuals.model);
    let has = |part| find_part(model, part).is_some();
    let mut main_hand = false;
    let mut off_hand = false;
    for main_left in [false, true] {
        for off in [false, true] {
            let left = main_left != off;
            let supported = match held_item_anchor(m) {
                HeldItemAnchor::Panda | HeldItemAnchor::Fox | HeldItemAnchor::Dolphin => !left,
                HeldItemAnchor::Witch => left == main_left && (has("arms") || has("nose")),
                HeldItemAnchor::CrossedArms => left == main_left && has("arms"),
                HeldItemAnchor::Hand if m.species() == Species::Allay => {
                    has("body") && has("right_arm")
                }
                HeldItemAnchor::Hand => has(hand_arm(left)),
            };
            if supported {
                if off {
                    off_hand = true;
                } else {
                    main_hand = true;
                }
            }
        }
    }
    (has_head_item_layer(m) && has("head"), main_hand, off_hand)
}

/// Draws `extra` (player armour layers) with the same body pose and transform as
/// [`render_with_frame`], for a mod layer calling `StockLayerDraw::draw_stock_layer`.
pub fn render_extra_layers(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    light: u32,
    data: &dyn RenderDataSource,
    extra: &[WornLayer<'_>],
    out: &mut Vec<Vertex>,
) {
    if extra.is_empty() || !layers_shown(m) {
        return;
    }
    let tables = data.mob_tables();
    let Some(body) = FrameBody::select(m, tables.data, tables.models, data.variants()) else {
        return;
    };
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let mut poses = Poses::pooled();
    poses.reset(body.model());
    anim::setup(m, &s, &mut poses, &data.mob_data().animations);
    if let Some(animation) = body
        .custom
        .as_deref()
        .and_then(|layer| layer.animation.as_ref())
    {
        animation.run(m, &s, &mut poses);
    }
    model_posed(
        m,
        alpha,
        Some(camera),
        data,
        body.keys()[0],
        body.layout(data.mob_data()).map(|l| &**l),
        &mut poses,
    );
    poses.apply_frame_visibility(false);
    for (part, uv, color, layer_overlay, glows) in extra {
        let light = if *glows { FULL_BRIGHT } else { light };
        part.submit(
            &mut None,
            base,
            &poses,
            Paint {
                uv: *uv,
                scroll: 0.0,
                color: *color,
                light,
                overlay: *layer_overlay,
            },
            out,
        );
    }
}

/// Where and how lit a mob is drawn this frame.
#[derive(Clone, Copy)]
pub struct LayerFrame {
    /// The partial tick.
    pub alpha: f32,
    /// The camera's position.
    pub camera: glam::DVec3,
    /// The packed light the layers are drawn with.
    pub light: u32,
}

/// Draws `equipment` (non-player mob armour layers) with the same body pose and transform as
/// [`render_with_frame`], for a mod layer calling `StockLayerDraw::draw_stock_layer`.
pub fn render_equipment_layers(
    m: &Mob,
    frame: LayerFrame,
    atlas: Atlas,
    data: &dyn RenderDataSource,
    equipment: Vec<Layer>,
    out: &mut Vec<Vertex>,
) {
    let LayerFrame {
        alpha,
        camera,
        light,
    } = frame;
    if equipment.is_empty() || !layers_shown(m) {
        return;
    }
    let tables = data.mob_tables();
    let Some(body) = FrameBody::select(m, tables.data, tables.models, data.variants()) else {
        return;
    };
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let cem = &body.cem;
    let body_model = body.model();
    let random = data
        .mob_data()
        .random_textures
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let blinks = data
        .mob_data()
        .blinks
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sprite = |texture: &str| {
        let variant = (!random.is_empty())
            .then(|| super::random_textures::texture(m, &random, texture))
            .flatten()
            .filter(|v| atlas(v).is_some());
        let frame = (!blinks.is_empty())
            .then(|| blinks.get(variant.unwrap_or(texture)))
            .flatten()
            .and_then(|b| {
                if super::villager::sleeping(m) {
                    Some(b.shut.as_str())
                } else {
                    b.frame(s.age as u32, m.id.wrapping_mul(0x9E37_79B1))
                }
            })
            .filter(|f| atlas(f).is_some());
        let name = frame.or(variant)?;
        Some((name, atlas(name)?))
    };
    let emissive = data
        .mob_data()
        .emissive
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let emissive_uv = |drawn: &str| {
        if emissive.is_empty() {
            return None;
        }
        atlas(emissive.overlay(drawn)?)
    };
    let mut poses = Poses::pooled();
    poses.reset(body_model);
    anim::setup(m, &s, &mut poses, &data.mob_data().animations);
    if let Some(animation) = body
        .custom
        .as_deref()
        .and_then(|layer| layer.animation.as_ref())
    {
        animation.run(m, &s, &mut poses);
    }
    model_posed(
        m,
        alpha,
        Some(camera),
        data,
        body.keys()[0],
        body.layout(data.mob_data()).map(|l| &**l),
        &mut poses,
    );
    let mut all_layers = layers(m, s.age);
    let first = all_layers.len();
    all_layers.extend(equipment);
    let overlay_body = u32::from(red_overlay(m));
    let mut layer_poses = Poses::pooled();
    let body_keys = body.keys();
    for (i, layer) in all_layers.iter().enumerate().skip(first) {
        if layer.equipment_hidden {
            continue;
        }
        let custom = cem.layer(&layer.model);
        let part = if let Some((model, slot)) = &layer.equipment_model {
            WornModel::Override(model, *slot)
        } else {
            let Some(part) = custom
                .map(|c| &c.model)
                .or_else(|| body.visuals.layer(&layer.model))
            else {
                continue;
            };
            WornModel::Vanilla(part)
        };
        let Some((drawn, uv)) = (if let Some(uv) = layer.equipment_uv {
            Some((layer.texture.as_ref(), uv))
        } else {
            let texture = cem_texture(
                custom.filter(|_| paints_with_jem(&all_layers, i, body_keys)),
                &layer.texture,
                atlas,
            )
            .unwrap_or(&layer.texture);
            sprite(texture).or_else(|| Some((texture, atlas(texture)?)))
        }) else {
            continue;
        };
        let on_parent = custom.is_none()
            && body.custom.is_none()
            && matches!(part, WornModel::Vanilla(part) if std::sync::Arc::ptr_eq(part, body_model));
        let uses_body_pose = matches!(part, WornModel::Override(..)) || on_parent;
        let draw_poses = match part {
            WornModel::Vanilla(part) if !uses_body_pose => {
                layer_poses.reset(part);
                anim::setup(m, &s, &mut layer_poses, &data.mob_data().animations);
                if let Some(animation) = custom.and_then(|c| c.animation.as_ref()) {
                    animation.run(m, &s, &mut layer_poses);
                }
                model_posed(
                    m,
                    alpha,
                    Some(camera),
                    data,
                    &layer.model,
                    custom
                        .map(|c| &*c.frame_model)
                        .or_else(|| data.mob_data().frame_models.get(part).map(|l| &**l)),
                    &mut layer_poses,
                );
                &mut layer_poses
            }
            _ => {
                poses.apply_frame_visibility(on_parent);
                &mut poses
            }
        };
        let layer_light = if layer.full_bright {
            FULL_BRIGHT
        } else {
            light
        };
        let overlay = layer.equipment_overlay.unwrap_or(overlay_body);
        let paint = Paint {
            uv,
            scroll: layer.scroll,
            color: layer.color,
            light: layer_light,
            overlay,
        };
        part.submit(&mut None, base, draw_poses, paint, out);
        if overlay != crate::gfx::ARMOR_GLINT_ONLY_OVERLAY
            && let Some(glow) = emissive_uv(drawn)
        {
            part.submit(
                &mut None,
                base,
                draw_poses,
                Paint {
                    uv: glow,
                    light: FULL_BRIGHT,
                    overlay: 0,
                    ..paint
                },
                out,
            );
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn render_visible(
    m: &Mob,
    alpha: f32,
    camera: Option<glam::DVec3>,
    bases: impl IntoIterator<Item = glam::Mat4>,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    extra: &[WornLayer<'_>],
    // `submit`'s `tintedColor` to draw the body in, or `None` for its layers only.
    paint: Option<u32>,
    frame: &mut FrameDraw<'_>,
    out: &mut Vec<Vertex>,
    mut skin: Option<&mut super::skin::Skin>,
) {
    frame.model = None;
    let tables = data.mob_tables();
    let (body, mut poses, layout) = match frame.prepared.take() {
        Some((body, prepared)) => (body, prepared.poses, Some(prepared.layout)),
        None => {
            let Some(body) = FrameBody::select(m, tables.data, tables.models, data.variants())
            else {
                return;
            };
            let mut poses = Poses::pooled();
            poses.reset(body.model());
            (body, poses, None)
        }
    };
    let texture = body.texture(atlas);
    let pre_vertices_start = frame.pre_vertices_start.take();
    for (i, base) in bases.into_iter().enumerate() {
        if i != 0 {
            // Each illusioner mirror runs vanilla setup and CEM from bind pose, with the same
            // overrides from the entity's single Pre call.
            poses.reset_animation(body.model());
        }
        poses.apply_frame_visibility(false);
        render_selected(
            m,
            alpha,
            camera,
            base,
            atlas,
            light,
            data,
            tables.data,
            extra,
            paint,
            &body,
            texture,
            &mut poses,
            pre_vertices_start.unwrap_or(out.len() as u32),
            out,
            skin.as_deref_mut(),
        );
    }
    if frame.capture_model
        && let Some(layout) = layout.or_else(|| body.layout(tables.data).cloned())
    {
        // Leave the last mirror's actual draw pose, not a temporary independent-layer pose.
        poses.apply_frame_visibility(true);
        frame.model = Some(FrameModelPose {
            layout,
            poses,
            bind: false,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn render_selected(
    m: &Mob,
    alpha: f32,
    camera: Option<glam::DVec3>,
    base: glam::Mat4,
    atlas: Atlas,
    light: u32,
    data: &dyn RenderDataSource,
    mob_data: &ClientMobData,
    extra: &[WornLayer<'_>],
    paint: Option<u32>,
    body: &FrameBody<'_>,
    texture: &str,
    poses: &mut Poses,
    mut cpu_cut: u32,
    out: &mut Vec<Vertex>,
    mut skin: Option<&mut super::skin::Skin>,
) {
    // The armour is a layer: none for a spectator.
    let equipment = data.equipment_layers(m);
    let extra = if layers_shown(m) { extra } else { &[] };
    let overlay = u32::from(red_overlay(m));
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let cem = &body.cem;
    let body_model = body.model();
    // OptiFine random textures and ETF blinking: the sprite a texture draws with instead, when
    // the atlas has it: the variant the mob wears, then its blink frame while it blinks (or
    // sleeps). The texture keeps its name for everything else.
    let random = mob_data
        .random_textures
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let blinks = mob_data
        .blinks
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sprite = |texture: &str| {
        let variant = (!random.is_empty())
            .then(|| super::random_textures::texture(m, &random, texture))
            .flatten()
            .filter(|v| atlas(v).is_some());
        let frame = (!blinks.is_empty())
            .then(|| blinks.get(variant.unwrap_or(texture)))
            .flatten()
            .and_then(|b| {
                if super::villager::sleeping(m) {
                    Some(b.shut.as_str())
                } else {
                    b.frame(s.age as u32, m.id.wrapping_mul(0x9E37_79B1))
                }
            })
            .filter(|f| atlas(f).is_some());
        let name = frame.or(variant)?;
        Some((name, atlas(name)?))
    };
    // OptiFine emissive overlays: the drawn sprite's `_e`, at full brightness with the model's
    // shading, as OptiFine draws them.
    let emissive = mob_data
        .emissive
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let emissive_uv = |drawn: &str| {
        if emissive.is_empty() {
            return None;
        }
        atlas(emissive.overlay(drawn)?)
    };
    anim::setup(m, &s, poses, &mob_data.animations);
    if let Some(animation) = body
        .custom
        .as_deref()
        .and_then(|layer| layer.animation.as_ref())
    {
        animation.run(m, &s, poses);
    }
    model_posed(
        m,
        alpha,
        camera,
        data,
        body.keys()[0],
        body.layout(data.mob_data()).map(|l| &**l),
        poses,
    );
    let mut all_layers = layers(m, s.age);
    all_layers.extend(equipment);
    // WardenRenderer: the translucent emissive layers fade with the animations.
    if m.species() == Species::Warden {
        for (model, texture, a) in super::warden::emissive_layers(m, alpha) {
            if a > 0.0 {
                all_layers.push(Layer {
                    equipment_overlay: None,
                    equipment_hidden: false,
                    equipment_model: None,
                    equipment_uv: None,
                    model: model.into(),
                    texture: texture.into(),
                    color: ((a.clamp(0.0, 1.0) * 255.0) as u32) << 24 | 0x00FF_FFFF,
                    full_bright: true,
                    scroll: 0.0,
                });
            }
        }
    }
    // Only ordinary models can share the posed stream: no custom models, translucent
    // body or energy swirl (which the CPU path changes after drawing). With improved
    // transparency off, the layers' tags are the only difference from an ordinary draw.
    if skin.as_deref().is_some_and(|skin| {
        body.custom.is_some()
            || !extra.is_empty()
            || m.species() == Species::SulfurCube
            || translucent_body(m)
            || energy_swirl(m, s.age).is_some()
            || all_layers.iter().any(|layer| {
                !layer.equipment_hidden
                    && (layer.equipment_overlay == Some(crate::gfx::ARMOR_GLINT_ONLY_OVERLAY)
                        || !(skin.models.tagged && (eyes_layer(m, layer) || !layer.full_bright)
                            || !layer.full_bright && !translucent_layer(m, layer))
                        || cem.layer(&layer.model).is_some()
                        || layer.equipment_model.is_some())
            })
    }) {
        skin = None;
    }
    let draw_body = |poses: &mut Poses, out: &mut Vec<Vertex>, skin: &mut Option<_>| {
        poses.apply_frame_visibility(true);
        // A player or mannequin whose skin has no atlas sprite (an evicted or
        // never stored download) draws with its default skin, not invisibly.
        let drawn = sprite(texture).or_else(|| Some((texture, atlas(texture)?)));
        let uv = drawn.map(|(_, uv)| uv).or_else(|| {
            if avatar(m) {
                atlas(&body.visuals.texture)
            } else if matches!(
                m.species(),
                Species::Wolf
                    | Species::Cat
                    | Species::Pig
                    | Species::Cow
                    | Species::Chicken
                    | Species::Frog
                    | Species::ZombieNautilus
            ) {
                atlas(crate::hud::MISSING_SPRITE)
            } else {
                None
            }
        });
        if let (Some(tint), Some(uv)) = (paint, uv) {
            let start = out.len();
            put(
                skin,
                body_model,
                base,
                poses,
                uv,
                0.0,
                tint,
                light,
                body_overlay(m, alpha),
                out,
            );
            if let Some(glow) = drawn.and_then(|(name, _)| emissive_uv(name)) {
                put(
                    skin,
                    body_model,
                    base,
                    poses,
                    glow,
                    0.0,
                    tint,
                    FULL_BRIGHT,
                    0,
                    out,
                );
            }
            // A body below full alpha (`forceTransparent`'s 654311423) blends too.
            if translucent_body(m) || tint >> 24 != 0xFF {
                mark_oit(&mut out[start..]);
            }
        }
    };
    // The translucent shell writes depth: draw the opaque core before it.
    let shell_last = m.species() == Species::SulfurCube;
    if !shell_last {
        let start = out.len() as u32;
        let first_job = skin
            .as_deref()
            .filter(|_| cpu_cut < start)
            .map(|skin| skin.frame.jobs.len());
        draw_body(poses, out, &mut skin);
        // NeoForge's RenderLivingEvent.Pre submits before the body. If the body
        // stays on the CPU, that prefix travels with it before the next GPU layer.
        if let (Some(skin), Some(first)) = (skin.as_deref_mut(), first_job)
            && let Some(job) = skin.frame.jobs.get(first)
        {
            skin.frame.insert_cpu_before(job.dst, cpu_cut..start);
            cpu_cut = start;
        }
    }
    // The worn armour's layers (and their emissive overlays, full bright).
    if !extra.is_empty() {
        poses.apply_frame_visibility(false);
        for (part, uv, color, layer_overlay, glows) in extra {
            let light = if *glows { FULL_BRIGHT } else { light };
            part.submit(
                &mut None,
                base,
                poses,
                Paint {
                    uv: *uv,
                    scroll: 0.0,
                    color: *color,
                    light,
                    overlay: *layer_overlay,
                },
                out,
            );
        }
    }
    // Refill one pooled pose map for layers that run their own setupAnim.
    let mut layer_poses = Poses::pooled();
    let body_keys = body.keys();
    for (i, layer) in all_layers.iter().enumerate() {
        if layer.equipment_hidden {
            continue;
        }
        let custom = cem.layer(&layer.model);
        // `layer.texture` stays the vanilla name (the villager hats below look for its
        // `/type/` folder).
        let part = if let Some((model, slot)) = &layer.equipment_model {
            WornModel::Override(model, *slot)
        } else {
            let Some(part) = custom
                .map(|c| &c.model)
                .or_else(|| body.visuals.layer(&layer.model))
            else {
                continue;
            };
            WornModel::Vanilla(part)
        };
        let Some((drawn, uv)) = (if let Some(uv) = layer.equipment_uv {
            Some((layer.texture.as_ref(), uv))
        } else {
            let texture = cem_texture(
                custom.filter(|_| paints_with_jem(&all_layers, i, body_keys)),
                &layer.texture,
                atlas,
            )
            .unwrap_or(&layer.texture);
            sprite(texture).or_else(|| Some((texture, atlas(texture)?)))
        }) else {
            continue;
        };
        // VillagerProfessionLayer: the type layer under a profession's full hat draws on
        // `noHatModel`, a model of its own, the others on the parent model.
        let type_hat_shown = if matches!(m.species(), Species::Villager | Species::ZombieVillager)
            && let Some((folder, _variant)) = layer
                .texture
                .split_once("/type/")
                .or_else(|| layer.texture.split_once("/baby/"))
        {
            villager_client::type_hat_shown(
                &m.extra.profession,
                folder == "entity/zombie_villager",
                m.extra.variant,
            )
        } else {
            true
        };
        let on_parent = custom.is_none()
            && body.custom.is_none()
            && matches!(part, WornModel::Vanilla(part) if std::sync::Arc::ptr_eq(part, body_model));
        let uses_body_pose = matches!(part, WornModel::Override(..)) || on_parent;
        let draw_poses = match part {
            WornModel::Vanilla(part) if !uses_body_pose => {
                layer_poses.reset(part);
                anim::setup(m, &s, &mut layer_poses, &data.mob_data().animations);
                if let Some(animation) = custom.and_then(|c| c.animation.as_ref()) {
                    animation.run(m, &s, &mut layer_poses);
                }
                model_posed(
                    m,
                    alpha,
                    camera,
                    data,
                    &layer.model,
                    custom
                        .map(|c| &*c.frame_model)
                        .or_else(|| data.mob_data().frame_models.get(part).map(|l| &**l)),
                    &mut layer_poses,
                );
                &mut layer_poses
            }
            _ => {
                poses.apply_frame_visibility(on_parent && type_hat_shown);
                &mut *poses
            }
        };
        // When the no-hat alternate borrows the body pose, its head visibility
        // change lasts only through this layer and its emissive overlay.
        let restore_head = !type_hat_shown && uses_body_pose && !draw_poses.is_hidden_named("head");
        if !type_hat_shown {
            draw_poses.visible("head", false);
        }
        let light = if layer.full_bright {
            FULL_BRIGHT
        } else {
            light
        };
        // EquipmentLayerRenderer draws armour with OverlayTexture.NO_OVERLAY: never
        // the hurt tint, the armour glint (`ARMOR_GLINT`) when enchanted.
        let overlay = layer.equipment_overlay.unwrap_or(overlay);
        let start = out.len();
        let first_job = skin.as_deref().map(|skin| skin.frame.jobs.len());
        let paint = Paint {
            uv,
            scroll: layer.scroll,
            color: layer.color,
            light,
            overlay,
        };
        part.submit(&mut skin, base, draw_poses, paint, out);
        if overlay != crate::gfx::ARMOR_GLINT_ONLY_OVERLAY
            && let Some(glow) = emissive_uv(drawn)
        {
            part.submit(
                &mut skin,
                base,
                draw_poses,
                Paint {
                    uv: glow,
                    light: FULL_BRIGHT,
                    overlay: 0,
                    ..paint
                },
                out,
            );
        }
        if restore_head {
            draw_poses.visible("head", true);
        }
        if translucent_layer(m, layer) {
            mark_oit(&mut out[start..]);
        }
        // The same marks on what the GPU path posed of the layer (only eyes
        // layers get there among the marked ones): the tags in the overlay word,
        // no normal.
        if let (Some(skin), Some(first)) = (skin.as_deref_mut(), first_job) {
            // Each model chooses its own route in `put`. Submit CPU draws accumulated
            // since this mob's last posed draw before the new posed layer, not after it.
            if let Some(job) = skin.frame.jobs.get(first)
                && cpu_cut < start as u32
            {
                skin.frame.insert_cpu_before(job.dst, cpu_cut..start as u32);
                cpu_cut = start as u32;
            }
            let tagged = translucent_layer(m, layer);
            let eyes = layer.full_bright && m.species() != Species::Warden;
            for job in &mut skin.frame.jobs[first..] {
                if tagged {
                    job.overlay |= crate::gfx::OIT_ENTITY;
                }
                if eyes {
                    job.overlay |= u32::from(EYES_TIER) << 16;
                    job.mode = crate::gfx::entity_skin::SkinJob::FLAT;
                }
            }
        }
        // The eyes render type's glowing eyes also shine in HDR output, at
        // `EYES_TIER` in the overlay's high bits (entity.wgsl fs_hdr_mark).
        if layer.full_bright && m.species() != Species::Warden {
            for vertex in &mut out[start..] {
                vertex.normal = [0; 4];
                vertex.layer |= u32::from(EYES_TIER) << 16;
            }
        }
        // The warden's heart and spots shine too, by how far their layers have faded in.
        if layer.full_bright && m.species() == Species::Warden {
            for vertex in &mut out[start..] {
                vertex.layer |= u32::from(EYES_TIER) << 16;
            }
        }
    }
    // EnergySwirlLayer: the armour model, additive, scrolling over the sprite.
    if let Some((model, texture, x_offset)) = energy_swirl(m, s.age)
        && let Some(part) = body.visuals.layer(model)
        && let Some([u0, v0, u1, v1]) = atlas(texture)
    {
        layer_poses.reset(part);
        anim::setup(m, &s, &mut layer_poses, &data.mob_data().animations);
        model_posed(
            m,
            alpha,
            camera,
            data,
            model,
            data.mob_data().frame_models.get(part).map(|l| &**l),
            &mut layer_poses,
        );
        // `RenderTypes.energySwirl`'s texture offset, as the sprite-local UV.
        let (du, dv) = (x_offset % 1.0, s.age * 0.01 % 1.0);
        let start = out.len();
        draw(
            part,
            base,
            &layer_poses,
            [du, dv, 1.0 + du, 1.0 + dv],
            crate::breaking::stage_light([u1 - u0, v1 - v0]),
            crate::breaking::stage_light([u0, v0]),
            crate::gfx::ENERGY_SWIRL_OVERLAY,
            out,
        );
        // No lighting: the colour bytes carry the sprite's size (fs_energy_swirl).
        for vertex in &mut out[start..] {
            vertex.normal = [0; 4];
        }
    }
    if shell_last {
        draw_body(poses, out, &mut skin);
    }
}
/// `EnergySwirlLayer`'s model, texture and `xOffset(t)` for a charged creeper
/// (`CreeperPowerLayer`) or a powered wither (`WitherArmorLayer`); `t` is `ageInTicks`.
fn energy_swirl(m: &Mob, t: f32) -> Option<(&'static str, &'static str, f32)> {
    match m.species() {
        Species::Creeper if m.extra.charged => {
            Some(("creeper#armor", "entity/creeper/creeper_armor", t * 0.01))
        }
        Species::Wither if super::wither::powered(m) => Some((
            "wither#armor",
            "entity/wither/wither_armor",
            (t * 0.02).mcos() * 3.0,
        )),
        _ => None,
    }
}
/// Render with a fixed texture rect.
pub fn render_single(
    m: &Mob,
    models: &ClientMobModels,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let Some(visuals) = models.get(&m.kind.id) else {
        return;
    };
    let base = base_transform(m, alpha, camera);
    let overlay = body_overlay(m, alpha);
    let s = anim::state(m, alpha);
    let model = &visuals.model;
    let mut poses = Poses::new(model);
    anim::setup(m, &s, &mut poses, &std::collections::HashMap::new());
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, overlay, out);
}

/// The transform of a named part (ModelPart.translateAndRotate down the tree).
pub fn part_matrix(
    part: &Part,
    parent: glam::Mat4,
    poses: &Poses,
    name: &str,
    x_offset: f32,
) -> Option<glam::Mat4> {
    let mut p = poses.pose(part);
    if part.name == name {
        p[0] += x_offset;
    }
    let m = parent
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if part.name == name {
        return Some(m);
    }
    part.children
        .iter()
        .find_map(|c| part_matrix(c, m, poses, name, x_offset))
}
/// The first part named `name`, depth first.
pub(super) fn find_part<'a>(part: &'a Part, name: &str) -> Option<&'a Part> {
    if part.name == name {
        return Some(part);
    }
    part.children.iter().find_map(|c| find_part(c, name))
}
/// The matrix up to a part that declares an item anchor (`attachments` in the pack's file,
/// EMF's `EMFAttachment`): the walk the positioner takes — every part on the way applies its
/// pose, like `part_matrix` — then the anchor's own offset, in model pixels.
fn attachment_matrix(
    part: &Part,
    parent: glam::Mat4,
    poses: &Poses,
    anchors: &[(String, String, [f32; 3])],
    key: &str,
) -> Option<glam::Mat4> {
    let p = poses.pose(part);
    let m = parent
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if let Some((_, _, [x, y, z])) = anchors
        .iter()
        .find(|(bone, kind, _)| *kind == key && part.name == *bone)
    {
        return Some(m * glam::Mat4::from_translation(glam::Vec3::new(*x, *y, *z) / 16.0));
    }
    part.children
        .iter()
        .find_map(|c| attachment_matrix(c, m, poses, anchors, key))
}
/// `AllayModel.translateToHand`: the root and body, then the right arm's
/// pitch, a 0.7 scale and a shift. Both hands use the right arm's pitch.
fn allay_hand(model: &Part, base: glam::Mat4, poses: &Poses) -> Option<glam::Mat4> {
    let body = part_matrix(model, base, poses, "body", 0.0)?;
    let x_rot = poses.pose(find_part(model, "right_arm")?)[3];
    Some(
        body * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0625, 0.1875))
            * glam::Mat4::from_rotation_x(x_rot)
            * glam::Mat4::from_scale(glam::Vec3::splat(0.7))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0625, 0.0, 0.0)),
    )
}
/// `ArmedModel.translateToHand`: a mob's hand in model space, as
/// `ItemInHandLayer.submitArmWithItem` has it before its item rotations. That is the
/// arm's part matrix, moved by the slim or skeleton offset, or the Allay's, Vex's or
/// Copper Golem's own hand. `hand_base` applies vanilla's item rotations and the hand
/// offset after it. A native mod takes the hand here through `[[native.mixin]]`, where a
/// Java mod's mixin on `submitArmWithItem` turns the pose stack before those rotations
/// (#3970). A pack's `attachments` anchor replaces the whole hand and doesn't come
/// through here.
#[sunrise_mod_api_macros::replaceable]
pub fn translate_to_hand(
    m: &Mob,
    model: &Part,
    base: glam::Mat4,
    poses: &Poses,
    left: bool,
) -> Option<glam::Mat4> {
    let skeleton = skeleton_arms(m.species());
    let arm = if left { "left_arm" } else { "right_arm" };
    // `SkeletonModel.translateToHand` shifts the arm by 1, `PlayerModel.translateToHand`
    // by 0.5 for slim arms: the hand is the middle of the arm.
    let offset = if skeleton {
        if left { -1.0 } else { 1.0 }
    } else if avatar(m) && m.slim {
        if left { -0.5 } else { 0.5 }
    } else {
        0.0
    };
    // The arm's matrix, `None` when the model has no such part. The function's only exit is
    // its value (no `?`, no early `return`), so a `RETURN` row applies to every call.
    let arm_matrix = if m.species() == Species::Allay {
        allay_hand(model, base, poses)
    } else {
        part_matrix(model, base, poses, arm, offset)
    };
    arm_matrix.map(|mut t| {
        if m.species() == Species::Vex {
            // `VexModel.translateToHand`: the held item is drawn at 0.55 scale.
            t *= glam::Mat4::from_scale(glam::Vec3::splat(0.55));
            t *= glam::Mat4::from_translation(glam::Vec3::new(
                if left { -0.046_875 } else { 0.046_875 },
                -0.156_25,
                0.078_125,
            ));
        }
        if m.species() == Species::CopperGolem {
            // `CopperGolemModel.translateToHand`: a still golem holds the item
            // across the arm, a working one scaled down in front of it.
            if m.extra.variant3 == super::copper_golem::IDLE {
                t *= glam::Mat4::from_rotation_y((if left { 90.0f32 } else { -90.0 }).to_radians())
                    * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.125));
            } else {
                t *= glam::Mat4::from_scale(glam::Vec3::splat(0.55))
                    * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.3125, -0.1875));
            }
        }
        t
    })
}
/// The skeleton family, whose `SkeletonModel.translateToHand` shifts the arm by 1.
fn skeleton_arms(species: Species) -> bool {
    matches!(
        species,
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::WitherSkeleton
    )
}
/// The hand of a mob in model space, up to the item's own transform
/// (`ItemInHandLayer.submitArmWithItem`).
fn hand_base(
    m: &Mob,
    model: &Part,
    base: glam::Mat4,
    poses: &Poses,
    anchors: &[(String, String, [f32; 3])],
    left: bool,
) -> Option<glam::Mat4> {
    // A pack's `attachments` anchor (`right_handheld_item`...) replaces the whole hand
    // anchor: the walk to the part that declares it and its offset, none of the vanilla
    // hand's own transforms — EMF's one-off override during `translateToHand`. Without one
    // the vanilla anchor stays.
    if let Some(t) = attachment_matrix(
        model,
        base,
        poses,
        anchors,
        if left {
            "left_handheld_item"
        } else {
            "right_handheld_item"
        },
    ) {
        return Some(t);
    }
    let hand = translate_to_hand(m, model, base, poses, left)?;
    Some(item_from_hand(hand, m, left))
}
/// `ItemInHandLayer.submitArmWithItem` after `translateToHand`: the two item rotations,
/// then the baby or adult hand offset, applied to the hand `translate_to_hand` gives.
fn item_from_hand(mut t: glam::Mat4, m: &Mob, left: bool) -> glam::Mat4 {
    let (ox, oy, oz) = if m.is_baby() {
        (0.0, 1.0, -4.5)
    } else {
        (1.0, 2.0, -10.0)
    };
    t *= glam::Mat4::from_rotation_x((-90.0f32).to_radians()) * glam::Mat4::from_rotation_y(PI);
    t *= glam::Mat4::from_translation(glam::Vec3::new(
        if left { -ox } else { ox } / 16.0,
        oy / 16.0,
        oz / 16.0,
    ));
    t
}
/// The transform of one of a mob's model parts, in camera space
/// (`ModelPart.translateAndRotate` down the tree).
pub fn part_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &dyn RenderDataSource,
    part: &str,
) -> Option<glam::Mat4> {
    let base = base_transform(m, alpha, camera);
    let (model, poses, _) = posed_body(m, alpha, data)?;
    part_matrix(&model, base, &poses, part, 0.0)
}
/// `PlayerItemInHandLayer.renderItemHeldToEye` up to the item: the root and
/// the head with its pitch clamped to -30 to 90 degrees, `translateToHead`
/// (`CustomHeadLayer.translateToHead`), then the hand's side of the eye.
pub fn held_to_eye_matrix(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &dyn RenderDataSource,
    left: bool,
) -> Option<glam::Mat4> {
    use glam::{Mat4, Vec3};
    let base = base_transform(m, alpha, camera);
    let (model, mut poses, _) = posed_body(m, alpha, data)?;
    let pitch =
        poses.get("head")[3].clamp(-std::f32::consts::FRAC_PI_6, std::f32::consts::FRAC_PI_2);
    poses.xrot("head", pitch);
    let head = part_matrix(&model, base, &poses, "head", 0.0)?;
    let side = if left { -2.5 / 16.0 } else { 2.5 / 16.0 };
    Some(
        head * Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_rotation_y(std::f32::consts::PI)
            * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
            * Mat4::from_translation(Vec3::new(side, -0.0625, 0.0)),
    )
}
/// `FoxHeldItemLayer`: the item sits at the head's position under the render
/// state's head roll, yaw and pitch (not the posed head's, which differ while
/// it sleeps), pushed out to the mouth.
fn fox_item_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &dyn RenderDataSource,
) -> Option<glam::Mat4> {
    use glam::{Mat4, Vec3};
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let (model, mut poses, anchors) = posed_body(m, alpha, data)?;
    let head = *poses.get("head");
    let fox = super::fox::pose(m, alpha);
    let baby = m.is_baby();
    let mut t = base_transform(m, alpha, camera)
        * Mat4::from_translation(Vec3::new(head[0], head[1], head[2]) / 16.0);
    if baby {
        t *= Mat4::from_scale(Vec3::splat(0.75));
    }
    t *= Mat4::from_rotation_z(fox.head_roll_angle)
        * Mat4::from_rotation_y(s.y_rot.to_radians())
        * Mat4::from_rotation_x(s.x_rot.to_radians());
    // A pack's `fox_item` anchor (Fresh Animations' fox declares one under its rebuilt
    // head) replaces the mouth offset: the walk to the part that declares it and its
    // offset, as EMF's fox offset injects the positioner in place of the layer's first
    // translate. The layer's own rotations (the item's upright) keep applying after
    // either way.
    if let Some(anchor) = attachment_matrix(&model, t, &poses, &anchors, "fox_item") {
        t = anchor;
    } else {
        let (x, y, z) = match (baby, fox.sleeping) {
            (true, true) => (0.4, 0.26, 0.15),
            (true, false) => (0.06, 0.26, -0.5),
            (false, true) => (0.46, 0.26, 0.22),
            (false, false) => (0.06, 0.27, -0.5),
        };
        t *= Mat4::from_translation(Vec3::new(x, y, z));
    }
    t *= Mat4::from_rotation_x(90.0f32.to_radians());
    if fox.sleeping {
        t *= Mat4::from_rotation_z(90.0f32.to_radians());
    }
    Some(t)
}
/// `CrossedArmsItemLayer.applyTranslation`: `translateToArms`, then the item's
/// placement over the folded arms.
fn crossed_arms_matrix(model: &Part, base: glam::Mat4, poses: &Poses) -> Option<glam::Mat4> {
    let arms = part_matrix(model, base, poses, "arms", 0.0)?;
    Some(
        arms * glam::Mat4::from_rotation_x(0.75)
            * glam::Mat4::from_scale(glam::Vec3::splat(1.07))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.13, -0.34))
            * glam::Mat4::from_rotation_x(std::f32::consts::PI),
    )
}
fn hand_arm(left: bool) -> &'static str {
    if left { "left_arm" } else { "right_arm" }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeldItemAnchor {
    Panda,
    Fox,
    Dolphin,
    Witch,
    CrossedArms,
    Hand,
}

fn held_item_anchor(m: &Mob) -> HeldItemAnchor {
    match m.species() {
        Species::Panda => HeldItemAnchor::Panda,
        Species::Fox => HeldItemAnchor::Fox,
        Species::Dolphin => HeldItemAnchor::Dolphin,
        Species::Witch => HeldItemAnchor::Witch,
        Species::Villager | Species::WanderingTrader => HeldItemAnchor::CrossedArms,
        _ => HeldItemAnchor::Hand,
    }
}

/// The pose a mob's held item is submitted under (`ItemInHandLayer`:
/// `translateToHand`, then the hand offset; the item model adds its own
/// display transform), or `None` when the mob shows no held items.
pub fn held_item_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &dyn RenderDataSource,
    left: bool,
) -> Option<glam::Mat4> {
    let anchor = held_item_anchor(m);
    // PandaHoldsItemLayer: the main hand's item in its mouth.
    if anchor == HeldItemAnchor::Panda {
        return super::panda::item_offset(m, alpha)
            .filter(|_| !left)
            .map(|offset| base_transform(m, alpha, camera) * offset);
    }
    // FoxHeldItemLayer: the main hand's item in its mouth.
    if anchor == HeldItemAnchor::Fox {
        return (!left)
            .then(|| fox_item_transform(m, alpha, camera, data))
            .flatten();
    }
    // DolphinCarryingItemLayer: the main hand's item at the snout, lifted and
    // pulled in as the dolphin pitches (up or down) toward 60 degrees.
    if anchor == HeldItemAnchor::Dolphin {
        return (!left).then(|| {
            let pitch = anim::state(m, alpha).drawn_from(m, &data).x_rot;
            let percent = pitch.abs() / 60.0;
            let offset = if pitch < 0.0 {
                glam::Vec3::new(0.0, 1.0 - percent * 0.5, -1.0 + percent * 0.5)
            } else {
                glam::Vec3::new(0.0, 1.0 + percent * 0.8, -1.0 + percent * 0.2)
            };
            base_transform(m, alpha, camera) * glam::Mat4::from_translation(offset)
        });
    }
    if !super::illager::shows_item(m) {
        return None;
    }
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha).drawn_from(m, &data);
    let (model, poses, anchors) = posed_body(m, alpha, data)?;
    // WitchItemLayer.applyTranslation: a potion is held at the nose, any other
    // item as CrossedArmsItemLayer holds it; only the main hand's item is drawn.
    if anchor == HeldItemAnchor::Witch {
        let item = m.extra.main_hand_id()?;
        if left != m.main_arm_left() {
            return None;
        }
        if super::witch::holds_potion(item) {
            let nose = part_matrix(&model, base, &poses, "nose", 0.0)?;
            return Some(
                nose * glam::Mat4::from_translation(glam::Vec3::new(0.0625, 0.25, 0.0))
                    * glam::Mat4::from_rotation_z(180.0f32.to_radians())
                    * glam::Mat4::from_rotation_x(140.0f32.to_radians())
                    * glam::Mat4::from_rotation_z(10.0f32.to_radians())
                    * glam::Mat4::from_rotation_x(180.0f32.to_radians()),
            );
        }
        return crossed_arms_matrix(&model, base, &poses);
    }
    // CrossedArmsItemLayer (villagers and wandering traders): the main hand's
    // item over the folded arms.
    if anchor == HeldItemAnchor::CrossedArms {
        m.extra.main_hand_id()?;
        if left != m.main_arm_left() {
            return None;
        }
        return crossed_arms_matrix(&model, base, &poses);
    }
    let mut t = hand_base(m, &model, base, &poses, &anchors, left)?;
    // ItemInHandLayer: a STAB swing of this arm thrusts its item out
    // (SpearAnimations.thirdPersonAttackItem, the main hand's forward
    // movement), and a spear in use moves with it (ArmPose.SPEAR's
    // animateUseItem).
    let swing_left = m.extra.swing_right_arm.map_or(m.main_arm_left(), |r| !r);
    if m.extra.swing_stab && swing_left == left {
        let forward = m
            .extra
            .main_hand_id()
            .and_then(crate::spear::Kinetic::of)
            .map_or(0.0, |k| k.forward_movement);
        t *= crate::spear::third_person_attack_item(s.swing, forward);
    }
    let off_hand = left != m.main_arm_left();
    if super::spear_use::is_using_item(m)
        && m.extra.using_off_hand == off_hand
        && let Some(kinetic) = (if off_hand {
            m.extra.off_hand_id()
        } else {
            m.extra.main_hand_id()
        })
        .and_then(crate::spear::Kinetic::of)
    {
        let time_held = m.extra.using_item as f32 + s.partial;
        t *= crate::spear::third_person_use_item(&kinetic, time_held, s.swing, 0.0, !left);
    }
    Some(t)
}

/// `ArmedEntityRenderState.extractArmedEntityRenderState`: each arm's item
/// model reads the stack held by that arm: each hand has its own slot, so
/// the off hand never inherits the main hand's item or components.
pub fn held_item_stack(extra: &super::Extra, off_hand: bool) -> Option<crate::items::ItemStack> {
    use sunrise_items::equipment::{MAINHAND, OFFHAND};
    extra
        .slot_stack(if off_hand { OFFHAND } else { MAINHAND })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profession_texture_preserves_nested_namespaces() {
        assert_eq!(
            villager_client::profession_texture("entity/villager", "minecraft:farmer"),
            "entity/villager/profession/farmer"
        );
        assert_eq!(
            villager_client::profession_texture(
                "entity/zombie_villager",
                "example:nested/engineer"
            ),
            "example:entity/zombie_villager/profession/nested/engineer"
        );
    }

    /// A zombie whose model is a humanoid built here (head with its hat, body, arms, legs: one
    /// face each), and baked mob data: what the frame-model tests draw, without `.local`.
    fn humanoid_zombie() -> (Mob, ClientMobModels, ClientMobData) {
        let cube = pose_test_cube;
        let mut head = cube(0.0);
        head["children"] = serde_json::json!({ "hat": cube(0.0) });
        let model = serde_json::json!({ "children": {
            "head": head, "body": cube(0.0), "right_arm": cube(-5.0), "left_arm": cube(5.0),
            "right_leg": cube(-2.0), "left_leg": cube(2.0),
        }});
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::Zombie;
        kind.anim = super::super::Anim::Zombie;
        let mut models = ClientMobModels::default();
        models.insert(
            kind.id.clone(),
            ClientMobVisuals {
                shadow: None,
                model: std::sync::Arc::new(Part::parse("root", &model)),
                layers: Default::default(),
                texture: String::new(),
            },
        );
        let mut mob = Mob::new(
            1,
            std::sync::Arc::new(kind),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        mob.age = 37;
        let data = ClientMobData {
            frame_models: BakedFrameModels::bake(models.types.values()),
            ..Default::default()
        };
        (mob, models, data)
    }

    // The final model and pose come from the body's own draw, on every entry the game draws
    // a mob or player through (CPU, GPU, layers only), and keeping them changes nothing drawn
    // (#2060, #2464).
    include!("render_pose_tests.rs");

    /// A part `render_player_pre` hid leaves the body, after Skin Customization, while the armour
    /// layered on it keeps the part (#2060, part 2).
    #[test]
    fn a_hidden_part_leaves_the_body_but_not_its_armour() {
        let (mob, models, data) = humanoid_zombie();
        let source = ClientMobRender {
            models: &models,
            data: &data,
        };
        let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
        let armour = models.model(&mob.kind.id).unwrap().clone();
        let extra = [(
            WornModel::Vanilla(&armour),
            [0.0, 0.0, 1.0, 1.0],
            0xFFFF_FFFF,
            0,
            false,
        )];
        let draw_parts = |names: &[&str], extra: &[WornLayer<'_>]| {
            let mut out = Vec::new();
            let mut frame = FrameDraw::new(true);
            hide_in_draw(&mut frame, &mob, &data, &models, names);
            render_with_frame(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &source,
                extra,
                &mut frame,
                &mut out,
            );
            (out.len(), frame.model.expect("the body model"))
        };
        let draw = |names: &[&str], extra: &[WornLayer<'_>]| draw_parts(names, extra).0;
        let whole = draw(&[], &[]);
        let one_face = 4;
        assert_eq!(draw(&["right_arm"], &[]), whole - one_face);
        assert_eq!(
            draw(&["head"], &[]),
            whole - 2 * one_face,
            "the head's child, the hat, goes with it"
        );
        assert_eq!(
            draw(&[], &extra),
            2 * whole,
            "the armour layer copies every part"
        );
        assert_eq!(
            draw(&["right_arm"], &extra),
            2 * whole - one_face,
            "only the body loses the arm"
        );

        // The final pose stays in place when a pre hook hides a part, and visibility
        // propagates to all its children even though only the head was marked hidden.
        use mod_api::FrameModel;
        let (_, shown) = draw_parts(&[], &[]);
        let (_, headless) = draw_parts(&["head"], &[]);
        for name in ["head", "hat", "body", "right_arm"] {
            let original = shown.part(name).unwrap();
            let edited = headless.part(name).unwrap();
            assert!(original.visible);
            assert_eq!(edited.visible, !matches!(name, "head" | "hat"), "{name}");
            assert_eq!(edited.pose(), original.pose(), "{name}");
        }
    }

    #[test]
    fn creeper_swelling_uses_local_fuse_and_keeps_java_zero_division() {
        let mut mob = Mob::new(
            1,
            crate::mobs::sensor_scan_tests::kind(),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        mob.previous_swell = 19;
        mob.swell = 20;
        assert_eq!(creeper_swelling(&mob, 0.5), 19.5 / 28.0);
        mob.max_swell = 40;
        assert_eq!(creeper_swelling(&mob, 0.5), 19.5 / 38.0);
        mob.max_swell = 2;
        assert!(creeper_swelling(&mob, 0.5).is_infinite());
        mob.previous_swell = 0;
        mob.swell = 0;
        assert!(creeper_swelling(&mob, 0.5).is_nan());
    }

    #[test]
    fn offhand_does_not_inherit_mainhand_components() {
        use crate::items::ItemStack;
        let mut extra = super::super::Extra::default();
        let mut main = ItemStack::new("minecraft:crossbow", 1);
        main.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Main hand".into()));
        extra.set_main_hand_stack(Some(main));
        extra.set_slot_id(
            sunrise_items::equipment::OFFHAND,
            Some("minecraft:crossbow".into()),
        );
        assert_eq!(
            held_item_stack(&extra, false)
                .unwrap()
                .custom_name()
                .as_deref(),
            Some("Main hand")
        );
        assert!(
            held_item_stack(&extra, true)
                .unwrap()
                .custom_name()
                .is_none()
        );
        // An admiring piglin holds gold beside its crossbow. Both the item
        // id and the model components must come from the appropriate hand.
        extra.set_slot_id(
            sunrise_items::equipment::OFFHAND,
            Some("minecraft:gold_ingot".into()),
        );
        assert_eq!(
            &*held_item_stack(&extra, true).unwrap().item,
            "minecraft:gold_ingot"
        );
        assert_eq!(
            &*held_item_stack(&extra, false).unwrap().item,
            "minecraft:crossbow"
        );
        extra.set_slot_id(sunrise_items::equipment::OFFHAND, None);
        assert!(held_item_stack(&extra, true).is_none());
    }

    /// The elytra's flight yaw (`AvatarRenderer.extractFlightData`) lerps
    /// the velocity of the previous tick towards this tick's, so the model
    /// does not step once per tick while the view turns (#4104).
    #[test]
    fn the_elytra_flight_yaw_lerps_the_velocity_between_ticks() {
        use crate::mobs::{Anim, Brain, Flags, MobType};
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;

        let kind = Arc::new(MobType {
            type_key: Default::default(),
            id: "minecraft:player".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.1,
            flying_speed: 0.02,
            attack_damage: 1.0,
            attack_knockback: 0.0,
            follow_range: 0.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Flags::default(),
            brain: Brain::Passive,
            anim: Anim::Camel,
            sounds: [""; 3],
            sound_overrides: Default::default(),
            food: Arc::default(),
            tempt: Arc::default(),
            passenger_attachments: Vec::new(),
        });
        let gliding = |previous: Vec3, current: Vec3| {
            let mut m = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
            m.extra.fall_flying = true;
            m.extra.fall_fly_ticks = 20;
            m.extra.previous_velocity = previous;
            m.velocity = current;
            m
        };
        let from = Vec3::new(0.4, 0.0, 0.4);
        let to = Vec3::new(0.0, 0.0, 0.5);
        let at = |m: &Mob, alpha: f32| base_transform_at(m, alpha, glam::Mat4::IDENTITY);
        let differs = |a: glam::Mat4, b: glam::Mat4| {
            a.to_cols_array()
                .iter()
                .zip(b.to_cols_array())
                .any(|(x, y)| (x - y).abs() > 1.0e-4)
        };
        let m = gliding(from, to);
        // The ends are the two ticks' velocities.
        assert!(!differs(at(&m, 0.0), at(&gliding(from, from), 0.0)));
        assert!(!differs(at(&m, 1.0), at(&gliding(to, to), 1.0)));
        // Between them the yaw is that of the lerped velocity, which is
        // neither end: the current tick's alone made it snap.
        let mid = Vec3::new(0.2, 0.0, 0.45);
        let half = at(&m, 0.5);
        assert!(!differs(half, at(&gliding(mid, mid), 0.5)));
        assert!(differs(half, at(&gliding(to, to), 0.5)));
        assert!(differs(half, at(&gliding(from, from), 0.5)));
    }

    /// Six faces whose corners span `min` to `max`.
    fn cube(min: [f32; 3], max: [f32; 3]) -> Vec<super::super::Polygon> {
        let corner = |p: [f32; 3]| [p[0], p[1], p[2], 0.0, 0.0];
        (0..6)
            .map(|_| super::super::Polygon {
                normal: [0.0; 3],
                vertices: [corner(min), corner(max), corner(min), corner(max)],
                cull_back: false,
            })
            .collect()
    }

    /// `ModelPart.cubes`: the corners of each cube of a part, six faces apiece.
    #[test]
    fn cube_bounds_follow_the_faces_of_each_cube() {
        let mut polygons = cube([-4.0, -8.0, -2.0], [4.0, 0.0, 2.0]);
        polygons.extend(cube([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]));
        let part = Part {
            name: "body".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons,
            children: Vec::new(),
            layer: None,
        };
        assert_eq!(
            cube_bounds(&part),
            vec![
                [-4.0, -8.0, -2.0, 4.0, 0.0, 2.0],
                [0.0, 0.0, 0.0, 1.0, 2.0, 3.0]
            ]
        );
    }

    /// `StuckInBodyLayer.snapToFace`.
    #[test]
    fn a_stuck_item_snaps_to_the_far_or_the_middle_face() {
        assert_eq!(snap_to_face(0.75), 1.0);
        assert_eq!(snap_to_face(0.5), 0.5);
        assert_eq!(snap_to_face(0.1), 0.5);
    }

    #[test]
    fn shoulder_parrot_textures_follow_the_variant() {
        assert_eq!(shoulder_parrot_texture(0), "entity/parrot/parrot_red_blue");
        assert_eq!(shoulder_parrot_texture(4), "entity/parrot/parrot_grey");
        // Parrot.Variant.byId clamps an id out of range.
        assert_eq!(shoulder_parrot_texture(5), "entity/parrot/parrot_grey");
        assert_eq!(shoulder_parrot_texture(-1), "entity/parrot/parrot_red_blue");
    }

    /// VillagerProfessionLayer's textures (type, or baby for a baby;
    /// profession; level) of every villager and zombie villager are in the
    /// prepared resources: a missing one draws nothing.
    #[test]
    fn villager_layers_name_textures_that_exist() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        for kind in ["minecraft:villager", "minecraft:zombie_villager"] {
            let kind = std::sync::Arc::new(types.types[kind].clone());
            for baby in [false, true] {
                for t in 0..VILLAGER_TYPES.len() {
                    for p in 0..crate::mobs::villager::profession_table().entries().len() {
                        let mut m =
                            Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
                        m.extra.age = if baby { -24000 } else { 0 };
                        (m.extra.variant, m.extra.villager_level) = (t as u8, 5);
                        m.extra.profession =
                            super::super::villager::ProfessionState::from_index(p as u8);
                        let layers = layers(&m, m.age as f32);
                        let group = if baby { "/baby/" } else { "/type/" };
                        assert!(
                            layers.iter().any(|l| l.texture.contains(group)),
                            "{kind:?} baby={baby}: no {group} layer"
                        );
                        for layer in layers {
                            let png = root
                                .join(format!("assets/minecraft/textures/{}.png", layer.texture));
                            assert!(png.is_file(), "missing {}", png.display());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sulfur_cube_opaque_core_is_submitted_before_its_translucent_shell() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let mut types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types.remove("minecraft:sulfur_cube").unwrap());
        let models = ClientMobModels::load(&root).unwrap();
        let client_data = ClientMobData::default();
        let data = ClientMobRender {
            models: &models,
            data: &client_data,
        };
        let atlas = |name: &str| {
            Some(if name.contains("_inner") {
                [0.0, 0.0, 0.25, 0.25]
            } else {
                [0.5, 0.5, 0.75, 0.75]
            })
        };
        for baby in [false, true] {
            let mut mob = Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.age = if baby { -24000 } else { 0 };
            super::super::sulfur_cube::finalize(&mut mob);
            assert_eq!(mob.is_baby(), baby);
            for posed in [false, true] {
                let mut vertices = Vec::new();
                if posed {
                    render_posed(
                        &mob,
                        0.5,
                        glam::Mat4::IDENTITY,
                        &atlas,
                        0xf0f0,
                        &data,
                        &mut vertices,
                    );
                } else {
                    render(
                        &mob,
                        0.5,
                        glam::DVec3::ZERO,
                        &atlas,
                        0xf0f0,
                        &data,
                        &mut vertices,
                    );
                }
                assert_eq!(
                    vertices.len(),
                    48,
                    "both real six-face models must be drawn"
                );
                assert!(
                    vertices[..24].iter().all(|v| v.uv[0] <= 0.25),
                    "opaque core must precede shell"
                );
                assert!(vertices[24..].iter().all(|v| v.uv[0] >= 0.5));
            }
            mob.extra.set_body(Some("minecraft:stone".into()));
            let mut vertices = Vec::new();
            render(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &mut vertices,
            );
            assert_eq!(
                vertices.len(),
                24,
                "a swallowed block still hides the default core"
            );
            assert!(vertices.iter().all(|v| v.uv[0] >= 0.5));
        }
    }
    /// CopperGolemRenderer: the emissive eye layer draws the eye texture on
    /// the body's model, every polygon of it, at each oxidation level. The
    /// prepared `copper_golem#eyes` model has no polygons, so the eyes
    /// showed the body texture's black sockets.
    #[test]
    fn copper_golem_eyes_are_drawn_on_the_body_model() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:copper_golem"].clone());
        let models = ClientMobModels::load(&root).unwrap();
        let client_data = ClientMobData::default();
        let data = ClientMobRender {
            models: &models,
            data: &client_data,
        };
        let atlas = |name: &str| {
            Some(if name.contains("copper_golem_eyes") {
                [0.5, 0.5, 0.75, 0.75]
            } else {
                [0.0, 0.0, 0.25, 0.25]
            })
        };
        for variant in 0..4 {
            let mut mob = Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.variant = variant;
            let mut vertices = Vec::new();
            render(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &mut vertices,
            );
            let body = vertices.iter().filter(|v| v.uv[0] <= 0.25).count();
            let eyes = vertices.iter().filter(|v| v.uv[0] >= 0.5).count();
            assert!(body > 0, "variant {variant}: no body drawn");
            assert_eq!(
                eyes, body,
                "variant {variant}: the eye layer covers the body"
            );
        }
    }
    /// `VexModel.translateToHand`: the held item is drawn at 0.55 scale and
    /// moved by the vex's own offset, so the vex's sword is the tiny one
    /// vanilla shows. It was drawn at the full size, like a zombie's.
    #[test]
    fn the_vex_holds_its_item_at_the_small_scale() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:vex"].clone());
        let mob = Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
        let models = ClientMobModels::load(&root).unwrap();
        let client_data = ClientMobData::default();
        let data = ClientMobRender {
            models: &models,
            data: &client_data,
        };
        for left in [false, true] {
            let t = held_item_transform(&mob, 0.5, glam::DVec3::ZERO, &data, left).unwrap();
            // The model's own part scale is 1, so the item's scale is the
            // renderer's: `poseStack.scale(0.55F, 0.55F, 0.55F)`.
            for axis in [t.x_axis, t.y_axis, t.z_axis] {
                let scale = axis.truncate().length();
                assert!(
                    (scale - 0.55).abs() < 1.0e-5,
                    "the vex's item is drawn at 0.55 scale, got {scale}"
                );
            }
        }
    }

    /// `CrossedArmsItemLayer`: a villager and a wandering trader show their
    /// main hand's item over the folded arms, and nothing in the off hand.
    #[test]
    fn villagers_and_traders_hold_items_in_crossed_arms() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let models = ClientMobModels::load(&root).unwrap();
        let client_data = ClientMobData::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &models,
        )
        .unwrap();
        let data = ClientMobRender {
            models: &models,
            data: &client_data,
        };
        for id in ["minecraft:villager", "minecraft:wandering_trader"] {
            let kind = std::sync::Arc::new(types.types[id].clone());
            let mut mob = Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
            let main = mob.main_arm_left();
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, main).is_none());
            mob.extra.set_main_hand(Some("minecraft:emerald".into()));
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, main).is_some());
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, !main).is_none());
        }
    }
    /// PhantomRenderer.scale moves the model down into its 0.5-high box:
    /// the model's origin ends 1.501 - 1.3125 blocks above the feet (times
    /// the size's scale), not 1.501 like other mobs (#3008).
    #[test]
    fn the_phantom_model_sits_in_its_box() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:phantom"].clone());
        let mut mob = Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
        for (size, scale) in [(0, 1.0f32), (5, 1.75)] {
            mob.extra.size = size;
            let origin =
                base_transform(&mob, 1.0, glam::DVec3::ZERO).transform_point3(glam::Vec3::ZERO);
            let y = (1.501 - 1.3125) * scale;
            assert!(
                (origin.y - y).abs() < 1.0e-4,
                "size {size}: origin {origin}"
            );
            assert!(
                (origin.z.abs() - 0.1875 * scale).abs() < 1.0e-4,
                "size {size}: origin {origin}"
            );
        }
    }
    /// `EquipmentLayerRenderer.renderLayers`: the extra armour layers (a
    /// player's) take their own overlay, `NO_OVERLAY` or the armour glint,
    /// never the hurt tint of the body, and the glint samples the model UV.
    #[test]
    fn extra_armour_layers_carry_their_glint_not_the_hurt_tint() {
        // The batch gives these overlays their own materials (13, the water
        // mask, draws depth only): the glint must not be one of them.
        use crate::gfx::{
            ENERGY_SWIRL_OVERLAY, SHADOW_OVERLAY, TEXT_POLYGON_OFFSET_OVERLAY,
            TEXT_POLYGON_OFFSET_SOLID_OVERLAY, TRANSLUCENT_GLINT_ITEM_OVERLAY,
            TRANSLUCENT_ITEM_OVERLAY, TRANSLUCENT_PARTICLE_OVERLAY, WATER_MASK_OVERLAY,
        };
        assert!(
            ![
                WATER_MASK_OVERLAY,
                SHADOW_OVERLAY,
                TRANSLUCENT_PARTICLE_OVERLAY,
                TRANSLUCENT_ITEM_OVERLAY,
                TRANSLUCENT_GLINT_ITEM_OVERLAY,
                ENERGY_SWIRL_OVERLAY,
                TEXT_POLYGON_OFFSET_OVERLAY,
                TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
            ]
            .contains(&ARMOR_GLINT)
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
        let mut mob = Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
        mob.hurt_time = 5;
        let models = ClientMobModels::load(&root).unwrap();
        let client_data = ClientMobData::default();
        let data = ClientMobRender {
            models: &models,
            data: &client_data,
        };
        let atlas = |_: &str| Some([0.0, 0.0, 0.25, 0.25]);
        let part = models.model("minecraft:zombie").unwrap();
        for overlay in [ARMOR_GLINT, 0] {
            let mut vertices = Vec::new();
            render_with(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &[(
                    WornModel::Vanilla(part),
                    [0.5, 0.5, 0.75, 0.75],
                    0xFFFF_FFFF,
                    overlay,
                    false,
                )],
                &mut vertices,
            );
            let (armour, body): (Vec<&Vertex>, Vec<&Vertex>) =
                vertices.iter().partition(|v| v.uv[0] >= 0.5);
            assert!(!armour.is_empty() && !body.is_empty());
            // The hurt body keeps its red overlay.
            let hurt = body_overlay(&mob, 0.5);
            assert_ne!(hurt, 0);
            assert!(body.iter().all(|v| v.layer == hurt));
            assert!(armour.iter().all(|v| v.layer == overlay));
            // The glint scrolls the layer's model UV (0..1 over the texture).
            for v in &armour {
                if overlay == ARMOR_GLINT {
                    assert!((v.glint_uv[0] - (v.uv[0] - 0.5) * 4.0).abs() < 1e-4);
                    assert!((v.glint_uv[1] - (v.uv[1] - 0.5) * 4.0).abs() < 1e-4);
                } else {
                    assert_eq!(v.glint_uv, [0.0, 0.0]);
                }
            }
        }
    }
    #[test]
    fn a_dyed_leather_armour_keeps_its_colour() {
        // EquipmentLayerRenderer.getColorForLayer: a dyeable layer takes
        // the stack's dye colour, the colour it has when undyed otherwise.
        let mut stack = crate::items::ItemStack {
            item: "minecraft:leather_helmet".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        // Undyed leather: the colour the leather layer has when undyed.
        assert_eq!(armor_tint(Some(0xA06540), Some(&stack)), abgr(0xA06540));
        // Dyed: the stack's own colour.
        stack.data_mut().dyed_color = Some(0x0000FF);
        assert_eq!(armor_tint(Some(0xA06540), Some(&stack)), abgr(0x0000FF));
        // A layer that is not dyeable is not tinted.
        assert_eq!(armor_tint(None, Some(&stack)), 0xFFFF_FFFF);
    }

    /// `ColorLerper.getLerpedColor(Type.SHEEP, ageInTicks)`: the points the
    /// 26.3 jar gave — 0, half a step, one whole step, a step boundary, the
    /// last step back into white, and the 400-tick (16-step) wrap.
    #[test]
    fn jeb_wool_lerps_through_the_sixteen_dye_colours() {
        assert_eq!(jeb_wool_rgb(0.0), 0xE6E6E6);
        assert_eq!(jeb_wool_rgb(12.5), 0xD0A37D);
        assert_eq!(jeb_wool_rgb(25.0), 0xBA6015);
        assert_eq!(jeb_wool_rgb(25.0), sheep_wool_rgb(1));
        assert_eq!(jeb_wool_rgb(50.0), 0x953A8D);
        assert_eq!(jeb_wool_rgb(399.0), 0xDDDDDD);
        assert_eq!(jeb_wool_rgb(400.0), 0xE6E6E6);
    }

    fn sheep_layer(m: &Mob, model: &str, age: f32) -> Option<Layer> {
        layers(m, age).into_iter().find(|l| l.model == model)
    }

    /// SheepRenderer.extractRenderState's `jeb_` magic name: the wool layer
    /// takes the lerped colour and cycles with the age, a white `jeb_` sheep
    /// keeps its undercoat (SheepWoolUndercoatLayer draws for it too), and
    /// the dye colour, the baby model and the shears still win.
    #[test]
    fn a_jeb_sheep_cycles_its_wool_colour_and_keeps_the_undercoat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:sheep"].clone());
        let mut mob = Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0);
        mob.custom_name = Some("jeb_".into());
        let young = sheep_layer(&mob, "sheep#wool", 0.0)
            .expect("an unsheared sheep draws its wool")
            .color;
        assert_eq!(young, abgr(0xE6E6E6));
        let older = sheep_layer(&mob, "sheep#wool", 100.0)
            .expect("an unsheared sheep draws its wool")
            .color;
        assert_ne!(young, older, "the colour cycles with the age");
        let undercoat = sheep_layer(&mob, "sheep#wool_undercoat", 0.0)
            .expect("a jeb_ sheep keeps its undercoat");
        assert_eq!(undercoat.color, abgr(0xE6E6E6));
        // A name that is not exactly "jeb_" leaves a dyed sheep's colour, and
        // a non-white sheep has its undercoat too.
        mob.custom_name = Some("jeb".into());
        mob.extra.color = 14;
        let dyed = abgr(sheep_wool_rgb(14));
        assert_eq!(
            sheep_layer(&mob, "sheep#wool", 0.0)
                .expect("an unsheared sheep draws its wool")
                .color,
            dyed
        );
        assert_eq!(
            sheep_layer(&mob, "sheep#wool_undercoat", 0.0)
                .expect("a dyed sheep keeps its undercoat")
                .color,
            dyed
        );
        // A baby `jeb_` sheep gets the lerped colour on the baby wool and
        // never an undercoat.
        mob.custom_name = Some("jeb_".into());
        mob.extra.color = 0;
        mob.extra.age = -24000;
        assert!(mob.is_baby());
        assert_eq!(
            sheep_layer(&mob, "sheep_baby#wool", 50.0)
                .expect("a baby sheep draws its wool")
                .color,
            abgr(0x953A8D)
        );
        assert!(sheep_layer(&mob, "sheep#wool_undercoat", 50.0).is_none());
        // Sheared: no wool layer at all.
        mob.extra.age = 0;
        mob.extra.sheared = true;
        assert!(sheep_layer(&mob, "sheep#wool", 0.0).is_none());
    }
}

#[cfg(test)]
mod scroll_tests {
    #[test]
    fn a_pufferfish_bobs_by_eight_hundredths() {
        // PufferfishRenderer.setupRotations: cos(ageInTicks * 0.05) * 0.08.
        assert!((super::pufferfish_bob_y(0.0) - 0.08).abs() < 1e-6);
        let quarter = std::f32::consts::FRAC_PI_2 / 0.05;
        assert!(super::pufferfish_bob_y(quarter).abs() < 1e-3);
        assert!(super::pufferfish_bob_y(2.0 * quarter) < -0.0799);
    }

    use super::*;

    #[test]
    fn the_scrolling_texture_wraps_around() {
        // RenderTypes.breezeWind: the breeze's wind layer's texture scrolls
        // by 0.02 of the texture per tick, the wind charge's by 0.03.
        assert_eq!(scroll_offset(0.0, BREEZE_WIND_SCROLL), 0.0);
        assert!((scroll_offset(25.0, BREEZE_WIND_SCROLL) - 0.5).abs() < 1e-6);
        assert!((scroll_offset(50.0, BREEZE_WIND_SCROLL) - 0.0).abs() < 1e-6);
        assert!((scroll_offset(50.0, WIND_CHARGE_SCROLL) - 0.5).abs() < 1e-6);
        assert!((scroll_offset(100.0, WIND_CHARGE_SCROLL) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn the_scrolled_u_wraps_inside_the_texture() {
        // Without a scroll the U is the vertex's own.
        assert_eq!(scroll_u(0.25, 0.0), 0.25);
        assert_eq!(scroll_u(1.0, 0.0), 1.0);
        // With one, it moves and wraps around the texture.
        assert!((scroll_u(0.25, 0.5) - 0.75).abs() < 1e-6);
        assert!((scroll_u(0.75, 0.5) - 0.25).abs() < 1e-6);
        assert!((scroll_u(1.0, 0.5) - 0.5).abs() < 1e-6);
    }

    /// `SheepWoolLayer.submit` returns early on a sheared sheep, but
    /// `SheepWoolUndercoatLayer.submit` has no isSheared guard: a sheared
    /// dyed sheep keeps its undercoat and loses only its wool.
    #[test]
    fn a_sheared_dyed_sheep_keeps_its_wool_undercoat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:sheep"].clone());
        let sheep = |color: u8, sheared: bool, baby: bool| {
            let mut m = Mob::new(1, kind.clone(), sunrise_core::geometry::Vec3::ZERO, 0.0);
            m.extra.color = color;
            m.extra.sheared = sheared;
            m.extra.age = if baby { -24000 } else { 0 };
            m
        };
        // The sheep's wool layers, model, texture and colour, in submission order.
        let wool = |m: &Mob| -> Vec<(String, String, u32)> {
            layers(m, 0.0)
                .into_iter()
                .filter(|l| l.model.contains("#wool"))
                .map(|l| (l.model.into_owned(), l.texture.into_owned(), l.color))
                .collect()
        };
        let dyed = abgr(sheep_wool_rgb(1));
        let layer = |model: &str, texture: &str| (model.to_owned(), texture.to_owned(), dyed);

        // The regression: the pre-fix single `!sheared` guard skipped both
        // layers, so a sheared dyed sheep drew nothing. It must keep the
        // undercoat alone, in its own colour and texture.
        assert_eq!(
            wool(&sheep(1, true, false)),
            [layer(
                "sheep#wool_undercoat",
                "entity/sheep/sheep_wool_undercoat"
            )]
        );
        // Guards, already green before the fix: a sheared white sheep has no
        // undercoat to keep, a baby has no undercoat at all, and an unsheared
        // dyed sheep draws both layers as before.
        assert!(wool(&sheep(0, true, false)).is_empty());
        assert_eq!(
            wool(&sheep(1, false, true)),
            [layer("sheep_baby#wool", "entity/sheep/sheep_wool_baby")]
        );
        assert!(wool(&sheep(1, true, true)).is_empty());
        assert_eq!(
            wool(&sheep(1, false, false)),
            [
                layer("sheep#wool_undercoat", "entity/sheep/sheep_wool_undercoat"),
                layer("sheep#wool", "entity/sheep/sheep_wool")
            ]
        );
    }

    /// `anim::Poses` keys parts by a hash of their name: within any vanilla model, two
    /// different names never share one (they would share a pose).
    #[test]
    fn no_two_part_names_share_a_key() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        fn names<'a>(p: &'a Part, out: &mut Vec<&'a str>) {
            out.push(&p.name);
            for c in &p.children {
                names(c, out);
            }
        }
        let mob_models = ClientMobModels::load(&root).unwrap();
        let mut models = 0;
        for (id, kind) in &mob_models.types {
            for model in std::iter::once(&kind.model).chain(kind.layers.values()) {
                let mut all = Vec::new();
                names(model, &mut all);
                let mut keys = std::collections::HashMap::new();
                for name in all {
                    let other = *keys.entry(super::super::anim::key(name)).or_insert(name);
                    assert_eq!(other, name, "{id} shares a key");
                }
                models += 1;
            }
        }
        assert!(models > 100);
    }

    /// The CPU cost of posing a mob farm (tools/stress_scenes.py `stress_mobs`): 1000 mobs of
    /// 8 kinds through `render_with`, as `mob_geometry` draws them each frame. Prints the median
    /// thread CPU time per frame (busy machines stretch wall time), the vertices and a hash of them (the same hash before and after a speed-up
    /// means the same picture). `cargo test --profile fast -p sunrise-client --lib
    /// benchmark_mob_farm_geometry -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual benchmark; needs the prepared resources"]
    fn benchmark_mob_farm_geometry() {
        use std::{hash::Hasher, hint::black_box, time::Instant};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let mob_models = ClientMobModels::load(&root).unwrap();
        let kinds = [
            "husk", "creeper", "cow", "pig", "sheep", "chicken", "villager", "spider",
        ]
        .map(|k| std::sync::Arc::new(types.types[&format!("minecraft:{k}")].clone()));
        let mobs: Vec<Mob> = (0..1000u32)
            .map(|i| {
                let (layer, row, col) = (i / 200, i / 20 % 10, i % 20);
                let at = sunrise_core::geometry::Vec3::new(
                    -14.25 + 1.5 * f64::from(col),
                    1.0 + 2.5 * f64::from(layer),
                    8.0 + 2.0 * f64::from(row),
                );
                let mut m = Mob::new(i + 1, kinds[i as usize % 8].clone(), at, 180.0);
                m.previous = at;
                m.age = i;
                m
            })
            .collect();
        let client_data = ClientMobData::default();
        let data = ClientMobRender {
            models: &mob_models,
            data: &client_data,
        };
        let atlas = |name: &str| {
            let h = name.len() as f32 / 64.0;
            Some([h, h, h + 0.25, h + 0.25])
        };
        let camera = glam::DVec3::new(0.5, 9.6, 0.5);
        let mut out = Vec::new();
        let frame = |out: &mut Vec<Vertex>| {
            out.clear();
            for m in &mobs {
                render_with(m, 0.5, camera, &atlas, 0xf0f0, &data, &[], out);
            }
        };
        frame(&mut out);
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        hash.write(bytemuck::cast_slice(&out));
        // The thread's own CPU time where the platform gives it, else wall time.
        let timed = |f: &mut dyn FnMut()| {
            let (wall, cpu) = (Instant::now(), sunrise_client_render::spans::thread_cpu());
            f();
            match (sunrise_client_render::spans::thread_cpu(), cpu) {
                (Some(end), Some(start)) => (end - start).as_secs_f64() * 1000.0,
                _ => wall.elapsed().as_secs_f64() * 1000.0,
            }
        };
        let mut times = Vec::new();
        for _ in 0..30 {
            times.push(timed(&mut || frame(black_box(&mut out))));
        }
        times.sort_by(f64::total_cmp);
        println!(
            "mob farm: 1000 mobs, {} vertices, {:.3} ms/frame (median of 30, min {:.3}), hash {:016x}",
            out.len(),
            times[15],
            times[0],
            hash.finish()
        );
        // Where it goes: the body's poses alone (state, pose map, setupAnim), then the
        // body's vertices alone from those poses.
        let median = |mut f: Box<dyn FnMut() + '_>| {
            let mut t: Vec<f64> = (0..30).map(|_| timed(&mut f)).collect();
            t.sort_by(f64::total_cmp);
            t[15]
        };
        let posed: Vec<(std::sync::Arc<Part>, Poses)> = mobs
            .iter()
            .map(|m| {
                let model = mob_models.model(&m.kind.id).unwrap().clone();
                let mut poses = Poses::new(&model);
                anim::setup(m, &anim::state(m, 0.5), &mut poses, &client_data.animations);
                (model, poses)
            })
            .collect();
        let poses_ms = median(Box::new(|| {
            for m in &mobs {
                let model = mob_models.model(&m.kind.id).unwrap().clone();
                let mut poses = Poses::new(&model);
                anim::setup(m, &anim::state(m, 0.5), &mut poses, &client_data.animations);
                black_box(&poses);
            }
        }));
        let mut body = Vec::new();
        let draw_ms = median(Box::new(|| {
            body.clear();
            for (m, (model, poses)) in mobs.iter().zip(&posed) {
                let base = base_transform(m, 0.5, camera);
                draw(
                    model,
                    base,
                    poses,
                    [0.0, 0.0, 0.25, 0.25],
                    u32::MAX,
                    0xf0f0,
                    0,
                    &mut body,
                );
            }
            black_box(&body);
        }));
        println!(
            "  body poses only {poses_ms:.3} ms; body vertices only {draw_ms:.3} ms ({} vertices)",
            body.len()
        );
    }
}

#[cfg(test)]
mod emissive_tests {
    use super::*;
    use crate::mobs::{Polygon, Species};
    use std::sync::{Arc, RwLock};
    use sunrise_optifine::{blink::Blinks, emissive::Emissive, random_textures::RandomTextures};

    /// A silverfish (drawn with its type's texture) whose model is one quad.
    fn silverfish() -> (Mob, ClientMobModels) {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.species = Species::Silverfish;
        let mut models = ClientMobModels::default();
        models.insert(
            k.id.clone(),
            ClientMobVisuals {
                shadow: None,
                model: Arc::new(Part {
                    name: "root".into(),
                    pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    polygons: vec![Polygon {
                        normal: [0.0, 0.0, -1.0],
                        cull_back: false,
                        vertices: [
                            [0.0, 0.0, 0.0, 0.0, 0.0],
                            [1.0, 0.0, 0.0, 1.0, 0.0],
                            [1.0, 1.0, 0.0, 1.0, 1.0],
                            [0.0, 1.0, 0.0, 0.0, 1.0],
                        ],
                    }],
                    children: Vec::new(),
                    layer: None,
                }),
                layers: Default::default(),
                texture: "entity/silverfish".into(),
            },
        );
        (
            Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0),
            models,
        )
    }

    fn data(files: &[&str]) -> ClientMobData {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "sunrise-render-emissive-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            // Rules files pick variant 2; images are only their names.
            let text = if file.ends_with(".properties") {
                "textures.1=2"
            } else {
                ""
            };
            std::fs::write(path, text).unwrap();
        }
        let random = RandomTextures::scan(
            &sunrise_assets::resource_source::ResourceSource::directory(root.to_owned()),
        );
        let blinks = Blinks::scan(
            &sunrise_assets::resource_source::ResourceSource::directory(root.to_owned()),
            &random,
        );
        let mut emissive = Emissive::scan(
            &sunrise_assets::resource_source::ResourceSource::directory(root.to_owned()),
            &random,
        );
        emissive.pair_beside(
            &sunrise_assets::resource_source::ResourceSource::directory(root.to_owned()),
            blinks.files(),
        );
        let _ = std::fs::remove_dir_all(&root);
        ClientMobData {
            random_textures: RwLock::new(random),
            emissive: RwLock::new(Arc::new(emissive)),
            blinks: RwLock::new(blinks),
            ..Default::default()
        }
    }

    /// Every sprite the test atlas has, each at its own rect.
    fn atlas(name: &str) -> Option<[f32; 4]> {
        match name {
            "entity/silverfish" => Some([0.0, 0.0, 0.5, 0.5]),
            "entity/silverfish_e" => Some([0.5, 0.5, 1.0, 1.0]),
            "entity/silverfish_blink" => Some([0.0, 0.5, 0.5, 1.0]),
            _ => None,
        }
    }

    #[test]
    fn an_emissive_overlay_draws_the_body_again_at_full_brightness() {
        let lit = (3 * 16) | ((5 * 16) << 8);
        let (m, models) = silverfish();
        let draw = |data: &ClientMobData| {
            let source = ClientMobRender {
                models: &models,
                data,
            };
            let mut out = Vec::new();
            render_with(
                &m,
                0.0,
                glam::DVec3::ZERO,
                &atlas,
                lit,
                &source,
                &[],
                &mut out,
            );
            out
        };
        // No `_e` file: one quad, as vanilla.
        let plain = draw(&data(&["assets/minecraft/textures/entity/silverfish.png"]));
        assert_eq!(plain.len(), 4);
        assert!(plain.iter().all(|v| v.light == lit));
        let glowing = draw(&data(&[
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_e.png",
        ]));
        assert_eq!(glowing.len(), 8);
        let (body, glow) = glowing.split_at(4);
        assert!(body.iter().all(|v| v.light == lit));
        assert!(glow.iter().all(|v| v.light == FULL_BRIGHT));
        // The overlay samples its own sprite, and keeps the body's shading normals.
        assert!(glow.iter().all(|v| v.uv[0] >= 0.5 && v.uv[1] >= 0.5));
        assert!(body.iter().all(|v| v.uv[0] <= 0.5 && v.uv[1] <= 0.5));
        assert_eq!(
            glow.iter().map(|v| v.normal).collect::<Vec<_>>(),
            body.iter().map(|v| v.normal).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_blink_frame_replaces_the_texture_for_a_few_ticks() {
        let data = data(&[
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_blink.png",
        ]);
        let uvs = |age: u32| {
            let (mut m, models) = silverfish();
            m.age = age;
            let source = ClientMobRender {
                models: &models,
                data: &data,
            };
            let mut out = Vec::new();
            render_with(
                &m,
                0.0,
                glam::DVec3::ZERO,
                &atlas,
                0,
                &source,
                &[],
                &mut out,
            );
            out.iter().map(|v| v.uv).collect::<Vec<_>>()
        };
        // Entity 1's phase is 0x9E3779B1 % 150 = 61: of every 150 ticks, it blinks for the
        // default 5 from age 89 (89 + 61 = 150).
        let shut = |uv: &Vec<[f32; 2]>| uv.iter().all(|uv| uv[0] <= 0.5 && uv[1] >= 0.5);
        let open = |uv: &Vec<[f32; 2]>| uv.iter().all(|uv| uv[0] <= 0.5 && uv[1] <= 0.5);
        assert!(open(&uvs(88)));
        assert!(shut(&uvs(89)));
        assert!(shut(&uvs(93)));
        assert!(open(&uvs(94)));
        assert!(shut(&uvs(89 + 150)));
    }

    #[test]
    fn a_blink_shows_its_own_glow_or_none() {
        let atlas = |name: &str| {
            let u = match name {
                "entity/silverfish" => 0.0,
                "entity/silverfish_e" => 0.25,
                "entity/silverfish_blink" => 0.5,
                "entity/silverfish_blink_e" => 0.75,
                _ => return None,
            };
            Some([u, 0.0, u + 0.25, 1.0])
        };
        let base = [
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_e.png",
            "assets/minecraft/textures/entity/silverfish_blink.png",
        ];
        let at = |data: &ClientMobData, age: u32| {
            let (mut m, models) = silverfish();
            m.age = age;
            drawn(&m, &models, data, &atlas)
        };
        // Age 89 is shut (see above), 0 open.
        let plain = data(&base);
        let open = at(&plain, 0);
        assert_eq!(open.len(), 8);
        assert!(inside(&open[4..], 0.25), "the texture's glow");
        let shut = at(&plain, 89);
        assert_eq!(shut.len(), 4, "no glowing open eyes over the shut ones");
        assert!(inside(&shut, 0.5));
        let mut own = base.to_vec();
        own.push("assets/minecraft/textures/entity/silverfish_blink_e.png");
        let shut = at(&data(&own), 89);
        assert_eq!(shut.len(), 8);
        assert!(inside(&shut[4..], 0.75), "the frame's own glow");
    }

    /// Draws `m` and returns each vertex's (uv, light).
    fn drawn(
        m: &Mob,
        models: &ClientMobModels,
        data: &ClientMobData,
        atlas: Atlas,
    ) -> Vec<([f32; 2], u32)> {
        let source = ClientMobRender { models, data };
        let mut out = Vec::new();
        render_with(m, 0.0, glam::DVec3::ZERO, atlas, 7, &source, &[], &mut out);
        out.iter().map(|v| (v.uv, v.light)).collect()
    }

    /// Whether every vertex samples inside the rect starting at `at` (each rect is 0.25 wide).
    fn inside(vertices: &[([f32; 2], u32)], at: f32) -> bool {
        vertices
            .iter()
            .all(|(uv, _)| (at..=at + 0.25).contains(&uv[0]))
    }

    #[test]
    fn a_layer_glows_with_its_own_overlay() {
        let (mut m, mut models) = silverfish();
        let k = Arc::make_mut(&mut m.kind);
        k.species = Species::Sheep;
        let visuals = models.types.get_mut(&k.id).unwrap();
        visuals
            .layers
            .insert("sheep#wool".into(), visuals.model.clone());
        let data = data(&[
            "assets/minecraft/textures/entity/sheep/sheep.png",
            "assets/minecraft/textures/entity/sheep/sheep_wool.png",
            "assets/minecraft/textures/entity/sheep/sheep_wool_e.png",
        ]);
        let atlas = |name: &str| {
            let u = match name {
                "entity/sheep/sheep" => 0.0,
                "entity/sheep/sheep_wool" => 0.25,
                "entity/sheep/sheep_wool_e" => 0.5,
                _ => return None,
            };
            Some([u, 0.0, u + 0.25, 1.0])
        };
        let v = drawn(&m, &models, &data, &atlas);
        assert_eq!(v.len(), 12, "body, wool, wool's glow");
        assert!(inside(&v[..4], 0.0) && inside(&v[4..8], 0.25) && inside(&v[8..], 0.5));
        assert!(v[..8].iter().all(|(_, light)| *light == 7));
        assert!(v[8..].iter().all(|(_, light)| *light == FULL_BRIGHT));
    }

    #[test]
    fn a_random_variant_glows_with_its_own_overlay_only() {
        const RANDOM: &str = "assets/minecraft/optifine/random/entity";
        let atlas = |name: &str| {
            let u = match name {
                "entity/silverfish" => 0.0,
                "entity/silverfish_e" => 0.25,
                "minecraft:optifine/random/entity/silverfish2" => 0.5,
                "minecraft:optifine/random/entity/silverfish2_e" => 0.75,
                _ => return None,
            };
            Some([u, 0.0, u + 0.25, 1.0])
        };
        let seen = || crate::mobs::random_textures::Seen {
            biome: None,
            y: 64,
            blocks: Vec::new(),
            moon_phase: 0,
            day_time: 0,
            weather: sunrise_optifine::random::Weather::Clear,
        };
        let props = format!("{RANDOM}/silverfish.properties");
        let variant = format!("{RANDOM}/silverfish2.png");
        let variant_e = format!("{RANDOM}/silverfish2_e.png");
        let base = [
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_e.png",
        ];
        for own_overlay in [true, false] {
            let mut files = vec![base[0], base[1], props.as_str(), variant.as_str()];
            if own_overlay {
                files.push(variant_e.as_str());
            }
            let data = data(&files);
            let (mut m, models) = silverfish();
            let source = ClientMobRender {
                models: &models,
                data: &data,
            };
            {
                let textures = data.random_textures.read().unwrap();
                let names = drawn_textures(&m, &source);
                crate::mobs::random_textures::choose(&mut m, &textures, names, Some(seen()), 0);
            }
            let v = drawn(&m, &models, &data, &atlas);
            assert!(inside(&v[..4], 0.5), "the variant");
            if own_overlay {
                assert_eq!(v.len(), 8);
                assert!(inside(&v[4..], 0.75), "its own overlay, not silverfish_e");
            } else {
                assert_eq!(v.len(), 4, "silverfish_e doesn't glow over silverfish2");
            }
        }
    }

    #[test]
    fn worn_armour_glows_when_its_extra_layer_says_so() {
        let (m, models) = silverfish();
        let data = data(&[]);
        let source = ClientMobRender {
            models: &models,
            data: &data,
        };
        let atlas = |_: &str| Some([0.0, 0.0, 0.5, 0.5]);
        let part = models.model(&m.kind.id).unwrap().clone();
        let extra = [
            (
                WornModel::Vanilla(&part),
                [0.5, 0.0, 1.0, 0.5],
                0xFFFF_FFFF,
                0,
                false,
            ),
            (
                WornModel::Vanilla(&part),
                [0.5, 0.5, 1.0, 1.0],
                0xFFFF_FFFF,
                0,
                true,
            ),
        ];
        let mut out = Vec::new();
        render_with(
            &m,
            0.0,
            glam::DVec3::ZERO,
            &atlas,
            7,
            &source,
            &extra,
            &mut out,
        );
        assert_eq!(out.len(), 12, "body, armour, armour's glow");
        assert!(out[..8].iter().all(|v| v.light == 7));
        assert!(out[8..].iter().all(|v| v.light == FULL_BRIGHT));
    }
}

#[cfg(test)]
mod random_texture_tests {
    use super::*;
    use crate::mobs::{Polygon, Species};
    use std::sync::{Arc, RwLock};
    use sunrise_optifine::random_textures::RandomTextures;

    /// A silverfish (drawn with its type's texture) whose model is one quad.
    fn silverfish() -> (Mob, ClientMobModels) {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.species = Species::Silverfish;
        let mut models = ClientMobModels::default();
        models.insert(
            k.id.clone(),
            ClientMobVisuals {
                shadow: None,
                model: Arc::new(Part {
                    name: "root".into(),
                    pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    polygons: vec![Polygon {
                        normal: [0.0, 0.0, -1.0],
                        cull_back: false,
                        vertices: [
                            [0.0, 0.0, 0.0, 0.0, 0.0],
                            [1.0, 0.0, 0.0, 1.0, 0.0],
                            [1.0, 1.0, 0.0, 1.0, 1.0],
                            [0.0, 1.0, 0.0, 0.0, 1.0],
                        ],
                    }],
                    children: Vec::new(),
                    layer: None,
                }),
                layers: Default::default(),
                texture: "entity/silverfish".into(),
            },
        );
        (
            Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0),
            models,
        )
    }

    /// A pack whose silverfish has a second variant, every silverfish wearing it.
    fn data() -> ClientMobData {
        let root =
            std::env::temp_dir().join(format!("sunrise-render-random-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("assets/minecraft/optifine/random/entity");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("silverfish.properties"), "textures.1=2").unwrap();
        std::fs::write(dir.join("silverfish2.png"), "").unwrap();
        let random = RandomTextures::scan(
            &sunrise_assets::resource_source::ResourceSource::directory(root.to_owned()),
        );
        let _ = std::fs::remove_dir_all(&root);
        ClientMobData {
            random_textures: RwLock::new(random),
            ..Default::default()
        }
    }

    fn uvs(m: &Mob, models: &ClientMobModels, data: &ClientMobData, atlas: Atlas) -> Vec<[f32; 2]> {
        let source = ClientMobRender { models, data };
        let mut out = Vec::new();
        render_with(m, 0.0, glam::DVec3::ZERO, atlas, 0, &source, &[], &mut out);
        out.iter().map(|v| v.uv).collect()
    }

    #[test]
    fn a_mob_draws_its_variant_and_the_texture_when_the_atlas_lacks_it() {
        let data = data();
        let (mut m, models) = silverfish();
        let source = ClientMobRender {
            models: &models,
            data: &data,
        };
        {
            let textures = data.random_textures.read().unwrap();
            let seen = crate::mobs::random_textures::Seen {
                biome: None,
                y: 64,
                blocks: Vec::new(),
                moon_phase: 0,
                day_time: 0,
                weather: sunrise_optifine::random::Weather::Clear,
            };
            let names = drawn_textures(&m, &source);
            crate::mobs::random_textures::choose(&mut m, &textures, names, Some(seen), 0);
        }
        let both = |name: &str| match name {
            "entity/silverfish" => Some([0.0, 0.0, 0.5, 0.5]),
            "minecraft:optifine/random/entity/silverfish2" => Some([0.5, 0.5, 1.0, 1.0]),
            _ => None,
        };
        let drawn = uvs(&m, &models, &data, &both);
        assert_eq!(drawn.len(), 4);
        assert!(drawn.iter().all(|uv| uv[0] >= 0.5 && uv[1] >= 0.5));
        let vanilla_only =
            |name: &str| (name == "entity/silverfish").then_some([0.0, 0.0, 0.5, 0.5]);
        let drawn = uvs(&m, &models, &data, &vanilla_only);
        assert_eq!(drawn.len(), 4, "still drawn");
        assert!(drawn.iter().all(|uv| uv[0] <= 0.5 && uv[1] <= 0.5));
    }

    /// `EndermanRenderer.getRenderOffset` moves a creepy enderman by
    /// `0.02 * state.scale` blocks sideways: at `minecraft:scale` 2 the same
    /// gaussian offset doubles, and at the default it is the old 0.02.
    #[test]
    fn a_scaled_creepy_endermans_render_offset_scales() {
        use sunrise_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:enderman".into();
        kind.species = Species::Enderman;
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        // Enderman.isCreepy: the angry flag that adds the offset.
        m.extra.flag = true;
        m.extra.model_scale = 1.0;
        let one = enderman_render_offset(&m, 0.5);
        assert!(one.length() > 0.0, "the offset is the renderer's gaussian");
        m.extra.model_scale = 2.0;
        let two = enderman_render_offset(&m, 0.5);
        assert!((two.x - 2.0 * one.x).abs() < 1.0e-12, "{two:?} vs {one:?}");
        assert!((two.z - 2.0 * one.z).abs() < 1.0e-12, "{two:?} vs {one:?}");
        assert_eq!(two.y, 0.0);
    }

    /// `LivingEntityRenderer.submit` translates a sleeper toward its pillow
    /// before `poseStack.scale(state.scale)`: the bed offset is not scaled and
    /// the pose after it is. The uniform scale after `setupRotations` instead
    /// would leave the pose's own translations unscaled; the one before the
    /// sleeping translation would scale the bed.
    #[test]
    fn a_scaled_sleepers_bed_offset_is_not_scaled() {
        use crate::mobs::Species;
        use sunrise_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:villager".into();
        kind.species = Species::Villager;
        (kind.width, kind.height, kind.eye_height) = (0.6, 1.95, 1.62);
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        // Villager.sleeping: the bed faces north, so the head goes to +z.
        m.extra.ints[1] = 1;
        // The eye height of the standing pose, as `submit` extracts it.
        let bed = |m: &Mob| {
            let eye = m.awake_dims().2 as f32;
            glam::Vec3::new(0.0, 0.0, eye - 0.1)
        };
        let unscaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let unscaled_pose = glam::Mat4::from_translation(-bed(&m)) * unscaled;
        m.extra.model_scale = 2.0;
        let scaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let scaled_pose = glam::Mat4::from_translation(-bed(&m)) * scaled;
        let expected = glam::Mat4::from_scale(glam::Vec3::splat(2.0)) * unscaled_pose;
        let got = scaled_pose.to_cols_array();
        let want = expected.to_cols_array();
        for i in 0..16 {
            assert!(
                (got[i] - want[i]).abs() < 1.0e-4,
                "element {i}: {} != {}\nscaled: {got:?}\nunscaled x2: {want:?}",
                got[i],
                want[i]
            );
        }
        m.custom_name = Some("Dinnerbone".into());
        assert_eq!(base_transform_at(&m, 0.5, glam::Mat4::IDENTITY), scaled);
    }

    /// A sleeping avatar's bed offset is `getEyeHeight(Pose.STANDING) - 0.1`,
    /// and the standing eye height carries `minecraft:scale`: at scale 2 the
    /// bed offset is 3.14, not the type's 1.52, and the pose after it doubles.
    #[test]
    fn a_scaled_sleeping_avatar_uses_its_scaled_eye_height() {
        use sunrise_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:player".into();
        (kind.width, kind.height, kind.eye_height) = (0.6, 1.8, 1.62);
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.ints[1] = 1;
        let bed = |m: &Mob| {
            let eye = m.kind.eye_height as f32 * m.model_scale();
            glam::Vec3::new(0.0, 0.0, eye - 0.1)
        };
        let unscaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let unscaled_pose = glam::Mat4::from_translation(-bed(&m)) * unscaled;
        m.extra.model_scale = 2.0;
        let scaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let scaled_pose = glam::Mat4::from_translation(-bed(&m)) * scaled;
        let expected = glam::Mat4::from_scale(glam::Vec3::splat(2.0)) * unscaled_pose;
        let got = scaled_pose.to_cols_array();
        let want = expected.to_cols_array();
        for i in 0..16 {
            assert!(
                (got[i] - want[i]).abs() < 1.0e-4,
                "element {i}: {} != {}\nscaled: {got:?}\nunscaled x2: {want:?}",
                got[i],
                want[i]
            );
        }
    }

    /// The avatar's 0.9375 `AvatarRenderer.scale` and the entity's own
    /// `minecraft:scale` are two factors: at scale 2 the model is 1.875 times
    /// the model's, not 3.75, so the submitted scale is not applied twice.
    #[test]
    fn an_avatar_at_scale_two_scales_its_model_once() {
        use sunrise_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:player".into();
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.model_scale = 2.0;
        let transform = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
            let length = transform.transform_vector3(axis).length();
            assert!(
                (length - 1.875).abs() < 1.0e-5,
                "{axis:?} scales by {length}, not 2 * 0.9375"
            );
        }
    }
}

#[cfg(test)]
mod translate_to_hand_tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    /// A zombie whose model is a root with two posed arms.
    fn armed() -> (Mob, Part) {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        Arc::make_mut(&mut kind).species = Species::Zombie;
        let model = Part::parse(
            "root",
            &json!({"children": {
                "right_arm": {"pose": [-5.0, 2.0, 0.0, -0.4, 0.1, 0.05, 1.0, 1.0, 1.0]},
                "left_arm": {"pose": [5.0, 2.0, 0.0, 0.3, -0.1, -0.05, 1.0, 1.0, 1.0]}
            }}),
        );
        (
            Mob::new(1, kind, sunrise_core::geometry::Vec3::ZERO, 0.0),
            model,
        )
    }

    /// `ItemInHandLayer.submitArmWithItem`: `translateToHand`, then the two item rotations
    /// and the hand offset. The hand `hand_base` places an item in is the one
    /// `translate_to_hand` returns, with only those three steps after it, so a mixin that
    /// turns the returned matrix turns the item before vanilla's rotations (#3970).
    #[test]
    fn the_item_rotations_and_hand_offset_come_after_translate_to_hand() {
        let (m, model) = armed();
        let poses = Poses::new(&model);
        let base = glam::Mat4::from_translation(glam::Vec3::new(0.25, 1.5, -0.75))
            * glam::Mat4::from_rotation_y(0.7);
        for left in [false, true] {
            let hand = translate_to_hand(&m, &model, base, &poses, left).unwrap();
            let arm = if left { "left_arm" } else { "right_arm" };
            // A zombie's `translateToHand` is its arm's own `translateAndRotate`.
            assert_eq!(hand, part_matrix(&model, base, &poses, arm, 0.0).unwrap());
            let expected = hand
                * (glam::Mat4::from_rotation_x((-90.0f32).to_radians())
                    * glam::Mat4::from_rotation_y(PI))
                * glam::Mat4::from_translation(glam::Vec3::new(
                    if left { -1.0 } else { 1.0 } / 16.0,
                    2.0 / 16.0,
                    -10.0 / 16.0,
                ));
            assert_eq!(
                hand_base(&m, &model, base, &poses, &[], left).unwrap(),
                expected
            );
        }
    }

    /// `held_item_transform`, the held item's pose the renderer draws, goes through
    /// `translate_to_hand`: it is exactly the hand that function returns for the posed model,
    /// then the item rotations and hand offset. If the renderer stopped calling it, a row on
    /// `translate_to_hand` would no longer move the item, and this test would fail.
    #[test]
    fn the_held_item_is_placed_from_translate_to_hand() {
        let (m, model) = armed();
        let mut models = ClientMobModels::default();
        models.insert(
            m.kind.id.clone(),
            ClientMobVisuals {
                shadow: None,
                model: Arc::new(model),
                layers: Default::default(),
                texture: String::new(),
            },
        );
        let client_data = ClientMobData::default();
        let data = ClientMobRender {
            models: &models,
            data: &client_data,
        };
        let (alpha, camera) = (0.5, glam::DVec3::new(-2.0, 1.0, 3.0));
        let (posed, poses, _) = posed_body(&m, alpha, &data).unwrap();
        let base = base_transform(&m, alpha, camera);
        for left in [false, true] {
            let hand = translate_to_hand(&m, &posed, base, &poses, left).unwrap();
            assert_eq!(
                held_item_transform(&m, alpha, camera, &data, left).unwrap(),
                item_from_hand(hand, &m, left)
            );
        }
    }

    /// What a `RETURN` row on `translate_to_hand` does to the item: the turn it multiplies
    /// into the hand comes before vanilla's item rotations, as a Java mixin's
    /// `poseStack.rotate` at `submitArmWithItem`'s first `rotateDegrees` does
    /// (BendableCuboids' forearm: about X, 0.25 below the arm's pivot). Turning after the
    /// rotations instead puts the item elsewhere for this pose.
    #[test]
    fn a_turn_of_the_hand_comes_before_the_item_rotations() {
        let (m, model) = armed();
        let poses = Poses::new(&model);
        let base = glam::Mat4::from_rotation_y(0.7);
        let elbow = glam::Vec3::new(0.0, 0.25, 0.0);
        let turn = glam::Mat4::from_translation(elbow)
            * glam::Mat4::from_rotation_x(0.5)
            * glam::Mat4::from_translation(-elbow);
        for left in [false, true] {
            let hand = translate_to_hand(&m, &model, base, &poses, left).unwrap();
            let turned = item_from_hand(hand * turn, &m, left);
            // The Java pose stack: translateToHand, the mixin's turn, the two item rotations,
            // the hand offset.
            let java = hand
                * turn
                * (glam::Mat4::from_rotation_x((-90.0f32).to_radians())
                    * glam::Mat4::from_rotation_y(PI))
                * glam::Mat4::from_translation(glam::Vec3::new(
                    if left { -1.0 } else { 1.0 } / 16.0,
                    2.0 / 16.0,
                    -10.0 / 16.0,
                ));
            assert_eq!(turned, java);
            let after = item_from_hand(hand, &m, left) * turn;
            assert!(
                !turned.abs_diff_eq(after, 1.0e-3),
                "a turn before the item rotations differs from one after them"
            );
        }
    }

    /// The skeleton family's `translateToHand` shifts the arm by 1 (outwards on each side).
    #[test]
    fn a_skeleton_hand_is_its_arm_shifted_by_one() {
        let (mut m, model) = armed();
        Arc::make_mut(&mut m.kind).species = Species::Skeleton;
        let poses = Poses::new(&model);
        let base = glam::Mat4::IDENTITY;
        for (left, arm, shift) in [(false, "right_arm", 1.0), (true, "left_arm", -1.0)] {
            assert_eq!(
                translate_to_hand(&m, &model, base, &poses, left).unwrap(),
                part_matrix(&model, base, &poses, arm, shift).unwrap()
            );
        }
    }
}
#[cfg(test)]
mod compile_part_tests {
    use super::*;
    use serde_json::json;

    /// A humanoid layer's tree: five parts of one quad each (facing +x, their texture corners
    /// apart), drawn body, head, left_arm, right_arm, right_leg (children in name order), so
    /// `right_arm`'s four vertices are the 13th to the 16th.
    fn humanoid_tree() -> serde_json::Value {
        let part = |name: &str, u: f32, pose: [f32; 9]| {
            let v = json!({
                "pose": pose,
                "polygons": [[[0, 0, 0, u, 0.25], [0, 12, 0, u, 0.35],
                              [0, 12, 4, u + 0.1, 0.35], [0, 0, 4, u + 0.1, 0.25]]],
                "normals": [[1, 0, 0]]
            });
            (name.to_owned(), v)
        };
        let children: serde_json::Map<_, _> = [
            part("body", 0.0, [0.0, 0.0, 0.0, 0.1, 0.0, 0.0, 1.0, 1.0, 1.0]),
            part("head", 0.1, [0.0, -2.0, 0.0, 0.0, 0.3, 0.0, 1.0, 1.0, 1.0]),
            part(
                "left_arm",
                0.2,
                [5.0, 2.0, 0.0, 0.3, -0.1, 0.0, 1.0, 1.0, 1.0],
            ),
            part(
                "right_arm",
                0.3,
                [-5.0, 2.0, 0.0, -0.4, 0.1, 0.05, 1.0, 1.0, 1.0],
            ),
            part(
                "right_leg",
                0.4,
                [-2.0, 12.0, 0.0, 0.2, 0.0, 0.0, 1.0, 1.25, 1.0],
            ),
        ]
        .into_iter()
        .collect();
        json!({ "children": children })
    }

    /// The player layer on [`humanoid_tree`].
    fn humanoid() -> Part {
        Part::parse_layer("minecraft:player#main", &humanoid_tree())
    }

    fn paint() -> Paint {
        Paint {
            uv: [0.125, 0.25, 0.75, 0.875],
            scroll: 0.0,
            color: 0xff80_c0ff,
            light: 0x70a0,
            overlay: 0,
        }
    }

    fn drawn(model: &Part, poses: &Poses, compile: Option<CompilePart>) -> Vec<Vertex> {
        let base = glam::Mat4::from_translation(glam::Vec3::new(0.25, 1.5, -0.75))
            * glam::Mat4::from_rotation_y(0.7);
        let mut out = Vec::new();
        draw_normals(
            model,
            base,
            ModelNormals::new(base),
            poses,
            paint(),
            model.layer.as_deref(),
            compile,
            &mut Quads::new(&mut out),
        );
        out
    }

    fn bytes(vertices: &[Vertex]) -> &[u8] {
        bytemuck::cast_slice(vertices)
    }

    /// `draw_normals` as it was before `compile_part` was split out of it: the reference
    /// for "with no replacement, a model draws as before".
    fn before_split(
        part: &Part,
        base: glam::Mat4,
        normals: ModelNormals,
        poses: &Poses,
        paint: Paint,
        out: &mut Vec<Vertex>,
    ) {
        let Paint {
            uv,
            scroll,
            color,
            light,
            overlay,
        } = paint;
        let p = poses.pose(part);
        let m = posed(base, p);
        if poses.is_hidden(part) || p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
            return;
        }
        let normals = normals.part(p);
        if !poses.skips(part) {
            for poly in &part.polygons {
                let normal = poly.vertex_normal(normals);
                for v in poly.vertices {
                    let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                    let glint_uv = if overlay == ARMOR_GLINT {
                        [v[3], v[4]]
                    } else {
                        [0.0, 0.0]
                    };
                    out.push(Vertex {
                        normal,
                        pos: pos.to_array(),
                        uv: [
                            uv[0] + (uv[2] - uv[0]) * scroll_u(v[3], scroll),
                            uv[1] + (uv[3] - uv[1]) * v[4],
                        ],
                        layer: overlay,
                        color,
                        light,
                        glint_uv,
                    });
                }
            }
        }
        for c in &part.children {
            before_split(c, m, normals, poses, paint, out);
        }
    }

    /// Without a replacement, `draw_scrolled` pushes the bytes the loop before the split
    /// pushed, through a nonuniform scale, a skipped part, a hidden one, a scroll and the
    /// armour glint; `compile_part`'s own body pushes those bytes too; and `render_model`,
    /// which now emits through the same step, pushes what it pushed before.
    #[test]
    fn without_a_replacement_a_model_draws_as_before_the_split() {
        let model = humanoid();
        let mut poses = Poses::new(&model);
        poses.get("right_arm")[6] = 0.5;
        poses.skip_draw("head");
        poses.visible("left_arm", false);
        let base = glam::Mat4::from_translation(glam::Vec3::new(0.25, 1.5, -0.75))
            * glam::Mat4::from_rotation_y(0.7)
            * glam::Mat4::from_scale(glam::Vec3::new(0.9375, 0.9375, -0.9375));
        for (scroll, overlay) in [(0.0, 0), (0.375, 0), (0.0, ARMOR_GLINT)] {
            let p = Paint {
                scroll,
                overlay,
                ..paint()
            };
            let mut expected = Vec::new();
            before_split(
                &model,
                base,
                ModelNormals::new(base),
                &poses,
                p,
                &mut expected,
            );
            let mut now = Vec::new();
            draw_scrolled(
                &model, base, &poses, p.uv, p.scroll, p.color, p.light, p.overlay, &mut now,
            );
            assert_eq!(expected.len(), 12);
            assert_eq!(bytes(&now), bytes(&expected));
            let mut through = Vec::new();
            draw_normals(
                &model,
                base,
                ModelNormals::new(base),
                &poses,
                p,
                model.layer.as_deref(),
                Some(compile_part),
                &mut Quads::new(&mut through),
            );
            assert_eq!(bytes(&through), bytes(&expected));
        }
        // `render_model` draws every part at its own pose, white, unscrolled.
        let plain = Poses::new(&model);
        for overlay in [0, ARMOR_GLINT] {
            let p = Paint {
                color: 0xFFFF_FFFF,
                overlay,
                ..paint()
            };
            let mut expected = Vec::new();
            before_split(
                &model,
                base,
                ModelNormals::new(base),
                &plain,
                p,
                &mut expected,
            );
            let mut now = Vec::new();
            crate::mobs::render_model(
                &model,
                base,
                &|part| part.pose,
                p.uv,
                p.light,
                overlay,
                &mut now,
            );
            assert_eq!(expected.len(), 20);
            assert_eq!(bytes(&now), bytes(&expected));
        }
    }

    /// The `lower` fn-replace-check fixture's crate, as its helpers name it: its own `Bend`
    /// type, and `note`.
    mod fnr_lower {
        include!("../../../../tools/ci/fn-replace-check/lower/src/shared.rs");
        pub fn note(_: &str) {}
    }
    // The fixture's `pose`, `bend` and `takes` helpers, compiled here in `render`'s scope as the
    // installer compiles them into its copy of this module.
    include!("../../../../tools/ci/fn-replace-check/lower/src/mixins/pose.rs");
    include!("../../../../tools/ci/fn-replace-check/lower/src/mixins/bend.rs");
    include!("../../../../tools/ci/fn-replace-check/lower/src/mixins/takes.rs");

    /// `anim::setup` with the fixture's `pose` RETURN row.
    fn setup_installed(
        m: &Mob,
        s: &anim::State<'_>,
        p: &mut Poses,
        animations: &std::collections::HashMap<String, super::super::keyframes::Animation>,
    ) {
        // The row hands the helper the game's return value: `()` here.
        anim::setup(m, s, p, animations);
        pose((), m, p)
    }
    /// `compile_part` with the fixture's cancellable `bend` HEAD row.
    fn compile_part_installed(
        layer: Option<&str>,
        part: &Part,
        matrix: glam::Mat4,
        normals: ModelNormals,
        paint: Paint,
        poses: &Poses,
        quads: &mut Quads<'_>,
    ) {
        if bend(layer, part, matrix, normals, paint, poses, quads).is_break() {
            return;
        }
        compile_part(layer, part, matrix, normals, paint, poses, quads);
    }
    /// `takes_part` with the fixture's `takes` RETURN row.
    fn takes_part_installed(layer: &str, part: &str) -> bool {
        takes(takes_part(layer, part), layer, part)
    }

    /// A zombie-animated mob drawn with `model`.
    fn mob(species: Species, model: Part, layers: &[(&str, Part)]) -> (Mob, ClientMobModels) {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = species;
        kind.anim = super::super::Anim::Zombie;
        let mut models = ClientMobModels::default();
        models.insert(
            kind.id.clone(),
            ClientMobVisuals {
                shadow: None,
                model: std::sync::Arc::new(model),
                layers: layers
                    .iter()
                    .map(|(key, part)| ((*key).to_owned(), std::sync::Arc::new(part.clone())))
                    .collect(),
                texture: String::new(),
            },
        );
        let mut mob = Mob::new(
            1,
            std::sync::Arc::new(kind),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        mob.age = 37;
        (mob, models)
    }

    /// The fixture's rows from the pose write to the emitted quads: its `pose` row stores the
    /// bend after the game's `setupAnim`, and its `bend` row draws `right_arm` of the player
    /// layer as two segments with the lower one turned by the stored angle at the elbow; every
    /// other part's bytes, and the arm's texture, colour, light and overlay, stay the game's.
    /// Its `takes` row takes that arm, so the player model leaves the GPU and a zombie's stays.
    #[test]
    fn the_lower_fixture_bends_the_right_forearm_from_pose_write_to_quads() {
        let model = humanoid();
        let (m, _) = mob(Species::Zombie, model.clone(), &[]);
        let state = anim::state(&m, 0.5);
        let mut poses = Poses::new(&model);
        setup_installed(&m, &state, &mut poses, &Default::default());
        let mut game = Poses::new(&model);
        anim::setup(&m, &state, &mut game, &Default::default());
        let plain = drawn(&model, &game, None);
        assert_eq!(plain.len(), 20);
        assert_eq!(
            bytes(&drawn(&model, &poses, None)),
            bytes(&plain),
            "the stored bend changes no pose"
        );
        let bent = drawn(&model, &poses, Some(compile_part_installed));
        assert_eq!(bent.len(), 24);
        assert_eq!(
            bytes(&bent[..12]),
            bytes(&plain[..12]),
            "body, head, left_arm"
        );
        assert_eq!(bytes(&bent[20..]), bytes(&plain[16..]), "right_leg");
        let arm = &plain[12..16];
        for segment in [&bent[12..16], &bent[16..20]] {
            for (v, a) in segment.iter().zip(arm) {
                assert_eq!(
                    (v.uv, v.color, v.light, v.layer, v.normal),
                    (a.uv, a.color, a.light, a.layer, a.normal)
                );
            }
        }
        // The quad's first two corners run down the arm, 12 pixels. The upper segment starts
        // at the shoulder and runs 6 of them; the lower one starts where it ends and runs 6
        // more, turned 0.8 radians away from it.
        let pos = |v: &Vertex| glam::Vec3::from_array(v.pos);
        let (upper, lower) = (&bent[12..16], &bent[16..20]);
        assert_eq!(upper[0].pos, arm[0].pos);
        let along = pos(&upper[1]) - pos(&upper[0]);
        let forearm = pos(&lower[1]) - pos(&lower[0]);
        assert!(pos(&lower[0]).distance(pos(&upper[1])) < 1e-6, "the elbow");
        assert!((along.length() - 6.0 / 16.0).abs() < 1e-6);
        assert!((forearm.length() - 6.0 / 16.0).abs() < 1e-6);
        let turned = along.angle_between(forearm);
        assert!((turned - 0.8).abs() < 1e-4, "turned by {turned}");
        // A model of another layer keeps its arm, bend stored or not.
        let mut zombie = model.clone();
        zombie.layer = Some("minecraft:zombie#main".into());
        assert_eq!(
            bytes(&drawn(&zombie, &poses, Some(compile_part_installed))),
            bytes(&drawn(&zombie, &game, None))
        );
        // The `takes` row: the player model is drawn on the CPU, the zombie's on the GPU.
        let mut models = super::super::skin::SkinModels::default();
        models.takes = Some(takes_part_installed);
        let mut frame = crate::gfx::entity_skin::SkinFrame::default();
        let mut skin = super::super::skin::Skin::new(&mut models, &mut frame);
        use super::super::skin::Route;
        assert_eq!(skin.route(&std::sync::Arc::new(model)), Route::Taken);
        assert_eq!(skin.route(&std::sync::Arc::new(zombie)), Route::Gpu);
    }

    /// The ordinary pass submits Pre, body and layer in order even when a draw is taken.
    /// Placement is decided once per model and kept across frames.
    #[test]
    fn a_taken_model_leaves_the_gpu_and_the_mobs_other_models_keep_their_order() {
        thread_local! {
            static ASKED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        fn none(_: &str, _: &str) -> bool {
            ASKED.with(|asked| asked.set(asked.get() + 1));
            false
        }
        fn body_arm(layer: &str, part: &str) -> bool {
            ASKED.with(|asked| asked.set(asked.get() + 1));
            layer == "minecraft:drowned#main" && part == "right_arm"
        }
        fn outer_arm(layer: &str, part: &str) -> bool {
            ASKED.with(|asked| asked.set(asked.get() + 1));
            layer == "minecraft:drowned#outer" && part == "right_arm"
        }
        let main = Part::parse_layer("minecraft:drowned#main", &humanoid_tree());
        let outer = Part::parse_layer("minecraft:drowned#outer", &humanoid_tree());
        let (m, mob_models) = mob(
            Species::Drowned,
            main.clone(),
            &[("drowned#main", main), ("drowned#outer", outer)],
        );
        let client_data = ClientMobData {
            frame_models: BakedFrameModels::bake(mob_models.types.values()),
            ..Default::default()
        };
        let data = ClientMobRender {
            models: &mob_models,
            data: &client_data,
        };
        // The outer layer's sprite apart from the body's, so the two draws differ.
        let atlas = |texture: &str| {
            Some(if texture.contains("outer") {
                [0.5, 0.0, 0.75, 0.5]
            } else {
                [0.0, 0.0, 0.25, 0.5]
            })
        };
        // The whole mob on the CPU: the body's 20 vertices, then the outer layer's 20.
        let mut cpu = Vec::new();
        render_with_frame(
            &m,
            0.5,
            glam::DVec3::ZERO,
            &atlas,
            0xf0f0,
            &data,
            &[],
            &mut FrameDraw::new(false),
            &mut cpu,
        );
        assert_eq!(cpu.len(), 40);
        let (body, layer) = cpu.split_at(20);
        assert_ne!(bytes(body), bytes(layer), "two draws apart");
        // Simulate a Pre listener submitting an ordinary quad before either model, as the
        // game marks the callback's output on the prepared frame draw.
        let pre = body.first_chunk::<4>().unwrap().map(|v| Vertex {
            uv: [v.uv[0] * 0.5 + 0.875, v.uv[1]],
            ..v
        });
        let drawn_by = |u: f32| {
            if u >= 0.875 {
                "pre"
            } else if u < 0.5 {
                "body"
            } else {
                "layer"
            }
        };
        for (takes, cpu, lanes, asked) in [
            (none as fn(&str, &str) -> bool, &[][..], ["GPU", "GPU"], 12),
            (body_arm, body, ["CPU", "GPU"], 11),
            (outer_arm, layer, ["GPU", "CPU"], 11),
        ] {
            ASKED.with(|a| a.set(0));
            let mut models = super::super::skin::SkinModels::default();
            models.takes = Some(takes);
            let mut frame = crate::gfx::entity_skin::SkinFrame::default();
            for prefix in [&[][..], &pre[..]] {
                frame.clear_draws();
                let mut frame_draw = FrameDraw::new(false);
                let mut out = Vec::new();
                if !prefix.is_empty() {
                    assert!(frame_draw.prepare(&m, &client_data, &mob_models).is_some());
                    out.extend_from_slice(prefix);
                    frame_draw.mark_pre_vertices(0);
                }
                let mut skin = super::super::skin::Skin::new(&mut models, &mut frame);
                render_skinned(
                    &m,
                    0.5,
                    glam::DVec3::ZERO,
                    &atlas,
                    0xf0f0,
                    &data,
                    &mut frame_draw,
                    &mut out,
                    &mut skin,
                );
                assert_eq!(bytes(&out[..prefix.len()]), bytes(prefix));
                assert_eq!(bytes(&out[prefix.len()..]), bytes(cpu));
                // Both models use ordinary quads; the entity batch leaves this CPU stream as-is.
                // Use the production upload and skin job destinations. The ordinary pass's
                // indices visit these buffer positions in order.
                let mut order = crate::gfx::entity_skin::OrdinaryOrder::default();
                order.prepare(&out, Some(&frame), false);
                let mut submitted = vec![("", ""); prefix.len() + 40];
                for (dst, range) in order.cpu_uploads(out.len() as u32) {
                    let src = &out[range.start as usize..range.end as usize];
                    for (slot, vertex) in submitted[dst as usize..].iter_mut().zip(src) {
                        *slot = ("CPU", drawn_by(vertex.uv[0]));
                    }
                }
                for job in order.posed_jobs(&frame) {
                    submitted[job.dst as usize..(job.dst + job.count) as usize]
                        .fill(("GPU", drawn_by(job.rect[0])));
                }
                let (pre, models) = submitted.split_at(prefix.len());
                assert!(pre.iter().all(|draw| *draw == ("CPU", "pre")));
                assert_eq!(models[..20], [(lanes[0], "body"); 20]);
                assert_eq!(models[20..], [(lanes[1], "layer"); 20]);
            }
            // Each model's parts asked once, on its first frame (a taken part ends its walk).
            assert_eq!(ASKED.with(std::cell::Cell::get), asked);
        }
    }
}
