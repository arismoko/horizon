//! Render pipelines mods register (`Hooks::register_render_pipelines`, NeoForge's
//! `RegisterRenderPipelinesEvent`): the registrations, kept once at start, and what each
//! resource load makes of them. A load reads every registered pipeline's GLSL core shader
//! program (`assets/<namespace>/shaders/<path>.vsh` and `.fsh`, resolving `#include <ns:path>`,
//! `#include "relative"` and `#moj_import` through the active resource stack), translates it
//! through [`sunrise_mod_pipeline`] on top of [`sunrise_glsl`], and hands validated naga modules
//! to the renderer (`gfx::mod_pipelines`). A required pipeline the load cannot build fails the
//! load; an optional one is logged and left out. A frame reads none of this by name.
use mod_api::{
    BUILTIN_RENDER_TYPE_IDS, BlendFactor, ColorTargetState, DepthCompare, Identifier,
    LayeringTransform, PackGlslInput, PackPassSlot, PackUniformType, PackUniformValue,
    PipelineBlend, PipelineModifier, PipelineModifierRef, PipelineState, PipelineTexture,
    PipelineTextureError, PrimitiveTopology, RenderPipeline, RenderPipelineError,
    RenderPipelineRef, RenderSetup, RenderTypeError, RenderTypeRef, TextureTransform,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use sunrise_mod_pipeline::{Options as PipelineOptions, ProgramSources};

static REGISTERED: OnceLock<Vec<RenderPipeline>> = OnceLock::new();
static REGISTERED_RENDER_TYPES: OnceLock<Vec<RenderSetup>> = OnceLock::new();
static REGISTERED_MODIFIERS: OnceLock<Vec<PipelineModifier>> = OnceLock::new();
static PACK_UNIFORMS: OnceLock<HashMap<&'static str, PackUniformType>> = OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
pub fn has_registered() -> bool {
    REGISTERED.get().is_some_and(|v| !v.is_empty())
}

/// The built-in tables exported by `RenderPipelines.requiredPipelines` and `optionalPipelines`.
#[derive(serde::Deserialize)]
struct VanillaPipelines {
    required: HashSet<String>,
    optional: HashSet<String>,
}

#[derive(Debug)]
pub enum RegistrationError {
    Read {
        path: std::path::PathBuf,
        error: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        error: serde_json::Error,
    },
    InvalidIdTable(std::path::PathBuf),
    VanillaCollision {
        owner: String,
        id: Identifier,
        optional: bool,
    },
    DuplicateModId {
        earlier: String,
        owner: String,
        id: Identifier,
    },
    InvalidPackBinding(String),
    InvalidShaderDefine(String),
    InvalidPipelineState(String),
    InvalidVertexFormat(String),
    InvalidBindGroupLayout(String),
    BuiltinRenderTypeCollision {
        owner: String,
        id: Identifier,
    },
    DuplicateRenderTypeId {
        earlier: String,
        owner: String,
        id: Identifier,
    },
    InvalidRenderSetup(String),
    UnknownRenderTypePipeline {
        owner: String,
        id: Identifier,
        pipeline: Identifier,
    },
    InvalidComputePipeline(String),
    AlreadyPublished,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("register_render_pipelines: ")?;
        match self {
            Self::Read { path, error } => write!(
                f,
                "could not read {}: {error}; prepare the client resources again",
                path.display()
            ),
            Self::Parse { path, error } => write!(
                f,
                "invalid {}: {error}; prepare the client resources again",
                path.display()
            ),
            Self::InvalidIdTable(path) => write!(
                f,
                "invalid pipeline ID table in {}; prepare the client resources again",
                path.display()
            ),
            Self::VanillaCollision {
                owner,
                id,
                optional,
            } => {
                let kind = if *optional { "optional" } else { "required" };
                write!(
                    f,
                    "mod {owner}: {kind} pipeline {id} collides with vanilla {kind} pipeline"
                )
            }
            Self::DuplicateModId { earlier, owner, id } => write!(
                f,
                "mod {owner}: pipeline {id} collides with earlier registration from mod {earlier}"
            ),
            Self::InvalidPackBinding(reason) => f.write_str(reason),
            Self::InvalidShaderDefine(reason) => f.write_str(reason),
            Self::InvalidPipelineState(reason) => f.write_str(reason),
            Self::InvalidVertexFormat(reason) => f.write_str(reason),
            Self::InvalidBindGroupLayout(reason) => f.write_str(reason),
            Self::BuiltinRenderTypeCollision { owner, id } => write!(
                f,
                "mod {owner}: render type {id} collides with built-in render type"
            ),
            Self::DuplicateRenderTypeId { earlier, owner, id } => write!(
                f,
                "mod {owner}: render type {id} collides with earlier registration from mod {earlier}"
            ),
            Self::InvalidRenderSetup(reason) => f.write_str(reason),
            Self::UnknownRenderTypePipeline {
                owner,
                id,
                pipeline,
            } => write!(
                f,
                "mod {owner}: render type {id} references unknown pipeline {pipeline}"
            ),
            Self::InvalidComputePipeline(reason) => f.write_str(reason),
            Self::AlreadyPublished => f.write_str("pipeline registrations were already published"),
        }
    }
}

impl std::error::Error for RegistrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { error, .. } => Some(error),
            Self::Parse { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Validate and publish the mods' registrations once at startup, as `register_render_pipelines`
/// left them: each ID at most once and never colliding with vanilla's same-kind table.
/// A pipeline's place in the list is its slot.
pub fn register(
    pipelines: Vec<RenderPipeline>,
    vanilla_root: &Path,
) -> Result<(), RegistrationError> {
    if pipelines.is_empty() {
        return Ok(());
    }
    let path = vanilla_root.join("render-pipelines.json");
    let bytes = std::fs::read(&path).map_err(|error| RegistrationError::Read {
        path: path.clone(),
        error,
    })?;
    let vanilla: VanillaPipelines =
        serde_json::from_slice(&bytes).map_err(|error| RegistrationError::Parse {
            path: path.clone(),
            error,
        })?;
    if [&vanilla.required, &vanilla.optional]
        .into_iter()
        .any(|ids| {
            ids.is_empty()
                || ids
                    .iter()
                    .any(|id| Identifier::parse_namespaced(id).is_none())
        })
    {
        return Err(RegistrationError::InvalidIdTable(path));
    }
    let mut seen = BTreeMap::new();
    for pipeline in &pipelines {
        let table = if pipeline.optional {
            &vanilla.optional
        } else {
            &vanilla.required
        };
        if table.contains(&pipeline.id.to_string()) {
            return Err(RegistrationError::VanillaCollision {
                owner: pipeline.owner().to_owned(),
                id: pipeline.id.clone(),
                optional: pipeline.optional,
            });
        }
        if let Some(earlier) = seen.insert(&pipeline.id, pipeline.owner()) {
            return Err(RegistrationError::DuplicateModId {
                earlier: earlier.to_owned(),
                owner: pipeline.owner().to_owned(),
                id: pipeline.id.clone(),
            });
        }
        if let Err(reason) = pipeline.validate_defines() {
            return Err(RegistrationError::InvalidShaderDefine(format!(
                "mod {}: pipeline {}: {reason}",
                pipeline.owner(),
                pipeline.id
            )));
        }
        if let Err(reason) = pipeline.validate_state() {
            return Err(RegistrationError::InvalidPipelineState(format!(
                "mod {}: pipeline {}: {reason}",
                pipeline.owner(),
                pipeline.id
            )));
        }
        if let Err(reason) = pipeline.validate_vertex_bindings() {
            return Err(RegistrationError::InvalidVertexFormat(format!(
                "mod {}: pipeline {}: {reason}",
                pipeline.owner(),
                pipeline.id
            )));
        }
        if let Err(reason) = pipeline.validate_bind_group_layout() {
            return Err(RegistrationError::InvalidBindGroupLayout(format!(
                "mod {}: pipeline {}: {reason}",
                pipeline.owner(),
                pipeline.id
            )));
        }
    }
    validate_pack_bindings(&pipelines).map_err(RegistrationError::InvalidPackBinding)?;
    REGISTERED
        .set(pipelines)
        .map_err(|_| RegistrationError::AlreadyPublished)
}

/// Built-in Iris/OptiFine pack GLSL sampler, image, and uniform names that mod `PackBinding`
/// declarations may not claim. Samplers, images, and value uniforms share GLSL's `uniform`
/// identifier namespace, so both `PackDepthTarget` samplers and `PackUniformDecl` names are
/// checked against this full table.
fn reserved_pack_identifier(name: &str) -> Option<&'static str> {
    let is_color_img = name
        .strip_prefix("colorimg")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < sunrise_shaders::BUFFERS);
    let is_shadow_img = name
        .strip_prefix("shadowcolorimg")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < 8);
    if sunrise_shaders::buffer_index(name).is_some()
        || sunrise_shaders::shadow_color(name).is_some()
        || is_color_img
        || is_shadow_img
        || matches!(
            name,
            "depthtex0"
                | "depthtex1"
                | "depthtex2"
                | "gdepthtex"
                | "shadowtex0"
                | "shadowtex1"
                | "shadowtex0HW"
                | "shadowtex1HW"
                | "watershadow"
                | "shadow"
                | "noisetex"
                | "texture"
                | "gtexture"
                | "tex"
                | "lightmap"
                | "normals"
                | "specular"
                | sunrise_shaders::WATER_MASK
        )
    {
        return Some("built-in shader-pack sampler or image");
    }
    if sunrise_render_shaderpack::pack_uniforms::NAMES.contains(&name) {
        return Some("built-in shader-pack uniform");
    }
    None
}

/// Validate GLSL syntax, reserved Iris/OptiFine names, and cross-mod uniqueness across every
/// registered `RenderPipeline::pack_binding`. Multiple pipelines owned by the same mod may
/// share one `PackDepthTarget.id` (with identical samplers), compatible defines, and typed
/// uniforms; cross-mod collisions and GLSL uniform/sampler namespace collisions fail startup
/// naming the offending mod(s) and identifier.
fn validate_pack_bindings(pipelines: &[RenderPipeline]) -> Result<(), String> {
    let mut depth_targets: BTreeMap<&Identifier, (&str, &mod_api::PackDepthTarget)> =
        BTreeMap::new();
    let mut samplers: BTreeMap<&str, (&str, &Identifier)> = BTreeMap::new();
    // Value uniforms keyed by GLSL name: `(owner, ty, is_previous)`.
    let mut uniforms: BTreeMap<&str, (&str, mod_api::PackUniformType, bool)> = BTreeMap::new();
    let mut defines: BTreeMap<&str, (&str, Option<&str>)> = BTreeMap::new();

    for pipeline in pipelines {
        let Some(binding) = &pipeline.pack_binding else {
            continue;
        };
        let owner = pipeline.owner();
        binding.validate_syntax().map_err(|reason| {
            format!(
                "Invalid pack binding on render pipeline {} (registered by {}): {reason}",
                pipeline.id, owner
            )
        })?;

        if let Some(target) = &binding.depth_target
            && let mod_api::PackDepthTarget::Aux {
                id,
                depth_samplers,
                opaque_depth_samplers,
                ..
            } = target
        {
            if let Some(&(first_owner, first_target)) = depth_targets.get(id) {
                if first_owner != owner {
                    return Err(format!(
                        "Duplicate pack depth target registration: {} (registered by {} and {})",
                        id, first_owner, owner
                    ));
                }
                if first_target != target {
                    return Err(format!(
                        "Conflicting pack depth target declaration for {} in mod {}: shared targets must declare identical samplers",
                        id, owner
                    ));
                }
            } else {
                depth_targets.insert(id, (owner, target));
            }
            for sampler in depth_samplers.iter().chain(opaque_depth_samplers.iter()) {
                if let Some(kind) = reserved_pack_identifier(sampler) {
                    return Err(format!(
                        "Reserved pack sampler name {:?} on render pipeline {} (registered by {}) collides with a {kind}",
                        sampler, pipeline.id, owner
                    ));
                }
                if let Some(&(u_owner, _, _)) = uniforms.get(sampler.as_str()) {
                    if u_owner != owner {
                        return Err(format!(
                            "Pack GLSL identifier collision on {:?}: registered as a uniform by {} and as a sampler by {}",
                            sampler, u_owner, owner
                        ));
                    }
                    return Err(format!(
                        "Conflicting pack GLSL identifier {:?} in mod {}: declared as both a uniform and a sampler",
                        sampler, owner
                    ));
                }
                if let Some(&(first_owner, first_target_id)) = samplers.get(sampler.as_str()) {
                    if first_owner != owner {
                        return Err(format!(
                            "Duplicate pack sampler registration: {:?} (registered by {} and {})",
                            sampler, first_owner, owner
                        ));
                    }
                    if first_target_id != id {
                        return Err(format!(
                            "Conflicting pack sampler registration {:?} in mod {}: bound to both {} and {}",
                            sampler, owner, first_target_id, id
                        ));
                    }
                } else {
                    samplers.insert(sampler.as_str(), (owner, id));
                }
            }
        }

        for uniform in &binding.frame_uniforms {
            let names = std::iter::once((uniform.name.as_str(), false))
                .chain(uniform.previous_name.as_deref().map(|n| (n, true)));
            for (name, is_previous) in names {
                if let Some(kind) = reserved_pack_identifier(name) {
                    return Err(format!(
                        "Reserved pack uniform name {:?} on render pipeline {} (registered by {}) collides with a {kind}",
                        name, pipeline.id, owner
                    ));
                }
                if let Some(&(s_owner, _)) = samplers.get(name) {
                    if s_owner != owner {
                        return Err(format!(
                            "Pack GLSL identifier collision on {:?}: registered as a sampler by {} and as a uniform by {}",
                            name, s_owner, owner
                        ));
                    }
                    return Err(format!(
                        "Conflicting pack GLSL identifier {:?} in mod {}: declared as both a sampler and a uniform",
                        name, owner
                    ));
                }
                if let Some(&(first_owner, first_ty, first_is_previous)) = uniforms.get(name) {
                    if first_owner != owner {
                        return Err(format!(
                            "Duplicate pack uniform registration: {:?} (registered by {} and {})",
                            name, first_owner, owner
                        ));
                    }
                    if first_ty != uniform.ty {
                        return Err(format!(
                            "Conflicting pack uniform type for {:?} in mod {}: declared as {:?} and {:?}",
                            name, owner, first_ty, uniform.ty
                        ));
                    }
                    if first_is_previous != is_previous {
                        return Err(format!(
                            "Conflicting pack uniform role for {:?} in mod {}: declared as both current and previous_name",
                            name, owner
                        ));
                    }
                } else {
                    uniforms.insert(name, (owner, uniform.ty, is_previous));
                }
            }
        }

        for (name, value) in &binding.defines {
            let val = value.as_deref();
            if let Some(&(first_owner, first_val)) = defines.get(name.as_str()) {
                if first_owner != owner {
                    return Err(format!(
                        "Duplicate pack define registration: {:?} (registered by {} and {})",
                        name, first_owner, owner
                    ));
                }
                if first_val != val {
                    return Err(format!(
                        "Conflicting pack define value for {:?} in mod {}: {:?} vs {:?}",
                        name, owner, first_val, val
                    ));
                }
            } else {
                defines.insert(name.as_str(), (owner, val));
            }
        }
    }
    Ok(())
}

