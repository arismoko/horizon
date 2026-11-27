//! Client render pipeline registration.

pub trait Hooks {
    /// Register render pipelines and custom render types once at client startup (NeoForge's
    /// `RegisterRenderPipelinesEvent`; 1.21.1 mods used `RegisterShadersEvent`): call
    /// `event.register(pipeline)` with each [`RenderPipeline`] and
    /// `event.register_render_type(setup)` with each [`RenderSetup`].
    /// `event.compute_pipeline(pipeline)` with each [`ComputePipeline`] (a GLSL compute shader
    /// `assets/<ns>/shaders/compute/<path>.csh`, its [`BindGroupLayout`] and defines). Each pipeline carries a
    /// [`PipelineState`] (blend including [`PipelineBlend::Opaque`] and
    /// [`PipelineBlend::Custom`], optional [`DepthState`] with [`DepthCompare`] and depth bias,
    /// culling, colour write mask, polygon mode, and [`PrimitiveTopology`]), optional multi-target
    /// [`ColorTargetState`] slots up to [`MAX_COLOR_TARGETS`] (8), optional `.order_independent()`
    /// opt-in for [`RenderLevelStage::OitTranslucent`], optional separate vertex shader and
    /// defines. Each [`RenderSetup`] wraps a registered mod or vanilla pipeline ID with texture
    /// bindings, lightmap/overlay toggles, crumbling/sorting/solid-phase flags,
    /// [`LayeringTransform`], [`TextureTransform`], and [`OutlineProperty`]. The game builds them
    /// with its own at startup and again on every resource reload.
    /// Any valid [`Identifier`] is accepted, and duplicate mod IDs fail startup, regardless of
    /// required or optional policy. Collisions with vanilla are checked in the corresponding
    /// required or optional table (and custom render types are checked against
    /// [`BUILTIN_RENDER_TYPE_IDS`]). An unavailable vanilla pipeline table fails startup
    /// registration. Non-finite depth bias slope scales, colour target indices `>= 8`, and
    /// differing blend functions across active colour targets fail startup registration. A
    /// required pipeline whose shader (or OIT variant when `.order_independent()`) is missing or
    /// doesn't build fails the load: the game does not start, and a reload fails and keeps the
    /// previous resources. An [optional](RenderPipeline::optional()) one is logged and resolves
    /// to [`RenderPipelineError::Invalid`].
    #[notes = r#"## Render pipelines

`Hooks::register_render_pipelines` is the client registration-phase counterpart of NeoForge's
`RegisterRenderPipelinesEvent` (and of 1.21.1's `RegisterShadersEvent`, which it replaced). A
pipeline is plain data: its id, a GLSL core shader program (`.vsh` + `.fsh`) in the mod's assets
or another namespace, optional preprocessor defines, optional sprite texture, blend mode and depth
mode. Pass it to `event.register(pipeline)`; registration takes no GPU object or shader handle.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.register(
        RenderPipeline::new(
            Identifier::parse("example:glow").unwrap(),
            Identifier::parse("example:core/glow").unwrap(),
            PipelineBlend::Additive,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        .define("PULSE"),
    );
}
```

The shader id `example:core/glow` names `assets/example/shaders/core/glow.vsh` and
`assets/example/shaders/core/glow.fsh` (matching vanilla's `withVertexShader`/`withFragmentShader`).
Call `.vertex_shader(id)` to reuse another program's `.vsh`, such as
`minecraft:core/position_tex_color` or a library mod's vertex stage. Both stages may use
`#include <ns:path>`, `#include "relative"`, and `#moj_import` resolved through the active
resource stack (vanilla assets, loaded mods, and resource packs), and both stages receive
`RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` followed by any `.define(...)` / `.define_value(...)` entries
declared on the pipeline.

```glsl
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;

void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef PULSE
    color.rgb *= 1.0 + sin(GameTime * 24000.0 * 0.2);
#endif
    fragColor = vec4(color.rgb, color.a * (1.0 - texCoord0.y));
}
```

The engine binds vanilla's four uniform blocks (`Globals`, `Projection`, `DynamicTransforms` with
camera-relative `ModelViewMat`, `ColorModulator = 1`, `ModelOffset = 0`, `TextureMat = identity`,
and `Fog`), vertex attributes `Position`, `UV0` and `Color` by name, and — when `.textured(...)`
is set — `Sampler0` plus `<sunrise:pipeline_sprite.glsl>` providing `vec4 sampleSprite(vec2 uv)`.
Mod pipelines work in vanilla's gamma space, matching vanilla core shaders: write what vanilla
writes (`texture * vertexColor * ColorModulator`). On frames holding linear light, the engine
decodes the fragment's location-0 output after `main()` (sanitizing negative/NaN RGB to 0 and
clamping alpha coverage to `[0, 1]`). Expanded GLSL stages are guarded for crash/memory
containment (1 MiB expanded, 200 chained operators per expression, 200 bracket/unary/brace-less
nesting levels, include depth 16 / 1,024 pastes / 8 MiB scanned).

Resolve the pipeline with `host.render_assets().render_pipeline(&id)` in `client_setup`
and again in `client_resources_reloaded`, then draw with `WorldDrawContext::pipeline_quads` from
`submit_custom_geometry` (or a debug renderer). A resource reload builds every registered pipeline
again, so a resource pack can replace a shader or an include; a handle from before the reload is
`Stale`.

A pipeline is required unless it is registered with `.optional()` (NeoForge's
`registerPipeline` and `registerOptionalPipeline`). Any valid `Identifier` is accepted, and
a mod pipeline ID must be unique across both policies, because mod lookups are by ID. A
collision with vanilla is checked in the corresponding required or optional table. Invalid GLSL
define names or values fail registration at startup. When a load cannot build a required pipeline
because its `.vsh`, `.fsh`, or an include is missing, refused or turned down by the GPU, the whole
load fails and names it: the game does not start, and a reload keeps the previous resources and
handles, then drops the selected resource packs and tries again, as vanilla does with any failed
reload. An optional pipeline that cannot be built is logged once with its id and resolves to
`Invalid`, and the other pipelines draw. Pipeline quads grow dynamically into the GPU buffers
with no fixed per-view cap (skipping any quad with non-finite coordinates or UVs individually);
in the deferred world pass, pipeline quads draw after the plain quads and lines, in submission
order.

## Compute pipelines

`event.compute_pipeline(ComputePipeline::builder(id, shader_id).bind_group_layout(layout)
.define("NAME").build())` registers a GLSL 450 compute shader on the same registrar. Vanilla's
`renderpearl` API has no compute pass, so a compute pipeline follows what it has: the shader is
`assets/<ns>/shaders/compute/<path>.csh`, its
bindings are a [`BindGroupLayout`] of uniform blocks, samplers, texel buffers, std430 storage
buffers and `image2D` storage images in one bind group, and the dispatch binds them with the same
[`DrawBindings`] a draw takes. Resolve the handle with `host.render_assets().compute_pipeline(&id)`
in `client_setup` and again in `client_resources_reloaded`.

Dispatch from a frame-graph compute node (`event.graph.add_compute_pass(ComputePassDescriptor::new(..)
.reads(..).writes(..), |ctx| ctx.dispatch(..))` in `frame_graph_setup`), ordered by the targets it
reads and writes and never culled; the node runs outside any render pass. `dispatch` takes workgroup
counts (each at most the device's `max_compute_workgroups_per_dimension`; a zero is a no-op) and
`dispatch_indirect` reads three `u32` counts from a storage buffer created with `indirect: true`.
The workgroup size (`max_compute_workgroup_size_*`, `max_compute_invocations_per_workgroup`), the
workgroup storage (`max_compute_workgroup_storage_size`) and the binding counts are checked
against the device's limits when the pipeline builds; a shader needing subgroup operations is
refused with [`ComputePipelineError::UnsupportedFeature`] on an adapter without them
([`RenderCapabilities::subgroup`]). A required-limit violation or a bad binding is an `Err`, never
a panic or a lost device. A compute pipeline whose shader is missing or does not build is logged once
and resolves to `Invalid`; it never fails the load. Compute pipelines and the GPU objects they
bind outlive a resource reload and a world close, but a [`ComputePipelineRef`] goes stale on a
reload like every pipeline handle: resolve it again in `client_resources_reloaded`."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_render_pipelines::native_register_render_pipelines",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "pipelines",
        registration_append = "attributed",
        context = "registration",
        neoforge = "RegisterRenderPipelinesEvent",
        version = 6
    )]
    fn register_render_pipelines(
        pipelines: &mut Vec<mod_api::RenderPipeline>,
        render_types: &mut Vec<mod_api::RenderSetup>,
        compute_pipelines: &mut Vec<mod_api::ComputePipeline>,
    );
}