/// The registered pipelines; none until registration publishes them.
pub fn registered() -> &'static [RenderPipeline] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

#[derive(Debug)]
pub enum ModifierRegistrationError {
    DuplicateModId {
        earlier: String,
        owner: String,
        id: Identifier,
    },
    InvalidState {
        owner: String,
        id: Identifier,
        reason: String,
    },
    AlreadyPublished,
}

impl std::fmt::Display for ModifierRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("register_pipeline_modifiers: ")?;
        match self {
            Self::DuplicateModId { earlier, owner, id } => write!(
                f,
                "mod {owner}: pipeline modifier {id} collides with earlier registration from mod {earlier}"
            ),
            Self::InvalidState { owner, id, reason } => {
                write!(f, "mod {owner}: pipeline modifier {id}: {reason}")
            }
            Self::AlreadyPublished => {
                f.write_str("pipeline modifier registrations were already published")
            }
        }
    }
}

impl std::error::Error for ModifierRegistrationError {}

/// Validate a modifier registration list without touching process-global state.
pub fn validate_modifiers(modifiers: &[PipelineModifier]) -> Result<(), ModifierRegistrationError> {
    let mut seen = BTreeMap::new();
    for modifier in modifiers {
        if let Some(earlier) = seen.insert(&modifier.id, modifier.owner()) {
            return Err(ModifierRegistrationError::DuplicateModId {
                earlier: earlier.to_owned(),
                owner: modifier.owner().to_owned(),
                id: modifier.id.clone(),
            });
        }
        if let Err(reason) = modifier.validate() {
            return Err(ModifierRegistrationError::InvalidState {
                owner: modifier.owner().to_owned(),
                id: modifier.id.clone(),
                reason,
            });
        }
    }
    Ok(())
}

/// Validate and publish the mods' pipeline modifier registrations once at startup.
pub fn register_modifiers(
    modifiers: Vec<PipelineModifier>,
) -> Result<(), ModifierRegistrationError> {
    if modifiers.is_empty() {
        return Ok(());
    }
    validate_modifiers(&modifiers)?;
    REGISTERED_MODIFIERS
        .set(modifiers)
        .map_err(|_| ModifierRegistrationError::AlreadyPublished)
}

/// The registered pipeline modifiers; none until registration publishes them.
pub fn registered_modifiers() -> &'static [PipelineModifier] {
    REGISTERED_MODIFIERS.get().map_or(&[], Vec::as_slice)
}

/// Well-known built-in render types matching [`BUILTIN_RENDER_TYPE_IDS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinRenderType {
    Solid,
    Cutout,
    Translucent,
    EntitySolid,
    EntityCutout,
    EntityTranslucent,
    Eyes,
    EnergySwirl,
    Lightning,
    ItemCutout,
    ItemTranslucent,
}

impl BuiltinRenderType {
    /// Resolve a built-in render type from its slot index in [`BUILTIN_RENDER_TYPE_IDS`].
    pub fn from_slot(slot: u32) -> Option<Self> {
        match slot {
            0 => Some(Self::Solid),
            1 => Some(Self::Cutout),
            2 => Some(Self::Translucent),
            3 => Some(Self::EntitySolid),
            4 => Some(Self::EntityCutout),
            5 => Some(Self::EntityTranslucent),
            6 => Some(Self::Eyes),
            7 => Some(Self::EnergySwirl),
            8 => Some(Self::Lightning),
            9 => Some(Self::ItemCutout),
            10 => Some(Self::ItemTranslucent),
            _ => None,
        }
    }

    /// Whether this built-in render type routes through the translucent sorted/OIT pass.
    pub fn is_translucent(self) -> bool {
        matches!(
            self,
            Self::Translucent
                | Self::EntityTranslucent
                | Self::Eyes
                | Self::EnergySwirl
                | Self::Lightning
                | Self::ItemTranslucent
        )
    }
}

/// Slot index of `id` in [`BUILTIN_RENDER_TYPE_IDS`], if it names a built-in render type.
pub fn builtin_render_type_slot(id: &Identifier) -> Option<u32> {
    let text = id.to_string();
    BUILTIN_RENDER_TYPE_IDS
        .iter()
        .position(|&builtin| builtin == text)
        .map(|index| index as u32)
}

fn validate_render_types(
    render_types: &[RenderSetup],
    mod_pipelines: &[RenderPipeline],
    vanilla: &VanillaPipelines,
) -> Result<(), RegistrationError> {
    let mod_ids: BTreeSet<&Identifier> = mod_pipelines.iter().map(|p| &p.id).collect();
    let mut seen = BTreeMap::new();
    for setup in render_types {
        if builtin_render_type_slot(&setup.id).is_some() {
            return Err(RegistrationError::BuiltinRenderTypeCollision {
                owner: setup.owner().to_owned(),
                id: setup.id.clone(),
            });
        }
        if let Some(earlier) = seen.insert(&setup.id, setup.owner()) {
            return Err(RegistrationError::DuplicateRenderTypeId {
                earlier: earlier.to_owned(),
                owner: setup.owner().to_owned(),
                id: setup.id.clone(),
            });
        }
        if let Err(reason) = setup.validate() {
            return Err(RegistrationError::InvalidRenderSetup(format!(
                "mod {}: render type {}: {reason}",
                setup.owner(),
                setup.id
            )));
        }
        let pipeline_str = setup.pipeline.to_string();
        let known_pipeline = mod_ids.contains(&setup.pipeline)
            || vanilla.required.contains(&pipeline_str)
            || vanilla.optional.contains(&pipeline_str);
        if !known_pipeline {
            return Err(RegistrationError::UnknownRenderTypePipeline {
                owner: setup.owner().to_owned(),
                id: setup.id.clone(),
                pipeline: setup.pipeline.clone(),
            });
        }
    }
    Ok(())
}

/// Validate and publish the mods' custom render type registrations once at startup, after
/// [`register`] has published the mod pipelines.
pub fn register_render_types(
    render_types: Vec<RenderSetup>,
    vanilla_root: &Path,
) -> Result<(), RegistrationError> {
    if render_types.is_empty() {
        return Ok(());
    }
    let path = vanilla_root.join("render-pipelines.json");
    let bytes = std::fs::read(&path).map_err(|error| RegistrationError::Read {
        path: path.clone(),
        error,
    })?;
    let vanilla: VanillaPipelines =
        serde_json::from_slice(&bytes).map_err(|error| RegistrationError::Parse {
            path: path.clone(),
            error,
        })?;
    if [&vanilla.required, &vanilla.optional]
        .into_iter()
        .any(|ids| {
            ids.is_empty()
                || ids
                    .iter()
                    .any(|id| Identifier::parse_namespaced(id).is_none())
        })
    {
        return Err(RegistrationError::InvalidIdTable(path));
    }
    validate_render_types(&render_types, registered(), &vanilla)?;
    REGISTERED_RENDER_TYPES
        .set(render_types)
        .map_err(|_| RegistrationError::AlreadyPublished)
}

/// The registered custom render types; none until startup registration publishes them.
pub fn registered_render_types() -> &'static [RenderSetup] {
    REGISTERED_RENDER_TYPES.get().map_or(&[], Vec::as_slice)
}

/// Resolved target of a live [`RenderTypeRef`] handle.
#[derive(Clone, Copy, Debug)]
pub enum ResolvedRenderType<'a> {
    Builtin(BuiltinRenderType),
    Custom {
        slot: u32,
        setup: &'a RenderSetup,
        mod_pipeline_slot: Option<u32>,
    },
}

impl ResolvedRenderType<'_> {
    /// Whether geometry drawn with this render type is translucent: a translucent built-in, or
    /// a custom [`RenderSetup`] with `sort_on_upload` that does not force the solid model phase.
    pub fn is_translucent(&self) -> bool {
        match self {
            Self::Builtin(builtin) => builtin.is_translucent(),
            Self::Custom { setup, .. } => setup.sort_on_upload && !setup.force_solid_model_phase,
        }
    }

    /// The model-view layering this render type applies (`LayeringTransform::None` for built-ins).
    pub fn layering(&self) -> LayeringTransform {
        match self {
            Self::Builtin(_) => LayeringTransform::None,
            Self::Custom { setup, .. } => setup.layering,
        }
    }

    /// The texture-coordinate transform this render type applies (`Default` for built-ins).
    pub fn texture_transform(&self) -> TextureTransform {
        match self {
            Self::Builtin(_) => TextureTransform::Default,
            Self::Custom { setup, .. } => setup.texture_transform,
        }
    }
}

/// Apply vanilla 26.3's `LayeringTransform` (`LayeringTransform.java` + `ProjectionType.java`)
/// to `model_view`.
///
/// Perspective scales by `1.0 - bias / 4096.0`; orthographic translates Z by `bias / 512.0`,
/// where `bias = +1.0` for `ViewOffsetZ` and `-1.0` for `ViewOffsetZForward`.
pub fn layering_transform_matrix(
    transform: LayeringTransform,
    model_view: glam::Mat4,
    orthographic: bool,
) -> glam::Mat4 {
    let bias = match transform {
        LayeringTransform::None => return model_view,
        LayeringTransform::ViewOffsetZ => 1.0f32,
        LayeringTransform::ViewOffsetZForward => -1.0f32,
        _ => return model_view,
    };
    if orthographic {
        model_view * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, bias / 512.0))
    } else {
        // JOML `Matrix4f.scale(s)`: the first three columns are multiplied by `s`.
        let s = 1.0 - bias / 4096.0;
        let mut scaled = model_view;
        scaled.x_axis *= s;
        scaled.y_axis *= s;
        scaled.z_axis *= s;
        scaled
    }
}

/// Build vanilla 26.3's `TextureTransform.createMatrix()` (`TextureTransform.java`).
pub fn texture_transform_matrix(
    transform: TextureTransform,
    millis: i64,
    glint_speed: f64,
) -> glam::Mat4 {
    let glint = |scale: f32| {
        let ticks = (millis as f64 * glint_speed * 8.0) as i64;
        let u = (ticks.rem_euclid(110_000) as f32) / 110_000.0;
        let v = (ticks.rem_euclid(30_000) as f32) / 30_000.0;
        // `translation(-u, v, 0).rotateZ(PI / 18).scale(scale)`, with JOML's `rotateZ` trig
        // (`Math.sin`, then `cosFromSin`) and its per-element products.
        const ANGLE: f32 = f32::from_bits(0x3e32_b8c2);
        let sin = crate::camera::joml::sin(ANGLE);
        let cos = crate::camera::joml::cos_from_sin(sin, ANGLE);
        glam::Mat4::from_cols(
            glam::Vec4::new(cos * scale, sin * scale, 0.0, 0.0),
            glam::Vec4::new(-sin * scale, cos * scale, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, scale, 0.0),
            glam::Vec4::new(-u, v, 0.0, 1.0),
        )
    };
    match transform {
        TextureTransform::Default => glam::Mat4::IDENTITY,
        TextureTransform::Glint => glint(8.0),
        TextureTransform::EntityGlint => glint(0.5),
        TextureTransform::ArmorEntityGlint => glint(0.16),
        TextureTransform::Offset([u, v]) => {
            glam::Mat4::from_translation(glam::Vec3::new(u, v, 0.0))
        }
        _ => glam::Mat4::IDENTITY,
    }
}

/// Resolve a registered pipeline modifier by id (static across resource reloads).
pub fn resolve_modifier(id: &Identifier) -> Result<PipelineModifierRef, RenderPipelineError> {
    resolve_modifier_in(registered_modifiers(), id)
}

pub(crate) fn resolve_modifier_in(
    modifiers: &[PipelineModifier],
    id: &Identifier,
) -> Result<PipelineModifierRef, RenderPipelineError> {
    let slot = modifiers
        .iter()
        .position(|m| &m.id == id)
        .ok_or(RenderPipelineError::Unknown)?;
    Ok(mod_api::__private::built::pipeline_modifier_ref(
        slot as u32,
    ))
}

/// Interned pipeline modifier stacks: stack id `0` is always the empty stack (no modifiers).
/// Each non-zero stack id names a distinct sequence of registered modifier slots in push order
/// (outermost first, innermost last). Applying a stack runs innermost first so the outermost
/// modifier wins on any field both set, matching NeoForge's `PipelineModifierStack.apply`.
#[derive(Clone, Debug)]
pub struct ModifierStacks {
    stacks: Vec<Vec<u32>>,
    index: HashMap<Vec<u32>, u32>,
}

impl Default for ModifierStacks {
    fn default() -> Self {
        Self::new()
    }
}

impl ModifierStacks {
    /// The empty stack id (no active modifiers).
    pub const EMPTY: u32 = 0;

    pub fn new() -> Self {
        Self {
            stacks: vec![Vec::new()],
            index: HashMap::new(),
        }
    }

    /// Clear interned non-empty stacks while retaining allocations for the next view.
    pub fn clear(&mut self) {
        self.stacks.truncate(1);
        self.index.clear();
    }

    /// Intern `stack` (modifier slots in push order: outermost first, innermost last).
    pub fn intern(&mut self, stack: &[u32]) -> u32 {
        if stack.is_empty() {
            return Self::EMPTY;
        }
        if let Some(&id) = self.index.get(stack) {
            return id;
        }
        let id = self.stacks.len() as u32;
        let owned = stack.to_vec();
        self.index.insert(owned.clone(), id);
        self.stacks.push(owned);
        id
    }

    /// Modifier slots in push order for `id` (empty for `0` or out-of-range).
    pub fn get(&self, id: u32) -> &[u32] {
        self.stacks.get(id as usize).map_or(&[], Vec::as_slice)
    }

    /// Apply stack `id` to `base` using `modifiers`: innermost applies first, then older ones,
    /// so for any field two modifiers set the outermost wins (`PipelineModifierStack.apply`).
    pub fn apply(
        &self,
        id: u32,
        state: PipelineState,
        modifiers: &[PipelineModifier],
    ) -> PipelineState {
        Self::apply_slots(self.get(id), state, modifiers)
    }

    /// [`apply`](Self::apply) for a modifier slot sequence (push order).
    pub fn apply_slots(
        slots: &[u32],
        mut state: PipelineState,
        modifiers: &[PipelineModifier],
    ) -> PipelineState {
        for &slot in slots.iter().rev() {
            if let Some(m) = modifiers.get(slot as usize) {
                state = m.apply(state);
            }
        }
        state
    }

    /// Format NeoForge's `<pipeline>/transform/<ns>/<path>` label for a modified variant.
    pub fn label(&self, id: u32, base_id: &str, modifiers: &[PipelineModifier]) -> String {
        Self::label_slots(self.get(id), base_id, modifiers)
    }

    /// [`label`](Self::label) for a modifier slot sequence (push order).
    pub fn label_slots(slots: &[u32], base_id: &str, modifiers: &[PipelineModifier]) -> String {
        let mut out = base_id.to_owned();
        for &slot in slots.iter().rev() {
            if let Some(m) = modifiers.get(slot as usize) {
                out.push_str("/transform/");
                out.push_str(m.id.namespace());
                out.push('/');
                out.push_str(m.id.path());
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.stacks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stacks.len() <= 1
    }
}

/// A registered pipeline one resource load built: validated GLSL naga modules (plus expanded
/// source and defines for lazy linear-light fragment translation) and its fixed state.
#[derive(Clone, Debug)]
pub struct Built {
    /// The owning mod, not necessarily the pipeline namespace.
    pub owner: String,
    /// The pipeline's id, for the log.
    pub id: Identifier,
    /// The validated vertex and fragment naga modules and set-0 binding usage.
    pub program: Arc<sunrise_glsl::Translated>,
    /// Expanded vertex GLSL kept for lazy linear-light fragment re-translation.
    pub vertex_expanded: Arc<str>,
    /// Expanded fragment GLSL kept for lazy linear-light fragment re-translation.
    pub fragment_expanded: Arc<str>,
    /// Pipeline defines (`NAME` or `NAME value`) applied during translation.
    pub defines: Arc<[String]>,
    pub state: PipelineState,
    /// Per-target colour blend and write-mask states (`1..=MAX_COLOR_TARGETS`).
    pub color_targets: Vec<Option<ColorTargetState>>,
    /// Effective vertex buffer bindings sorted by binding index.
    pub vertex_bindings: Vec<(u8, mod_api::VertexFormat)>,
    /// Whether this pipeline uses the default vertex format (`VertexFormat::position_tex_color`).
    pub uses_default_vertex_format: bool,
    /// Whether the pipeline opted in to OIT (`RenderPipeline::order_independent()`).
    pub order_independent: bool,
    /// Three translated OIT variants (`DepthBounds`, `Transmittance`, `Accumulate`) when
    /// `order_independent && state.blend != PipelineBlend::Opaque`.
    pub oit: Option<Arc<[sunrise_glsl::Translated; 3]>>,
    /// Whether the load may go on without it ([`RenderPipeline::optional()`]).
    pub optional: bool,
    /// What its shader samples, if anything.
    pub texture: Option<BuiltTexture>,
    /// Mod-declared set-1 bind-group layout (`#5428`).
    pub bind_group_layout: mod_api::BindGroupLayout,
    /// Declared push-constant byte size (`0..=128`, multiple of 4).
    pub push_constant_size: u32,
}

impl Built {
    /// Convert this pipeline's set-1 [`mod_api::BindGroupLayout`] into [`sunrise_mod_pipeline::ModBinding`]
    /// entries for shader translation.
    pub fn mod_bindings(&self) -> Vec<sunrise_mod_pipeline::ModBinding<'_>> {
        mod_bindings_from_layout(&self.bind_group_layout)
    }
}

pub(crate) fn mod_bindings_from_layout(
    layout: &mod_api::BindGroupLayout,
) -> Vec<sunrise_mod_pipeline::ModBinding<'_>> {
    layout
        .entries
        .iter()
        .filter_map(|e| {
            let kind = match e.kind {
                mod_api::BindingKind::UniformBuffer { .. } => {
                    sunrise_mod_pipeline::ModBindingKind::UniformBuffer
                }
                mod_api::BindingKind::CombinedImageSampler { .. } => {
                    sunrise_mod_pipeline::ModBindingKind::CombinedImageSampler
                }
                mod_api::BindingKind::TexelBuffer { .. } => {
                    sunrise_mod_pipeline::ModBindingKind::TexelBuffer
                }
                mod_api::BindingKind::StorageBuffer { read_only, .. } => {
                    sunrise_mod_pipeline::ModBindingKind::StorageBuffer { read_only }
                }
                mod_api::BindingKind::StorageImage { access } => {
                    sunrise_mod_pipeline::ModBindingKind::StorageImage {
                        access: match access {
                            mod_api::StorageImageAccess::WriteOnly => {
                                sunrise_glsl::ImageAccess::WriteOnly
                            }
                            mod_api::StorageImageAccess::ReadOnly => {
                                sunrise_glsl::ImageAccess::ReadOnly
                            }
                            mod_api::StorageImageAccess::ReadWrite => {
                                sunrise_glsl::ImageAccess::ReadWrite
                            }
                            _ => return None,
                        },
                    }
                }
                _ => return None,
            };
            Some(sunrise_mod_pipeline::ModBinding {
                name: e.name.as_str(),
                kind,
            })
        })
        .collect()
}

/// A textured pipeline's texture as a load found it ([`PipelineTexture`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuiltTexture {
    /// A sprite of the 2D atlas, by its name there (`block/stone`, `powah:block/energy_charge`),
    /// looked up when the renderer builds the pipeline.
    Sprite(String),
}

/// The registered pipelines as one resource load built them, by slot. An optional pipeline the
/// load refused has no shader here and resolves to [`RenderPipelineError::Invalid`].
#[derive(Clone, Debug, Default)]
pub struct Pipelines {
    keys: Vec<Identifier>,
    built: Vec<Option<Built>>,
    /// The render assets' generation this set was loaded for; 0 for none.
    generation: u64,
    invalid: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    oit_ineligible_logged: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    /// The texture-taking built-in render types made so far, shared by clones of this set.
    textured: std::sync::Arc<std::sync::Mutex<TexturedRenderTypes>>,
    /// The registered compute pipelines of the same load (`mod_compute`).
    compute: crate::mod_compute::ComputeSet,
}

/// The texture-taking built-in render types of one load (`RenderTypes.entitySolid(Identifier)`,
/// `entityCutout`, `entityTranslucent`): one per built-in and texture, in the order they were
/// first asked for, which is their slot after the built-in and custom ones.
#[derive(Debug, Default)]
struct TexturedRenderTypes {
    by_texture: BTreeMap<(u32, Identifier), u32>,
    made: Vec<(BuiltinRenderType, Identifier)>,
}

impl Pipelines {
    /// Build every registered pipeline from `source`, for the load of `generation`. An optional
    /// pipeline whose file is missing or refused is logged with its reason and the others build;
    /// a required one fails the load, naming every required pipeline refused and why.
    pub fn load(
        source: &crate::resource_source::ResourceSource,
        generation: u64,
    ) -> Result<Self, String> {
        let read = |file: &str| match source.read_to_string(file) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        let mut set = Self::build(registered(), generation, read)?;
        set.compute = crate::mod_compute::ComputeSet::build(
            crate::mod_compute::registered(),
            generation,
            read,
        );
        Ok(set)
    }

    /// Build every registered pipeline, reading `.vsh`, `.fsh`, and include files via `read`
    /// (returning `Ok(None)` when absent, `Err(why)` when unreadable). A pipeline's texture is
    /// named here; its sprite is looked up when the renderer builds it.
    pub(crate) fn build(
        registered: &[RenderPipeline],
        generation: u64,
        read: impl Fn(&str) -> Result<Option<String>, String> + Sync,
    ) -> Result<Self, String> {
        let mut required = Vec::new();
        let read_stage = |file: &str| -> Result<String, String> {
            read(file)
                .map_err(|cause| format!("{file}: {cause}"))?
                .ok_or_else(|| format!("{file}: not found"))
        };
        let include = |id: &str| -> Result<Option<String>, String> {
            let Some((ns, rest)) = id.split_once(':') else {
                return Ok(None);
            };
            let asset = format!("assets/{ns}/{rest}");
            read(&asset)
        };
        let built = registered
            .iter()
            .map(|pipeline| {
                let vertex_id = pipeline
                    .vertex_shader
                    .as_ref()
                    .unwrap_or(&pipeline.fragment);
                let vertex_file = shader_file(vertex_id, "vsh");
                let vertex_dir = shader_dir(vertex_id);
                let fragment_file = shader_file(&pipeline.fragment, "fsh");
                let fragment_dir = shader_dir(&pipeline.fragment);
                let defines_owned: Vec<String> = pipeline
                    .defines
                    .iter()
                    .map(|(name, value)| match value {
                        Some(v) => format!("{name} {v}"),
                        None => name.clone(),
                    })
                    .collect();
                let define_refs: Vec<&str> = defines_owned.iter().map(String::as_str).collect();
                let vertex_bindings = pipeline.effective_vertex_bindings();
                let vertex_inputs_owned: Vec<(String, &'static str)> = vertex_bindings
                    .iter()
                    .flat_map(|(_, fmt)| {
                        fmt.attributes
                            .iter()
                            .map(|a| (a.name.clone(), a.format.glsl_type()))
                    })
                    .collect();
                let vertex_input_refs: Vec<(&str, &str)> = vertex_inputs_owned
                    .iter()
                    .map(|(name, ty)| (name.as_str(), *ty))
                    .collect();
                let mod_bindings = mod_bindings_from_layout(&pipeline.bind_group_layout);
                let loaded = pipeline
                    .validate_defines()
                    .and_then(|()| pipeline.validate_state())
                    .and_then(|()| pipeline.validate_vertex_bindings())
                    .and_then(|()| {
                        pipeline
                            .validate_bind_group_layout()
                            .map_err(|e| e.to_string())
                    })
                    .and_then(|()| {
                        let vertex = read_stage(&vertex_file)?;
                        let fragment = read_stage(&fragment_file)?;
                        let sources = ProgramSources {
                            vertex_file: &vertex_file,
                            vertex_dir: &vertex_dir,
                            vertex: &vertex,
                            fragment_file: &fragment_file,
                            fragment_dir: &fragment_dir,
                            fragment: &fragment,
                        };
                        let options = PipelineOptions {
                            textured: pipeline.texture.is_some(),
                            defines: &define_refs,
                            vertex_inputs: &vertex_input_refs,
                            mod_bindings: &mod_bindings,
                            push_constant_size: pipeline.push_constant_size,
                            linear_light: false,
                            check_backends: false,
                            interface: sunrise_mod_pipeline::DEVICELESS_INTERFACE_LIMITS,
                        };
                        let built_program = crate::crash::contain(|| {
                            sunrise_mod_pipeline::translate(&sources, &options, &include)
                        })
                        .map_err(|panic| format!("{fragment_file}: checking it panicked: {panic}"))?
                        .map_err(|problems| {
                            problems
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })?;
                        let oit = if pipeline.order_independent
                            && pipeline.state.blend != PipelineBlend::Opaque
                        {
                            let additive = pipeline.state.blend == PipelineBlend::Additive;
                            let variants = crate::crash::contain(|| {
                                sunrise_mod_pipeline::translate_oit(
                                    &sources, &options, additive, &include,
                                )
                            })
                            .map_err(|panic| {
                                format!("{fragment_file}: checking OIT variants panicked: {panic}")
                            })?
                            .map_err(|problems| {
                                problems
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            })?;
                            let [d, t, a] = variants;
                            Some(Arc::new([d.translated, t.translated, a.translated]))
                        } else {
                            None
                        };
                        Ok((built_program, oit))
                    })
                    .and_then(|(built_program, oit)| {
                        let texture = match &pipeline.texture {
                            None => None,
                            Some(texture) => Some(load_texture(texture).map_err(|cause| {
                                format!("its texture: {}", texture_refusal(cause))
                            })?),
                        };
                        Ok((built_program, oit, texture))
                    });
                match loaded {
                    Ok((built_program, oit, texture)) => Some(Built {
                        owner: pipeline.owner().to_owned(),
                        id: pipeline.id.clone(),
                        program: Arc::new(built_program.translated),
                        vertex_expanded: Arc::from(built_program.vertex_expanded),
                        fragment_expanded: Arc::from(built_program.fragment_expanded),
                        defines: Arc::from(defines_owned),
                        state: pipeline.state,
                        color_targets: pipeline.effective_color_targets(),
                        vertex_bindings,
                        uses_default_vertex_format: pipeline.uses_default_vertex_format(),
                        order_independent: pipeline.order_independent,
                        oit,
                        optional: pipeline.optional,
                        texture,
                        bind_group_layout: pipeline.bind_group_layout.clone(),
                        push_constant_size: pipeline.push_constant_size,
                    }),
                    Err(reason) => {
                        let refusal = format!(
                            "render pipeline {} from mod {} refused: {reason}",
                            pipeline.id,
                            pipeline.owner()
                        );
                        if pipeline.optional {
                            log::warn!("Sunrise: {refusal}");
                        } else {
                            required.push(format!("required {refusal}"));
                        }
                        None
                    }
                }
            })
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        Ok(Self {
            keys: registered.iter().map(|p| p.id.clone()).collect(),
            built,
            generation,
            invalid: Default::default(),
            oit_ineligible_logged: Default::default(),
            textured: Default::default(),
            compute: Default::default(),
        })
    }

    /// The compute pipelines of this load: what `RenderAssets::compute_pipeline` resolves in and
    /// the renderer builds on its device.
    pub fn compute(&self) -> &crate::mod_compute::ComputeSet {
        &self.compute
    }

    /// The registered [`PrimitiveTopology`] of `slot`, or `None` if `slot` is out of range or
    /// was not built.
    pub fn topology(&self, slot: u32) -> Option<PrimitiveTopology> {
        self.built
            .get(slot as usize)
            .and_then(|b| b.as_ref())
            .map(|b| b.state.topology)
    }

    /// The effective vertex buffer bindings of `slot`, or `None` if `slot` is out of range or
    /// was not built.
    pub fn vertex_bindings(&self, slot: u32) -> Option<&[(u8, mod_api::VertexFormat)]> {
        self.built
            .get(slot as usize)
            .and_then(|b| b.as_ref())
            .map(|b| b.vertex_bindings.as_slice())
    }

    /// The mod-declared set-1 [`mod_api::BindGroupLayout`] and `push_constant_size` of `slot`,
    /// or `None` if `slot` is out of range or was not built.
    pub fn bind_group_layout(&self, slot: u32) -> Option<(&mod_api::BindGroupLayout, u32)> {
        self.built
            .get(slot as usize)
            .and_then(|b| b.as_ref())
            .map(|b| (&b.bind_group_layout, b.push_constant_size))
    }

    /// Whether `slot` writes depth (`depth_stencil.is_some_and(|d| d.write)`).
    pub fn writes_depth(&self, slot: u32) -> bool {
        self.built
            .get(slot as usize)
            .and_then(|b| b.as_ref())
            .is_some_and(|b| b.state.depth_stencil.is_some_and(|d| d.write))
    }

    /// Whether `slot` uses the default vertex format (`VertexFormat::position_tex_color`), which
    /// makes it eligible for `pipeline_quads`.
    pub fn uses_default_vertex_format(&self, slot: u32) -> bool {
        self.built
            .get(slot as usize)
            .and_then(|b| b.as_ref())
            .is_some_and(|b| b.uses_default_vertex_format)
    }

    /// Whether `slot` may draw at `RenderLevelStage::OitTranslucent`: opted in via
    /// `.order_independent()` with a non-`Opaque` blend mode and compiled OIT variants.
    pub fn is_oit_eligible(&self, slot: u32) -> bool {
        self.built
            .get(slot as usize)
            .and_then(|b| b.as_ref())
            .is_some_and(|b| {
                b.order_independent && b.state.blend != PipelineBlend::Opaque && b.oit.is_some()
            })
    }

    /// Log once per `(mod, pipeline)` when an ineligible pipeline is submitted to
    /// `RenderLevelStage::OitTranslucent`.
    pub fn warn_oit_ineligible(&self, slot: u32) {
        let slot = slot as usize;
        let mut logged = self
            .oit_ineligible_logged
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if logged.contains(&slot) {
            return;
        }
        logged.push(slot);
        if let Some(Some(built)) = self.built.get(slot) {
            let reason = if built.state.blend == PipelineBlend::Opaque {
                "Opaque pipelines cannot draw at OitTranslucent"
            } else {
                "pipeline was not registered with .order_independent()"
            };
            log::warn!(
                "Sunrise: mod {} pipeline {} refused at OitTranslucent: {reason}",
                built.owner,
                built.id
            );
        }
    }

    /// Marks `slot` as one the GPU refused; false when it already was, so it is reported once.
    pub(crate) fn invalidate(&self, slot: usize) -> bool {
        let mut invalid = self.invalid.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = !invalid.contains(&slot);
        if fresh {
            invalid.push(slot);
        }
        fresh
    }

    /// Makes `slot` usable again: a renderer that built it took the refusal of another over.
    pub(crate) fn revalidate(&self, slot: usize) {
        self.invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|&invalid| invalid != slot);
    }

    /// The load this set belongs to: a renderer holding another builds its pipelines again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every slot in order, with what the load built for it.
    pub fn built(&self) -> &[Option<Built>] {
        &self.built
    }

    /// The handle of the registered pipeline `id` in this load.
    pub fn resolve(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        let slot = self
            .keys
            .iter()
            .position(|registered| registered == id)
            .ok_or(RenderPipelineError::Unknown)?;
        if self.built[slot].is_none()
            || self
                .invalid
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&slot)
        {
            return Err(RenderPipelineError::Invalid);
        }
        Ok(mod_api::__private::built::render_pipeline_ref(
            self.generation,
            slot as u32,
        ))
    }

    /// The slot a handle draws with, checked once when a batch is submitted.
    pub fn slot(&self, pipeline: RenderPipelineRef) -> Result<u32, RenderPipelineError> {
        use mod_api::__private::game::Fields;
        let fields = pipeline.fields();
        if fields.generation != self.generation {
            return Err(RenderPipelineError::Stale);
        }
        if self
            .invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(fields.slot as usize))
        {
            return Err(RenderPipelineError::Invalid);
        }
        match self.built.get(fields.slot as usize) {
            Some(Some(_)) => Ok(fields.slot),
            Some(None) => Err(RenderPipelineError::Invalid),
            None => Err(RenderPipelineError::Unknown),
        }
    }

    /// Resolve a built-in ([`BUILTIN_RENDER_TYPE_IDS`]) or registered custom [`RenderSetup`]
    /// into a generational [`RenderTypeRef`] for this resource load.
    pub fn resolve_render_type(&self, id: &Identifier) -> Result<RenderTypeRef, RenderTypeError> {
        self.resolve_render_type_in(registered_render_types(), id)
    }

    pub(crate) fn resolve_render_type_in(
        &self,
        custom: &[RenderSetup],
        id: &Identifier,
    ) -> Result<RenderTypeRef, RenderTypeError> {
        if self.generation == 0 {
            return Err(RenderTypeError::Unknown);
        }
        if let Some(slot) = builtin_render_type_slot(id) {
            return Ok(mod_api::__private::built::render_type_ref(
                self.generation,
                slot,
            ));
        }
        let index = custom
            .iter()
            .position(|setup| &setup.id == id)
            .ok_or(RenderTypeError::Unknown)?;
        let setup = &custom[index];
        // If backed by a registered mod pipeline, reflect its build validity in this load.
        if let Some(pipeline_slot) = self.keys.iter().position(|k| k == &setup.pipeline)
            && (self.built[pipeline_slot].is_none()
                || self
                    .invalid
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(&pipeline_slot))
        {
            return Err(RenderTypeError::Invalid);
        }
        Ok(mod_api::__private::built::render_type_ref(
            self.generation,
            (BUILTIN_RENDER_TYPE_IDS.len() + index) as u32,
        ))
    }

    /// The handle of the built-in entity render type `kind` (`EntitySolid`, `EntityCutout` or
    /// `EntityTranslucent`; another built-in takes no texture and is
    /// [`RenderTypeError::Invalid`]) drawing the dynamic texture `texture`, as vanilla's
    /// `RenderTypes.entitySolid(Identifier)` and its siblings are memoized per texture
    /// (`RenderTypes.java`): the same texture gives the same handle. It does not ask whether the
    /// texture exists; a draw does.
    pub fn textured_render_type(
        &self,
        kind: BuiltinRenderType,
        texture: &Identifier,
    ) -> Result<RenderTypeRef, RenderTypeError> {
        self.textured_render_type_in(registered_render_types().len(), kind, texture)
    }

    pub(crate) fn textured_render_type_in(
        &self,
        custom: usize,
        kind: BuiltinRenderType,
        texture: &Identifier,
    ) -> Result<RenderTypeRef, RenderTypeError> {
        let key = match kind {
            BuiltinRenderType::EntitySolid => 3,
            BuiltinRenderType::EntityCutout => 4,
            BuiltinRenderType::EntityTranslucent => 5,
            _ => return Err(RenderTypeError::Invalid),
        };
        // An unpublished set has no generation to name a handle in.
        if self.generation == 0 {
            return Err(RenderTypeError::Unknown);
        }
        let mut table = self.textured.lock().unwrap_or_else(|e| e.into_inner());
        let index = match table.by_texture.get(&(key, texture.clone())) {
            Some(&index) => index,
            None => {
                let index =
                    u32::try_from(table.made.len()).map_err(|_| RenderTypeError::Invalid)?;
                table.made.push((kind, texture.clone()));
                table.by_texture.insert((key, texture.clone()), index);
                index
            }
        };
        let slot = (BUILTIN_RENDER_TYPE_IDS.len() + custom)
            .checked_add(index as usize)
            .and_then(|slot| u32::try_from(slot).ok())
            .ok_or(RenderTypeError::Invalid)?;
        Ok(mod_api::__private::built::render_type_ref(
            self.generation,
            slot,
        ))
    }

    /// The built-in entity render type and the dynamic texture a handle from
    /// [`Self::textured_render_type`] names: `Unknown` for any other handle (a plain built-in or
    /// a custom render type), `Stale` for one of an earlier load.
    pub fn textured_render_type_of(
        &self,
        handle: RenderTypeRef,
    ) -> Result<(BuiltinRenderType, Identifier), RenderTypeError> {
        self.textured_render_type_of_in(registered_render_types().len(), handle)
    }

    pub(crate) fn textured_render_type_of_in(
        &self,
        custom: usize,
        handle: RenderTypeRef,
    ) -> Result<(BuiltinRenderType, Identifier), RenderTypeError> {
        use mod_api::__private::game::Fields;
        let fields = handle.fields();
        if fields.generation != self.generation {
            return Err(RenderTypeError::Stale);
        }
        let index = (fields.slot as usize)
            .checked_sub(BUILTIN_RENDER_TYPE_IDS.len() + custom)
            .ok_or(RenderTypeError::Unknown)?;
        self.textured
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .made
            .get(index)
            .cloned()
            .ok_or(RenderTypeError::Unknown)
    }

    /// Validate a [`RenderTypeRef`] against this load and return its resolved descriptor.
    pub fn render_type_slot(
        &self,
        handle: RenderTypeRef,
    ) -> Result<ResolvedRenderType<'static>, RenderTypeError> {
        self.render_type_slot_in(registered_render_types(), handle)
    }

    pub(crate) fn render_type_slot_in<'a>(
        &self,
        custom: &'a [RenderSetup],
        handle: RenderTypeRef,
    ) -> Result<ResolvedRenderType<'a>, RenderTypeError> {
        use mod_api::__private::game::Fields;
        let fields = handle.fields();
        if fields.generation != self.generation {
            return Err(RenderTypeError::Stale);
        }
        if let Some(builtin) = BuiltinRenderType::from_slot(fields.slot) {
            return Ok(ResolvedRenderType::Builtin(builtin));
        }
        let Some(index) = (fields.slot as usize).checked_sub(BUILTIN_RENDER_TYPE_IDS.len()) else {
            return Err(RenderTypeError::Unknown);
        };
        let setup = custom.get(index).ok_or(RenderTypeError::Unknown)?;
        let mod_pipeline_slot =
            if let Some(pipeline_slot) = self.keys.iter().position(|k| k == &setup.pipeline) {
                if self.built[pipeline_slot].is_none()
                    || self
                        .invalid
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .contains(&pipeline_slot)
                {
                    return Err(RenderTypeError::Invalid);
                }
                Some(pipeline_slot as u32)
            } else {
                None
            };
        Ok(ResolvedRenderType::Custom {
            slot: index as u32,
            setup,
            mod_pipeline_slot,
        })
    }
}

/// `mymod:core/beam/glow` is `assets/mymod/shaders/core/beam/glow.<ext>`.
fn shader_file(shader: &Identifier, ext: &str) -> String {
    format!(
        "assets/{}/shaders/{}.{ext}",
        shader.namespace(),
        shader.path()
    )
}

/// Include base folder for `mymod:core/beam/glow`: `mymod:shaders/core/beam/`.
fn shader_dir(shader: &Identifier) -> String {
    let path = shader.path();
    let folder = path.rfind('/').map_or("", |i| &path[..=i]);
    format!("{}:shaders/{folder}", shader.namespace())
}

/// `texture` as a load finds it, or its named refusal.
fn load_texture(texture: &PipelineTexture) -> Result<BuiltTexture, PipelineTextureError> {
    match texture {
        PipelineTexture::Sprite(id) => {
            // A file's path or name is not a sprite key.
            let path = id.path();
            if path.starts_with("textures/") || path.ends_with(".png") {
                return Err(PipelineTextureError::NotASpriteKey);
            }
            // Named as the 2D atlas names the blocks and items atlases' sprites.
            Ok(BuiltTexture::Sprite(match id.namespace() {
                "minecraft" => path.to_owned(),
                _ => id.to_string(),
            }))
        }
        // A texture kind a later API adds is one this game has no route for yet.
        _ => Err(PipelineTextureError::NoSuchTextureFile),
    }
}

/// The diagnostics line of a refused texture: its named cause, then what it means.
pub fn texture_refusal(cause: PipelineTextureError) -> String {
    format!("{cause:?}: {cause}")
}

/// Convert a mod-API [`PackUniformType`] to `sunrise-render-shaderpack`'s [`ModUniformType`].
pub fn mod_uniform_type(
    ty: PackUniformType,
) -> sunrise_render_shaderpack::pack_post::ModUniformType {
    use sunrise_render_shaderpack::pack_post::ModUniformType as M;
    match ty {
        PackUniformType::Float => M::Float,
        PackUniformType::Int => M::Int,
        PackUniformType::Vec2 => M::Vec2,
        PackUniformType::Vec3 => M::Vec3,
        PackUniformType::Vec4 => M::Vec4,
        PackUniformType::Mat3 => M::Mat3,
        PackUniformType::Mat4 => M::Mat4,
        _ => M::Float,
    }
}

/// Convert a mod-API [`PackUniformValue`] to `sunrise-render-shaderpack`'s [`ModUniformValue`].
pub fn mod_uniform_value(
    value: PackUniformValue,
) -> sunrise_render_shaderpack::pack_post::ModUniformValue {
    use sunrise_render_shaderpack::pack_post::ModUniformValue as M;
    match value {
        PackUniformValue::Float(v) => M::Float(v),
        PackUniformValue::Int(v) => M::Int(v),
        PackUniformValue::Vec2(v) => M::Vec2(v),
        PackUniformValue::Vec3(v) => M::Vec3(v),
        PackUniformValue::Vec4(v) => M::Vec4(v),
        PackUniformValue::Mat3(v) => M::Mat3(v),
        PackUniformValue::Mat4(v) => M::Mat4(v),
        _ => M::Float(0.0),
    }
}

fn mod_pass_slot(slot: PackPassSlot) -> sunrise_render_shaderpack::pack_post::ModPassSlot {
    use sunrise_render_shaderpack::pack_post::ModPassSlot as S;
    match slot {
        PackPassSlot::Shadow => S::Shadow,
        PackPassSlot::AfterSky => S::AfterSky,
        PackPassSlot::AfterOpaqueGbuffers => S::AfterOpaqueGbuffers,
        PackPassSlot::AfterTranslucentGbuffers => S::AfterTranslucentGbuffers,
        PackPassSlot::AfterWeather => S::AfterWeather,
        _ => S::AfterOpaqueGbuffers,
    }
}

fn shader_group_for_slot(slot: PackPassSlot) -> sunrise_shaders::Group {
    match slot {
        PackPassSlot::Shadow => sunrise_shaders::Group::Shadow,
        PackPassSlot::AfterSky | PackPassSlot::AfterOpaqueGbuffers => {
            sunrise_shaders::Group::Gbuffers
        }
        PackPassSlot::AfterTranslucentGbuffers | PackPassSlot::AfterWeather => {
            sunrise_shaders::Group::Translucent
        }
        _ => sunrise_shaders::Group::Gbuffers,
    }
}

/// A shader-pack blend factor, or `None` for a `Constant*` factor, which registration refuses
/// (`RenderPipeline::validate_state`, `PipelineModifier::validate`).
fn pack_blend_factor(factor: BlendFactor) -> Option<sunrise_shaders::Factor> {
    use sunrise_shaders::Factor;
    Some(match factor {
        BlendFactor::Zero => Factor::Zero,
        BlendFactor::One => Factor::One,
        BlendFactor::SrcColor => Factor::SrcColor,
        BlendFactor::OneMinusSrcColor => Factor::OneMinusSrcColor,
        BlendFactor::DstColor => Factor::DstColor,
        BlendFactor::OneMinusDstColor => Factor::OneMinusDstColor,
        BlendFactor::SrcAlpha => Factor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => Factor::OneMinusSrcAlpha,
        BlendFactor::DstAlpha => Factor::DstAlpha,
        BlendFactor::OneMinusDstAlpha => Factor::OneMinusDstAlpha,
        BlendFactor::SrcAlphaSaturate => Factor::SrcAlphaSaturate,
        _ => return None,
    })
}

/// The shader-pack blend of a registered pipeline's blend mode; `None` draws without blending.
/// A custom blend whose factors the pack table cannot express has been refused at registration,
/// so it never reaches here.
fn pack_blend_for(blend: PipelineBlend) -> Option<sunrise_shaders::Blend> {
    let function = blend.to_blend_function()?;
    Some([
        pack_blend_factor(function.color.src)?,
        pack_blend_factor(function.color.dst)?,
        pack_blend_factor(function.alpha.src)?,
        pack_blend_factor(function.alpha.dst)?,
    ])
}

fn glsl_input_expr(input: &PackGlslInput) -> String {
    match (input.source.as_str(), input.ty) {
        ("position" | "pos", PackUniformType::Vec3) => "mod_position".to_owned(),
        ("position" | "pos", PackUniformType::Vec4) => "vec4(mod_position, 1.0)".to_owned(),
        ("uv" | "uv0", PackUniformType::Vec2) => "mod_uv".to_owned(),
        ("uv" | "uv0", PackUniformType::Vec4) => "vec4(mod_uv, 0.0, 1.0)".to_owned(),
        ("color", PackUniformType::Vec4) => "mod_color".to_owned(),
        ("color", PackUniformType::Vec3) => "mod_color.rgb".to_owned(),
        ("normal", PackUniformType::Vec3) => "vec3(0.0, 1.0, 0.0)".to_owned(),
        ("light" | "uv2", PackUniformType::Vec2) => "vec2(240.0, 240.0)".to_owned(),
        (_, PackUniformType::Float) => "0.0".to_owned(),
        (_, PackUniformType::Int) => "0".to_owned(),
        (_, PackUniformType::Vec2) => "vec2(0.0)".to_owned(),
        (_, PackUniformType::Vec3) => "vec3(0.0)".to_owned(),
        (_, PackUniformType::Vec4) => "vec4(0.0)".to_owned(),
        (_, PackUniformType::Mat3) => "mat3(1.0)".to_owned(),
        (_, PackUniformType::Mat4) => "mat4(1.0)".to_owned(),
        _ => "0.0".to_owned(),
    }
}

fn mod_pipeline_gbuffers_inputs(
    binding: Option<&mod_api::PackBinding>,
) -> sunrise_shaders::GbuffersInputs {
    let mut attributes = vec![
        ("vaPosition".to_owned(), "mod_position".to_owned()),
        ("vaColor".to_owned(), "mod_color".to_owned()),
        ("vaUV0".to_owned(), "mod_uv".to_owned()),
        ("vaUV2".to_owned(), "ivec2(240, 240)".to_owned()),
        ("vaNormal".to_owned(), "vec3(0.0, 1.0, 0.0)".to_owned()),
    ];
    if let Some(binding) = binding {
        for input in &binding.glsl_bindings {
            let expr = glsl_input_expr(input);
            if let Some(existing) = attributes.iter_mut().find(|(k, _)| k == &input.glsl_name) {
                existing.1 = expr;
            } else {
                attributes.push((input.glsl_name.clone(), expr));
            }
        }
    }
    sunrise_shaders::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 mod_position;\n\
             layout(location = 1) in vec2 mod_uv;\n\
             layout(location = 2) in vec4 mod_color;\n"
            .to_owned(),
        body: "    hz_Vertex = vec4(mod_position, 1.0);\n\
             hz_Color = mod_color;\n\
             hz_MultiTexCoord0 = vec4(mod_uv, 0.0, 1.0);\n\
             hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n\
             hz_MultiTexCoord2 = hz_MultiTexCoord1;\n\
             hz_Normal = vec3(0.0, 1.0, 0.0);\n"
            .to_owned(),
        attributes,
        ..Default::default()
    }
}

/// Deduplicated shader-pack preprocessor defines across all registered pipelines.
pub fn pack_defines() -> Vec<(&'static str, &'static str)> {
    pack_defines_of(registered())
}

pub(crate) fn pack_defines_of(pipelines: &[RenderPipeline]) -> Vec<(&str, &str)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for pipeline in pipelines {
        let Some(binding) = &pipeline.pack_binding else {
            continue;
        };
        for (name, value) in &binding.defines {
            if seen.insert(name.as_str()) {
                out.push((name.as_str(), value.as_deref().unwrap_or("")));
            }
        }
    }
    out
}

/// Shader-pack program resolution requests for every registered pipeline with a pack slot
/// and candidate program chain.
pub fn pack_program_requests() -> Vec<sunrise_shaders::ModPackProgramRequest> {
    pack_program_requests_of(registered())
}

pub(crate) fn pack_program_requests_of(
    pipelines: &[RenderPipeline],
) -> Vec<sunrise_shaders::ModPackProgramRequest> {
    let mut out = Vec::new();
    for (slot, pipeline) in pipelines.iter().enumerate() {
        let Some(pass_slot) = pipeline.effective_pack_slot() else {
            continue;
        };
        let programs: Vec<String> = pipeline
            .effective_pack_programs()
            .into_iter()
            .map(str::to_owned)
            .collect();
        if programs.is_empty() {
            continue;
        }
        out.push(sunrise_shaders::ModPackProgramRequest {
            slot,
            programs,
            group: shader_group_for_slot(pass_slot),
            inputs: mod_pipeline_gbuffers_inputs(pipeline.pack_binding.as_ref()),
            default_blend: match pass_slot {
                PackPassSlot::AfterOpaqueGbuffers | PackPassSlot::Shadow => None,
                PackPassSlot::AfterSky
                | PackPassSlot::AfterTranslucentGbuffers
                | PackPassSlot::AfterWeather => pack_blend_for(pipeline.state.blend),
                _ => None,
            },
        });
    }
    out
}

pub(crate) fn wgpu_compare(compare: DepthCompare) -> wgpu::CompareFunction {
    match compare {
        DepthCompare::Always => wgpu::CompareFunction::Always,
        DepthCompare::Less => wgpu::CompareFunction::Less,
        DepthCompare::LessEqual => wgpu::CompareFunction::LessEqual,
        DepthCompare::Equal => wgpu::CompareFunction::Equal,
        DepthCompare::NotEqual => wgpu::CompareFunction::NotEqual,
        DepthCompare::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
        DepthCompare::Greater => wgpu::CompareFunction::Greater,
        DepthCompare::Never => wgpu::CompareFunction::Never,
        _ => wgpu::CompareFunction::GreaterEqual,
    }
}

/// Owned runtime descriptors (`depth_targets`, `uniforms`, `pipelines`) for `PackPost`.
pub fn pack_runtime_specs() -> sunrise_render_shaderpack::pack_post::OwnedModPackRuntime {
    pack_runtime_specs_of(registered())
}

pub(crate) fn pack_runtime_specs_of(
    pipelines: &[RenderPipeline],
) -> sunrise_render_shaderpack::pack_post::OwnedModPackRuntime {
    use sunrise_render_shaderpack::pack_post::{
        ModDepthTargetDecl, ModPipelinePackSpec, ModProgramMode, ModUniformDecl,
        OwnedModPackRuntime,
    };
    let mut depth_targets = Vec::new();
    let mut depth_target_ids: Vec<Identifier> = Vec::new();
    let mut uniforms = Vec::new();
    let mut seen_uniforms = HashSet::new();
    let mut specs = Vec::new();

    for (slot, pipeline) in pipelines.iter().enumerate() {
        if let Some(binding) = &pipeline.pack_binding {
            for uniform in &binding.frame_uniforms {
                if seen_uniforms.insert(uniform.name.as_str()) {
                    uniforms.push(ModUniformDecl {
                        name: uniform.name.clone(),
                        ty: mod_uniform_type(uniform.ty),
                        previous_name: uniform.previous_name.clone(),
                    });
                }
            }
        }
        let Some(pass_slot) = pipeline.effective_pack_slot() else {
            continue;
        };
        if pipeline.effective_pack_programs().is_empty() {
            continue;
        }
        let depth_target = pipeline
            .pack_binding
            .as_ref()
            .and_then(|b| b.depth_target.as_ref())
            .and_then(|target| match target {
                mod_api::PackDepthTarget::Aux {
                    id,
                    depth_samplers,
                    opaque_depth_samplers,
                    ..
                } => Some(
                    if let Some(idx) = depth_target_ids.iter().position(|seen| seen == id) {
                        idx
                    } else {
                        let idx = depth_targets.len();
                        depth_target_ids.push(id.clone());
                        depth_targets.push(ModDepthTargetDecl {
                            label: id.to_string(),
                            depth_samplers: depth_samplers.clone(),
                            opaque_depth_samplers: opaque_depth_samplers.clone(),
                        });
                        idx
                    },
                ),
                // The pack's main depth: no auxiliary plane.
                _ => None,
            });
        let depth_state = pipeline.state.depth_stencil;
        let depth_compare = depth_state.map_or(wgpu::CompareFunction::Always, |ds| {
            wgpu_compare(ds.compare.flip_for_standard_z())
        });
        let names_main_depth = matches!(
            pipeline
                .pack_binding
                .as_ref()
                .and_then(|b| b.depth_target.as_ref()),
            Some(mod_api::PackDepthTarget::Main)
        );
        let depth_write = depth_state.is_some_and(|ds| ds.write)
            && (depth_target.is_some()
                || names_main_depth
                || matches!(
                    pass_slot,
                    PackPassSlot::AfterOpaqueGbuffers | PackPassSlot::Shadow
                ));
        specs.push(ModPipelinePackSpec {
            slot,
            pass_slot: mod_pass_slot(pass_slot),
            depth_target,
            depth_compare,
            depth_write,
            cull: pipeline.state.cull,
            program_mode: pipeline
                .pack_binding
                .as_ref()
                .map_or(ModProgramMode::default(), |b| {
                    mod_program_mode(b.program_mode)
                }),
        });
    }

    OwnedModPackRuntime {
        depth_targets,
        uniforms,
        pipelines: specs,
    }
}

fn mod_program_mode(
    mode: mod_api::PackProgramMode,
) -> sunrise_render_shaderpack::pack_post::ModProgramMode {
    use sunrise_render_shaderpack::pack_post::ModProgramMode as M;
    match mode {
        mod_api::PackProgramMode::ModVertexPackFragment => M::ModVertexPackFragment,
        _ => M::PackVertexAndFragment,
    }
}

fn pack_uniform_map(pipelines: &[RenderPipeline]) -> HashMap<&str, PackUniformType> {
    let mut map = HashMap::new();
    for pipeline in pipelines {
        let Some(binding) = &pipeline.pack_binding else {
            continue;
        };
        for uniform in &binding.frame_uniforms {
            map.entry(uniform.name.as_str()).or_insert(uniform.ty);
        }
    }
    map
}

/// Look up the registered `'static` name and [`PackUniformType`] of a pack frame uniform in O(1)
/// without per-frame allocation.
pub fn lookup_pack_uniform(name: &str) -> Option<(&'static str, PackUniformType)> {
    let pipelines = REGISTERED.get()?;
    let table = PACK_UNIFORMS.get_or_init(|| pack_uniform_map(pipelines));
    table.get_key_value(name).map(|(&k, &ty)| (k, ty))
}

/// Look up the declared [`PackUniformType`] of a registered pack frame uniform by name.
pub fn lookup_pack_uniform_type(name: &str) -> Option<PackUniformType> {
    lookup_pack_uniform(name).map(|(_, ty)| ty)
}

#[cfg(test)]
pub(crate) fn lookup_pack_uniform_type_in(
    pipelines: &[RenderPipeline],
    name: &str,
) -> Option<PackUniformType> {
    pack_uniform_map(pipelines).get(name).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::WorldDepth;

    const VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";

    const GLOW_FSH: &str = "\
#version 330
#include <minecraft:globals.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vec4(vertexColor.rgb * (1.0 + sin(GameTime)), vertexColor.a * texCoord0.y);
}
";

    fn stub_asset(file: &str) -> Option<String> {
        let rest = file.strip_prefix("assets/")?;
        let (ns, path) = rest.split_once('/')?;
        sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned)
    }

    fn pipeline(id: &str, shader: &str) -> RenderPipeline {
        RenderPipeline::new(
            Identifier::parse(id).unwrap(),
            Identifier::parse(shader).unwrap(),
            PipelineBlend::Additive,
            WorldDepth::ThroughWalls,
        )
    }
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn an_optional_pipeline_that_fails_leaves_the_others_drawing_and_refuses_its_own_use() {
        let registered = [
            pipeline("example:glow", "example:core/glow"),
            pipeline("example:broken", "example:core/broken").optional(),
            pipeline("example:missing", "example:core/nested/missing").optional(),
            pipeline("example:driver", "example:core/glow").optional(),
        ];
        let read = |file: &str| match file {
            "assets/example/shaders/core/glow.vsh" => Ok(Some(VSH.to_owned())),
            "assets/example/shaders/core/glow.fsh" => Ok(Some(GLOW_FSH.to_owned())),
            "assets/example/shaders/core/broken.vsh" => Ok(Some(VSH.to_owned())),
            "assets/example/shaders/core/broken.fsh" => Ok(Some("void nothing() {}".to_owned())),
            "assets/example/shaders/core/nested/missing.vsh" => Ok(None),
            other => Ok(stub_asset(other)),
        };
        let set = Pipelines::build(&registered, 7, read).unwrap();

        let glow = set.resolve(&id("example:glow")).unwrap();
        assert_eq!(set.slot(glow), Ok(0));
        let built = set.built()[0].as_ref().unwrap();
        assert_eq!(
            (
                built.state.blend,
                built.state.depth_stencil.map(|ds| ds.compare)
            ),
            (PipelineBlend::Additive, Some(DepthCompare::Always))
        );
        assert_eq!(
            set.resolve(&id("example:broken")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:missing")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:never")),
            Err(RenderPipelineError::Unknown)
        );

        // The GPU turns down a pipeline the load built: it is reported once, a handle taken
        // before is refused, and the rest still draw.
        let driver = set.resolve(&id("example:driver")).unwrap();
        assert!(set.invalidate(3));
        assert!(!set.invalidate(3));
        assert_eq!(set.slot(driver), Err(RenderPipelineError::Invalid));
        assert_eq!(
            set.resolve(&id("example:driver")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(set.slot(glow), Ok(0));
    }

    #[test]
    fn a_required_pipeline_that_fails_fails_the_load_by_name() {
        let registered = [
            pipeline("example:glow", "example:core/glow"),
            pipeline("example:beam", "example:core/beam"),
            pipeline("example:extra", "example:core/extra").optional(),
        ];
        let all_good = |file: &str| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(GLOW_FSH.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        };
        let before = Pipelines::build(&registered, 7, all_good).unwrap();
        let handle = before.resolve(&id("example:beam")).unwrap();

        // A pack broke the required beam and the optional extra: the load fails naming the
        // required one only, so nothing of it is published and the previous load still draws.
        let error = Pipelines::build(&registered, 8, |file| match file {
            "assets/example/shaders/core/glow.vsh"
            | "assets/example/shaders/core/beam.vsh"
            | "assets/example/shaders/core/extra.vsh" => Ok(Some(VSH.to_owned())),
            "assets/example/shaders/core/glow.fsh" => Ok(Some(GLOW_FSH.to_owned())),
            "assets/example/shaders/core/beam.fsh" | "assets/example/shaders/core/extra.fsh" => {
                Ok(Some("garbage".to_owned()))
            }
            other => Ok(stub_asset(other)),
        })
        .unwrap_err();
        assert!(
            error.starts_with(
                "required render pipeline example:beam from mod  refused: assets/example/shaders/core/beam.fsh:"
            ),
            "{error}"
        );
        assert!(!error.contains("example:glow"), "{error}");
        assert!(!error.contains("example:extra"), "{error}");
        assert_eq!(before.slot(handle), Ok(1));
    }

    /// `RenderTypes.entitySolid(Identifier)` and its siblings are memoized per texture: one
    /// texture gives one handle, another texture another, and the handle names its texture and
    /// kind back. Only the three entity kinds take a texture.
    #[test]
    fn a_textured_render_type_is_one_handle_per_kind_and_texture() {
        let set = Pipelines::build(&[], 7, |_| Ok(None)).unwrap();
        let (a, b) = (id("example:dynamic/a"), id("example:dynamic/b"));
        let solid =
            |texture| set.textured_render_type_in(0, BuiltinRenderType::EntitySolid, texture);
        assert_eq!(solid(&a), solid(&a));
        assert_ne!(solid(&a), solid(&b));
        let cutout = set.textured_render_type_in(0, BuiltinRenderType::EntityCutout, &a);
        assert_ne!(cutout, solid(&a));
        assert_eq!(
            set.textured_render_type_of_in(0, cutout.unwrap()),
            Ok((BuiltinRenderType::EntityCutout, a.clone()))
        );
        assert_eq!(
            set.textured_render_type_of_in(0, solid(&b).unwrap()),
            Ok((BuiltinRenderType::EntitySolid, b))
        );
        assert_eq!(
            set.textured_render_type_in(0, BuiltinRenderType::Lightning, &a),
            Err(RenderTypeError::Invalid)
        );
        // A plain built-in handle names no texture, and a handle of another load is stale.
        let plain = mod_api::__private::built::render_type_ref(7, 0);
        assert_eq!(
            set.textured_render_type_of_in(0, plain),
            Err(RenderTypeError::Unknown)
        );
        let earlier = mod_api::__private::built::render_type_ref(6, 40);
        assert_eq!(
            set.textured_render_type_of_in(0, earlier),
            Err(RenderTypeError::Stale)
        );
    }

    #[test]
    fn initial_required_failure_has_a_literal_message() {
        let registered = [pipeline("example:glow", "example:core/glow")];
        let error = Pipelines::build(&registered, 7, |_| Ok(None)).unwrap_err();
        assert_eq!(
            error,
            "required render pipeline example:glow from mod  refused: assets/example/shaders/core/glow.vsh: not found"
        );
    }

    #[test]
    fn a_reload_makes_old_handles_stale_and_can_fix_or_break_a_pipeline() {
        let registered = [pipeline("example:glow", "example:core/glow").optional()];
        let good = |file: &str| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(GLOW_FSH.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        };
        let before = Pipelines::build(&registered, 7, good).unwrap();
        let handle = before.resolve(&id("example:glow")).unwrap();

        // A pack replaced the file with one that doesn't compile: the pipeline is invalid in
        // the new load, and the old handle is stale there, not silently redirected.
        let broken = Pipelines::build(&registered, 8, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some("garbage".to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();
        assert_eq!(broken.slot(handle), Err(RenderPipelineError::Stale));
        assert_eq!(
            broken.resolve(&id("example:glow")),
            Err(RenderPipelineError::Invalid)
        );

        let fixed = Pipelines::build(&registered, 9, good).unwrap();
        assert_eq!(fixed.slot(handle), Err(RenderPipelineError::Stale));
        let again = fixed.resolve(&id("example:glow")).unwrap();
        assert_eq!(fixed.slot(again), Ok(0));
    }

    /// Separate `.vertex_shader(...)`, custom defines, and shared `#include <ns:path>` /
    /// `#moj_import` files all resolve through the resource stack.
    #[test]
    fn vertex_shader_reuse_defines_and_shared_includes_build() {
        let shared_inc = "vec3 palette(float t) { return vec3(t, 1.0 - t, 0.5); }\n";
        let fsh = "\
#version 330
#include <libmod:palette.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
#ifdef BRIGHT
    fragColor = vec4(palette(texCoord0.x) * float(SCALE), vertexColor.a);
#else
    fragColor = vertexColor;
#endif
}
";
        let registered = [
            pipeline("app:a", "app:core/fx")
                .vertex_shader(id("minecraft:core/position_tex_color"))
                .define("BRIGHT")
                .define_value("SCALE", "2"),
            pipeline("app:b", "app:core/fx").vertex_shader(id("minecraft:core/position_tex_color")),
        ];
        let set = Pipelines::build(&registered, 1, |file| match file {
            "assets/app/shaders/core/fx.fsh" => Ok(Some(fsh.to_owned())),
            "assets/libmod/shaders/include/palette.glsl" => Ok(Some(shared_inc.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        assert!(set.built()[0].is_some());
        assert!(set.built()[1].is_some());
        assert_eq!(
            set.built()[0].as_ref().unwrap().defines.as_ref(),
            ["BRIGHT", "SCALE 2"]
        );
        assert!(set.built()[1].as_ref().unwrap().defines.is_empty());
    }

    /// Sprites are named as the 2D atlas names them; an id naming a file is refused by name,
    /// and a required pipeline's refusal fails the load with the named cause.
    #[test]
    fn a_textured_pipeline_names_its_sprite_and_refuses_a_file_path() {
        use mod_api::PipelineTexture;
        let pipeline = |id: &str| {
            let name = Identifier::parse("test:core/glow").unwrap();
            RenderPipeline::new(
                Identifier::parse("test:glow").unwrap(),
                name,
                PipelineBlend::Additive,
                WorldDepth::Tested,
            )
            .textured(PipelineTexture::Sprite(Identifier::parse(id).unwrap()))
        };
        let shader = |file: &str| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(GLOW_FSH.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        };
        let built = |id| {
            Pipelines::build(&[pipeline(id)], 1, shader)
                .map(|set| set.built()[0].as_ref().unwrap().texture.clone().unwrap())
        };
        assert_eq!(
            built("minecraft:block/stone"),
            Ok(BuiltTexture::Sprite("block/stone".into()))
        );
        assert_eq!(
            built("powah:block/energy_charge"),
            Ok(BuiltTexture::Sprite("powah:block/energy_charge".into()))
        );
        assert_eq!(
            load_texture(&PipelineTexture::Sprite(
                Identifier::parse("test:textures/glow").unwrap()
            )),
            Err(PipelineTextureError::NotASpriteKey)
        );
        assert_eq!(
            built("test:effect/glow.png"),
            Err("required render pipeline test:glow from mod  refused: its texture: NotASpriteKey: the id names a file, not a sprite key".into())
        );
    }

    #[test]
    fn register_validates_pack_bindings_collisions_reserved_names_and_ordering() {
        use mod_api::{
            Hooks, ModEntry, PackBinding, PackDepthTarget, PackPassSlot, PackUniformDecl,
            PackUniformType, PipelineGeometryClass, RegisterRenderPipelinesEvent,
            RegistrationContext,
        };

        struct Custom(Box<dyn Fn(&mut RegisterRenderPipelinesEvent<'_>) + Send + Sync>);
        #[mod_api::__subscriptions]
        impl Hooks for Custom {
            fn register_render_pipelines(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterRenderPipelinesEvent<'_>,
            ) {
                (self.0)(event);
            }
        }

        let run = |mods: Vec<ModEntry>| -> Result<Vec<RenderPipeline>, String> {
            let mut result = Err(String::new());
            mod_platform::testing::with_mods(mods.into(), || {
                let mut staged = Vec::new();
                let mut staged_render_types = Vec::new();
                let mut staged_compute = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &RegistrationContext::new(),
                    &mut staged_render_types,
                    &mut staged_compute,
                    &mut staged,
                );
                result = validate_pack_bindings(&staged).map(|()| staged);
            });
            result
        };

        // Same mod sharing one auxiliary depth target, uniform, and define across opaque +
        // translucent pipelines succeeds, and preserves pack pass ordering.
        let shared_target = || {
            PackDepthTarget::new(id("alpha:aux_depth"))
                .depth_sampler("alphaDepthTex0")
                .opaque_depth_sampler("alphaDepthTex1")
        };
        let ok = run(vec![
            ModEntry::new(
                "alpha",
                Box::new(Custom(Box::new(move |ev| {
                    ev.register(
                        pipeline("alpha:opaque", "alpha:opaque")
                            .geometry_class(PipelineGeometryClass::Terrain)
                            .pack_binding(
                                PackBinding::new()
                                    .program("alpha_terrain")
                                    .slot(PackPassSlot::AfterOpaqueGbuffers)
                                    .define("ALPHA_LAYER")
                                    .define_value("ALPHA_MAT_LEAF", "1")
                                    .depth_target(shared_target())
                                    .frame_uniform(
                                        PackUniformDecl::new(
                                            "alphaProjection",
                                            PackUniformType::Mat4,
                                        )
                                        .with_previous("alphaPreviousProjection"),
                                    ),
                            ),
                    );
                    ev.register(
                        pipeline("alpha:water", "alpha:water")
                            .geometry_class(PipelineGeometryClass::Translucent)
                            .pack_binding(
                                PackBinding::new()
                                    .program("alpha_water")
                                    .program("alpha_terrain")
                                    .slot(PackPassSlot::AfterTranslucentGbuffers)
                                    .define("ALPHA_LAYER")
                                    .depth_target(shared_target())
                                    .frame_uniform(
                                        PackUniformDecl::new(
                                            "alphaProjection",
                                            PackUniformType::Mat4,
                                        )
                                        .with_previous("alphaPreviousProjection"),
                                    ),
                            ),
                    );
                }))),
            ),
            ModEntry::new(
                "beta",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("beta:contraption", "beta:contraption")
                            .geometry_class(PipelineGeometryClass::Entity),
                    );
                }))),
            ),
        ])
        .unwrap();
        assert_eq!(ok.len(), 3);
        assert_eq!(
            ok[0].effective_pack_programs(),
            [
                "alpha_terrain",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            ok[1].effective_pack_programs(),
            [
                "alpha_water",
                "alpha_terrain",
                "gbuffers_water",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            ok[2].effective_pack_programs(),
            [
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert!(ok[0].effective_pack_slot().unwrap() < ok[1].effective_pack_slot().unwrap());

        // Cross-mod sampler collision names both mods and the colliding sampler.
        let sampler_err = run(vec![
            ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new().depth_target(
                            PackDepthTarget::new(id("mod_one:d")).depth_sampler("sharedTex"),
                        ),
                    ));
                }))),
            ),
            ModEntry::new(
                "mod_two",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(pipeline("mod_two:p", "mod_two:p").pack_binding(
                        PackBinding::new().depth_target(
                            PackDepthTarget::new(id("mod_two:d")).opaque_depth_sampler("sharedTex"),
                        ),
                    ));
                }))),
            ),
        ])
        .unwrap_err();
        assert!(sampler_err.contains("sharedTex"), "{sampler_err}");
        assert!(sampler_err.contains("mod_one"), "{sampler_err}");
        assert!(sampler_err.contains("mod_two"), "{sampler_err}");

        // Cross-mod uniform collision (including previous_name) names both mods.
        let uniform_err = run(vec![
            ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("mod_one:p", "mod_one:p").pack_binding(
                            PackBinding::new().frame_uniform(
                                PackUniformDecl::new("customProj", PackUniformType::Mat4)
                                    .with_previous("customPrevProj"),
                            ),
                        ),
                    );
                }))),
            ),
            ModEntry::new(
                "mod_two",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(pipeline("mod_two:p", "mod_two:p").pack_binding(
                        PackBinding::new().frame_uniform(PackUniformDecl::new(
                            "customPrevProj",
                            PackUniformType::Mat4,
                        )),
                    ));
                }))),
            ),
        ])
        .unwrap_err();
        assert!(uniform_err.contains("customPrevProj"), "{uniform_err}");
        assert!(uniform_err.contains("mod_one"), "{uniform_err}");
        assert!(uniform_err.contains("mod_two"), "{uniform_err}");

        // Cross-mod define collision names both mods.
        let define_err = run(vec![
            ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("mod_one:p", "mod_one:p")
                            .pack_binding(PackBinding::new().define("SHARED_FLAG")),
                    );
                }))),
            ),
            ModEntry::new(
                "mod_two",
                Box::new(Custom(Box::new(|ev| {
                    ev.register(
                        pipeline("mod_two:p", "mod_two:p")
                            .pack_binding(PackBinding::new().define("SHARED_FLAG")),
                    );
                }))),
            ),
        ])
        .unwrap_err();
        assert!(define_err.contains("SHARED_FLAG"), "{define_err}");
        assert!(define_err.contains("mod_one"), "{define_err}");
        assert!(define_err.contains("mod_two"), "{define_err}");

        // Reserved vanilla/Iris samplers, built-in images, and uniforms are refused in either
        // sampler or uniform position (they share GLSL's `uniform` namespace).
        for reserved_sampler in [
            "depthtex0",
            "colortex0",
            "shadowtex0",
            "noisetex",
            "colorimg0",
            "colorimg15",
            "shadowcolorimg0",
            "shadowcolorimg1",
            "cameraPosition",
        ] {
            let err = run(vec![ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(move |ev| {
                    ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new().depth_target(
                            PackDepthTarget::new(id("mod_one:d")).depth_sampler(reserved_sampler),
                        ),
                    ));
                }))),
            )])
            .unwrap_err();
            assert!(err.contains(reserved_sampler), "{err}");
            assert!(err.contains("mod_one"), "{err}");
        }
        for reserved_uniform in [
            "gbufferProjection",
            "cameraPosition",
            "worldTime",
            "depthtex0",
            "colorimg0",
            "shadowcolorimg0",
        ] {
            let err = run(vec![ModEntry::new(
                "mod_one",
                Box::new(Custom(Box::new(move |ev| {
                    ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new().frame_uniform(PackUniformDecl::new(
                            reserved_uniform,
                            PackUniformType::Mat4,
                        )),
                    ));
                }))),
            )])
            .unwrap_err();
            assert!(err.contains(reserved_uniform), "{err}");
            assert!(err.contains("mod_one"), "{err}");
        }

        // Same identifier claimed as both a sampler and a uniform is refused (same-mod and cross-mod).
        let same_mod_ns_err = run(vec![ModEntry::new(
            "mod_one",
            Box::new(Custom(Box::new(|ev| {
                ev.register(
                    pipeline("mod_one:p", "mod_one:p").pack_binding(
                        PackBinding::new()
                            .depth_target(
                                PackDepthTarget::new(id("mod_one:d")).depth_sampler("sharedId"),
                            )
                            .frame_uniform(PackUniformDecl::new(
                                "sharedId",
                                PackUniformType::Float,
                            )),
                    ),
                );
            }))),
        )])
        .unwrap_err();
        assert!(same_mod_ns_err.contains("sharedId"), "{same_mod_ns_err}");
        assert!(same_mod_ns_err.contains("mod_one"), "{same_mod_ns_err}");

        let cross_mod_ns_err =
            run(vec![
                ModEntry::new(
                    "mod_one",
                    Box::new(Custom(Box::new(|ev| {
                        ev.register(pipeline("mod_one:p", "mod_one:p").pack_binding(
                            PackBinding::new().frame_uniform(PackUniformDecl::new(
                                "crossNs",
                                PackUniformType::Mat4,
                            )),
                        ));
                    }))),
                ),
                ModEntry::new(
                    "mod_two",
                    Box::new(Custom(Box::new(|ev| {
                        ev.register(pipeline("mod_two:p", "mod_two:p").pack_binding(
                            PackBinding::new().depth_target(
                                PackDepthTarget::new(id("mod_two:d")).depth_sampler("crossNs"),
                            ),
                        ));
                    }))),
                ),
            ])
            .unwrap_err();
        assert!(cross_mod_ns_err.contains("crossNs"), "{cross_mod_ns_err}");
        assert!(cross_mod_ns_err.contains("mod_one"), "{cross_mod_ns_err}");
        assert!(cross_mod_ns_err.contains("mod_two"), "{cross_mod_ns_err}");

        // Within-mod uniform role collision across two pipelines (`name` vs `previous_name`).
        let role_err = run(vec![ModEntry::new(
            "mod_one",
            Box::new(Custom(Box::new(|ev| {
                ev.register(pipeline("mod_one:a", "mod_one:a").pack_binding(
                    PackBinding::new().frame_uniform(
                        PackUniformDecl::new("projA", PackUniformType::Mat4).with_previous("projB"),
                    ),
                ));
                ev.register(
                    pipeline("mod_one:b", "mod_one:b").pack_binding(
                        PackBinding::new()
                            .frame_uniform(PackUniformDecl::new("projB", PackUniformType::Mat4)),
                    ),
                );
            }))),
        )])
        .unwrap_err();
        assert!(role_err.contains("projB"), "{role_err}");
        assert!(role_err.contains("mod_one"), "{role_err}");

        // Invalid define syntax is refused at registration.
        let bad_define = run(vec![ModEntry::new(
            "mod_one",
            Box::new(Custom(Box::new(|ev| {
                ev.register(
                    pipeline("mod_one:p", "mod_one:p")
                        .pack_binding(PackBinding::new().define_value("VALID", "1\n#define BAD")),
                );
            }))),
        )])
        .unwrap_err();
        assert!(bad_define.contains("mod_one"), "{bad_define}");
        assert!(bad_define.contains("VALID"), "{bad_define}");
    }

    #[test]
    fn pack_runtime_helpers_deduplicate_targets_uniforms_and_defines() {
        use mod_api::{
            PackBinding, PackDepthTarget, PackGlslInput, PackUniformDecl, PipelineGeometryClass,
        };
        let shared_depth = PackDepthTarget::new(id("example:aux_depth"))
            .depth_sampler("modDepthTex0")
            .opaque_depth_sampler("modDepthTex1");
        let opaque = RenderPipeline::new(
            id("example:opaque"),
            id("example:opaque"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .depth_write(true)
        .geometry_class(PipelineGeometryClass::Terrain)
        .pack_binding(
            PackBinding::new()
                .program("custom_terrain")
                .define("EXAMPLE_EXT")
                .define_value("EXAMPLE_LEVEL", "2")
                .depth_target(shared_depth.clone())
                .frame_uniform(
                    PackUniformDecl::new("modProj", PackUniformType::Mat4)
                        .with_previous("prevModProj"),
                )
                .glsl_binding(PackGlslInput::new(
                    "color",
                    "modColor",
                    PackUniformType::Vec4,
                )),
        );
        let water = RenderPipeline::new(
            id("example:water"),
            id("example:water"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .depth_write(true)
        .geometry_class(PipelineGeometryClass::Translucent)
        .pack_binding(
            PackBinding::new()
                .program("custom_water")
                .define("EXAMPLE_EXT")
                .depth_target(shared_depth)
                .frame_uniform(PackUniformDecl::new("modNear", PackUniformType::Float)),
        );
        let uniform_only = RenderPipeline::new(
            id("example:uniform_only"),
            id("example:uniform_only"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .pack_binding(
            PackBinding::new()
                .frame_uniform(PackUniformDecl::new("modFar", PackUniformType::Float)),
        );
        let plain = pipeline("example:plain", "example:plain");
        let list = [opaque, water, uniform_only, plain];

        let defines = pack_defines_of(&list);
        assert_eq!(defines, vec![("EXAMPLE_EXT", ""), ("EXAMPLE_LEVEL", "2")]);

        let reqs = pack_program_requests_of(&list);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].slot, 0);
        assert_eq!(reqs[0].programs[0], "custom_terrain");
        assert_eq!(reqs[0].programs[1], "gbuffers_terrain");
        assert_eq!(reqs[0].group, sunrise_shaders::Group::Gbuffers);
        assert_eq!(reqs[0].default_blend, None);
        assert!(
            reqs[0]
                .inputs
                .attributes
                .iter()
                .any(|(k, v)| k == "modColor" && v == "mod_color")
        );
        assert_eq!(reqs[1].slot, 1);
        assert_eq!(reqs[1].programs[0], "custom_water");
        assert_eq!(reqs[1].programs[1], "gbuffers_water");
        assert_eq!(reqs[1].group, sunrise_shaders::Group::Translucent);
        assert_eq!(
            reqs[1].default_blend,
            Some(sunrise_shaders::TRANSLUCENT_BLEND)
        );

        let runtime = pack_runtime_specs_of(&list);
        assert_eq!(runtime.depth_targets.len(), 1);
        assert_eq!(
            runtime.depth_targets[0].depth_samplers,
            vec!["modDepthTex0"]
        );
        assert_eq!(
            runtime.depth_targets[0].opaque_depth_samplers,
            vec!["modDepthTex1"]
        );
        assert_eq!(runtime.uniforms.len(), 3);
        assert!(runtime.uniforms.iter().any(|u| u.name == "modFar"));
        assert_eq!(runtime.pipelines.len(), 2);
        assert_eq!(runtime.pipelines[0].depth_target, Some(0));
        assert_eq!(runtime.pipelines[1].depth_target, Some(0));
        assert!(runtime.pipelines[0].depth_write);
        assert!(runtime.pipelines[1].depth_write);

        assert_eq!(
            lookup_pack_uniform_type_in(&list, "modProj"),
            Some(PackUniformType::Mat4)
        );
        assert_eq!(
            lookup_pack_uniform_type_in(&list, "modNear"),
            Some(PackUniformType::Float)
        );
        assert_eq!(lookup_pack_uniform_type_in(&list, "unknown"), None);
    }

    #[test]
    fn an_explicit_main_depth_target_allows_requested_writes_and_cull_reaches_the_pack_spec() {
        use mod_api::{PackBinding, PackDepthTarget, PackProgramMode, PipelineGeometryClass};
        use sunrise_render_shaderpack::pack_post::ModProgramMode;
        let translucent = |name: &str, binding: PackBinding, cull: bool| {
            RenderPipeline::new(
                id(name),
                id(name),
                PipelineBlend::Translucent,
                WorldDepth::Tested,
            )
            .geometry_class(PipelineGeometryClass::Translucent)
            .depth_write(true)
            .cull(cull)
            .pack_binding(binding)
        };
        let list = [
            // A translucent slot without an explicit target cannot write depth.
            translucent("example:plain", PackBinding::new(), false),
            translucent(
                "example:main",
                PackBinding::new().depth_target(PackDepthTarget::Main),
                true,
            ),
            translucent(
                "example:own_vertex",
                PackBinding::new().program_mode(PackProgramMode::ModVertexPackFragment),
                true,
            ),
        ];
        let runtime = pack_runtime_specs_of(&list);
        let [plain, main, own_vertex] = &runtime.pipelines[..] else {
            panic!("three pipelines expected");
        };
        assert!(!plain.depth_write && !plain.cull);
        assert_eq!(plain.program_mode, ModProgramMode::PackVertexAndFragment);
        assert!(main.depth_write);
        assert_eq!(main.depth_target, None);
        assert!(main.cull);
        assert_eq!(main.program_mode, ModProgramMode::PackVertexAndFragment);
        assert!(own_vertex.cull);
        assert_eq!(
            own_vertex.program_mode,
            ModProgramMode::ModVertexPackFragment
        );
    }

    #[test]
    fn modifier_stacks_intern_and_apply_outermost_wins_with_neoforge_labels() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{Hooks, ModEntry, RegisterPipelineModifiersEvent, RegistrationContext};

        struct ModRegistrar(Box<dyn Fn(&mut RegisterPipelineModifiersEvent<'_>) + Send + Sync>);
        #[mod_api::__subscriptions]
        impl Hooks for ModRegistrar {
            fn register_pipeline_modifiers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterPipelineModifiersEvent<'_>,
            ) {
                (self.0)(event);
            }
        }

        let collect = |mods: Vec<ModEntry>| -> Vec<PipelineModifier> {
            let mut staged = Vec::new();
            mod_platform::testing::with_mods(mods.into(), || {
                mod_api::dispatch_register_pipeline_modifiers(
                    &RegistrationContext::new(),
                    &mut staged,
                );
            });
            staged
        };

        let modifiers = collect(vec![
            ModEntry::new(
                "mod_a",
                Box::new(ModRegistrar(Box::new(|ev| {
                    ev.register(
                        PipelineModifier::new(id("mod_a:outer"))
                            .depth(WorldDepth::ThroughWalls)
                            .cull(true),
                    );
                }))),
            ),
            ModEntry::new(
                "mod_b",
                Box::new(ModRegistrar(Box::new(|ev| {
                    ev.register(
                        PipelineModifier::new(id("mod_b:inner"))
                            .depth(WorldDepth::Tested)
                            .depth_write(true),
                    );
                }))),
            ),
        ]);
        assert!(validate_modifiers(&modifiers).is_ok());

        let mut stacks = ModifierStacks::new();
        assert_eq!(stacks.intern(&[]), ModifierStacks::EMPTY);
        let s1 = stacks.intern(&[0, 1]);
        assert_eq!(stacks.intern(&[0, 1]), s1);
        let base = PipelineState::new(PipelineBlend::Translucent, WorldDepth::Tested);
        // Push order: [outer=0, inner=1]. Inner applies first (depth=Tested, depth_write=true),
        // then outer applies (depth=ThroughWalls, cull=true) — outermost wins on `depth`.
        let applied = stacks.apply(s1, base, &modifiers);
        assert_eq!(
            applied.depth_stencil.map(|ds| (ds.compare, ds.write)),
            Some((DepthCompare::Always, true))
        );
        assert!(applied.cull);
        assert_eq!(
            stacks.label(s1, "example:glow", &modifiers),
            "example:glow/transform/mod_b/inner/transform/mod_a/outer"
        );

        // Non-finite depth bias on a modifier fails validation.
        let bad_bias = collect(vec![ModEntry::new(
            "bad_mod",
            Box::new(ModRegistrar(Box::new(|ev| {
                ev.register(PipelineModifier::new(id("bad_mod:nan")).depth_bias(0, f32::NAN));
            }))),
        )]);
        assert!(matches!(
            validate_modifiers(&bad_bias),
            Err(ModifierRegistrationError::InvalidState { .. })
        ));

        // Duplicate modifier ID fails validation naming both mods.
        let dup = collect(vec![
            ModEntry::new(
                "first_mod",
                Box::new(ModRegistrar(Box::new(|ev| {
                    ev.register(PipelineModifier::new(id("shared:dup")));
                }))),
            ),
            ModEntry::new(
                "second_mod",
                Box::new(ModRegistrar(Box::new(|ev| {
                    ev.register(PipelineModifier::new(id("shared:dup")));
                }))),
            ),
        ]);
        let err = validate_modifiers(&dup).unwrap_err().to_string();
        assert!(
            err.contains("first_mod") && err.contains("second_mod"),
            "{err}"
        );
    }

    #[test]
    fn build_refuses_invalid_pipeline_state_and_exposes_registered_topology() {
        let read = |file: &str| match file {
            "assets/example/shaders/core/tri.vsh" => Ok(Some(VSH.to_owned())),
            "assets/example/shaders/core/tri.fsh" => Ok(Some(GLOW_FSH.to_owned())),
            other => Ok(stub_asset(other)),
        };

        let valid = [RenderPipeline::new(
            id("example:tri"),
            id("example:core/tri"),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .topology(PrimitiveTopology::TriangleFan)];
        let set = Pipelines::build(&valid, 1, read).unwrap();
        assert_eq!(set.topology(0), Some(PrimitiveTopology::TriangleFan));
        assert_eq!(set.topology(1), None);

        let bad_bias = [RenderPipeline::new(
            id("example:nan_bias"),
            id("example:core/tri"),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .depth_bias(0, f32::NAN)];
        assert!(Pipelines::build(&bad_bias, 2, read).is_err());

        let bad_target = [RenderPipeline::new(
            id("example:bad_target"),
            id("example:core/tri"),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .color_target(mod_api::MAX_COLOR_TARGETS, Some(ColorTargetState::DEFAULT))];
        assert!(Pipelines::build(&bad_target, 3, read).is_err());

        let mixed_mrt = [RenderPipeline::new(
            id("example:mixed_mrt"),
            id("example:core/tri"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
        .color_target(
            0,
            Some(ColorTargetState::blended(PipelineBlend::Translucent)),
        )
        .color_target(1, Some(ColorTargetState::blended(PipelineBlend::Additive)))];
        assert!(Pipelines::build(&mixed_mrt, 4, read).is_err());
    }

    #[test]
    fn custom_render_types_validate_resolve_and_reject_stale_handles() {
        let read = |file: &str| match file {
            "assets/example/shaders/core/tri.vsh" => Ok(Some(VSH.to_owned())),
            "assets/example/shaders/core/tri.fsh" => Ok(Some(GLOW_FSH.to_owned())),
            other => Ok(stub_asset(other)),
        };
        let mod_pipe = RenderPipeline::new(
            id("example:glow"),
            id("example:core/tri"),
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        );
        let vanilla = VanillaPipelines {
            required: HashSet::from(["minecraft:pipeline/solid".to_owned()]),
            optional: HashSet::new(),
        };

        let custom = vec![
            RenderSetup::new(id("example:glow_rt"), id("example:glow"))
                .use_lightmap(true)
                .sort_on_upload(true)
                .layering(LayeringTransform::ViewOffsetZ)
                .texture_transform(TextureTransform::Offset([0.25, 0.5])),
        ];
        assert!(validate_render_types(&custom, std::slice::from_ref(&mod_pipe), &vanilla).is_ok());

        // Builtin collision fails.
        let builtin_clash = vec![RenderSetup::new(
            id("minecraft:item_cutout"),
            id("example:glow"),
        )];
        assert!(matches!(
            validate_render_types(&builtin_clash, std::slice::from_ref(&mod_pipe), &vanilla),
            Err(RegistrationError::BuiltinRenderTypeCollision { .. })
        ));

        // Unknown pipeline fails.
        let unknown_pipe = vec![RenderSetup::new(
            id("example:orphan"),
            id("example:missing_pipeline"),
        )];
        assert!(matches!(
            validate_render_types(&unknown_pipe, std::slice::from_ref(&mod_pipe), &vanilla),
            Err(RegistrationError::UnknownRenderTypePipeline { .. })
        ));

        // Duplicate mod render type ID fails.
        let dup = vec![
            RenderSetup::new(id("example:dup"), id("example:glow")),
            RenderSetup::new(id("example:dup"), id("minecraft:pipeline/solid")),
        ];
        assert!(matches!(
            validate_render_types(&dup, std::slice::from_ref(&mod_pipe), &vanilla),
            Err(RegistrationError::DuplicateRenderTypeId { .. })
        ));

        let gen1 = Pipelines::build(std::slice::from_ref(&mod_pipe), 1, read).unwrap();
        let gen2 = Pipelines::build(&[mod_pipe], 2, read).unwrap();

        // Built-in resolves and validates.
        let item_cutout = gen1
            .resolve_render_type_in(&custom, &id("minecraft:item_cutout"))
            .unwrap();
        assert!(matches!(
            gen1.render_type_slot_in(&custom, item_cutout),
            Ok(ResolvedRenderType::Builtin(BuiltinRenderType::ItemCutout))
        ));

        // Custom resolves in gen1 and is stale in gen2.
        let glow_rt = gen1
            .resolve_render_type_in(&custom, &id("example:glow_rt"))
            .unwrap();
        assert!(matches!(
            gen1.render_type_slot_in(&custom, glow_rt),
            Ok(ResolvedRenderType::Custom {
                slot: 0,
                mod_pipeline_slot: Some(0),
                ..
            })
        ));
        let resolved = gen1.render_type_slot_in(&custom, glow_rt).unwrap();
        assert!(resolved.is_translucent());
        assert_eq!(resolved.layering(), LayeringTransform::ViewOffsetZ);
        assert_eq!(
            resolved.texture_transform(),
            TextureTransform::Offset([0.25, 0.5])
        );
        let cutout = gen1.render_type_slot_in(&custom, item_cutout).unwrap();
        assert!(!cutout.is_translucent());
        assert_eq!(cutout.layering(), LayeringTransform::None);
        assert_eq!(
            gen2.render_type_slot_in(&custom, glow_rt).unwrap_err(),
            RenderTypeError::Stale
        );
        assert_eq!(
            gen1.resolve_render_type_in(&custom, &id("example:nope")),
            Err(RenderTypeError::Unknown)
        );
    }

    #[test]
    fn layering_and_texture_transform_matrices_match_vanilla_formulas() {
        let id_mat = glam::Mat4::IDENTITY;
        assert_eq!(
            layering_transform_matrix(LayeringTransform::None, id_mat, false),
            id_mat
        );
        // 1 - 1/4096 and 1 + 1/4096 are exact in f32.
        let persp_z = layering_transform_matrix(LayeringTransform::ViewOffsetZ, id_mat, false);
        assert_eq!(persp_z.x_axis.x, 0.999_755_86);
        assert_eq!(persp_z.w_axis, glam::Vec4::new(0.0, 0.0, 0.0, 1.0));
        let persp_fwd =
            layering_transform_matrix(LayeringTransform::ViewOffsetZForward, id_mat, false);
        assert_eq!(persp_fwd.z_axis.z, 1.000_244_1);
        let ortho_z = layering_transform_matrix(LayeringTransform::ViewOffsetZ, id_mat, true);
        assert_eq!(ortho_z.w_axis.z, 0.001_953_125);
        let ortho_fwd =
            layering_transform_matrix(LayeringTransform::ViewOffsetZForward, id_mat, true);
        assert_eq!(ortho_fwd.w_axis.z, -0.001_953_125);

        assert_eq!(
            texture_transform_matrix(TextureTransform::Default, 1000, 0.5),
            id_mat
        );
        let offset = texture_transform_matrix(TextureTransform::Offset([0.25, -0.5]), 0, 0.5);
        assert_eq!(offset.w_axis.x, 0.25);
        assert_eq!(offset.w_axis.y, -0.5);
        // sin and cos of the f32 `PI / 18`, times each glint scale, rounded to f32.
        let glint = texture_transform_matrix(TextureTransform::Glint, 0, 0.5);
        assert_eq!(
            glint.x_axis,
            glam::Vec4::new(
                f32::from_bits(0x40fc1c5c),
                f32::from_bits(0x3fb1d0d4),
                0.0,
                0.0
            )
        );
        assert_eq!(
            glint.y_axis,
            glam::Vec4::new(
                -f32::from_bits(0x3fb1d0d4),
                f32::from_bits(0x40fc1c5c),
                0.0,
                0.0
            )
        );
        assert_eq!(glint.z_axis.z, 8.0);
        let entity = texture_transform_matrix(TextureTransform::EntityGlint, 0, 0.5);
        assert_eq!(entity.x_axis.x, f32::from_bits(0x3efc1c5c));
        assert_eq!(entity.x_axis.y, f32::from_bits(0x3db1d0d4));
        let armor = texture_transform_matrix(TextureTransform::ArmorEntityGlint, 0, 0.5);
        assert_eq!(armor.x_axis.x, f32::from_bits(0x3e2159d4));
        assert_eq!(armor.y_axis.x, -f32::from_bits(0x3ce39aa9));
        // `ticks = millis * speed * 8`: 55000 ms at 1.0 is 440000 ticks, which wraps to
        // 440000 % 110000 = 0 and 440000 % 30000 = 20000 → u = 0, v = 2/3.
        let scrolled = texture_transform_matrix(TextureTransform::Glint, 55_000, 1.0);
        assert_eq!(scrolled.w_axis.x, 0.0);
        assert_eq!(scrolled.w_axis.y, f32::from_bits(0x3f2aaaab));
    }
}
