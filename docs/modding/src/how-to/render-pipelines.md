# Drawing with your own render pipeline

A client mod can draw world quads with its own fragment shader. The mod describes the pipeline as
data, the game builds it, and the mod draws by handle. This is the counterpart of NeoForge 26.3's
`RegisterRenderPipelinesEvent`, in a restricted form: the mod supplies the fragment stage and
 the host fixes the rest. Vanilla requires a vertex shader and a primitive type; this API fixes
 both rather than exposing the full pipeline description.

## 1. Write the shader

Put a WGSL file at `assets/<namespace>/shaders/pipeline/<path>.wgsl`. It defines one function:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let pulse = 1.0 + sin(frame.time * 4.0);
    return vec4(input.color.rgb * pulse, input.color.a * (1.0 - input.uv.y));
}
```

The game puts this before your file:

| Name | Type | Meaning |
| --- | --- | --- |
| `input.uv` | `vec2<f32>` | The quad's texture coordinate at this fragment, as its corners gave it. |
| `input.color` | `vec4<f32>` | The quad's colour in linear light; alpha is coverage. |
| `input.position` | `vec3<f32>` | This point in blocks, relative to the camera. |
| `frame.time` | `f32` | Seconds of game time, smooth between ticks, wrapping to 0 every hour. |
| `frame.viewport` | `vec2<f32>` | The world picture's size in pixels. |

Colours follow the [mod render colour rule](../concepts/mod-render-colour.md): the function reads
and returns linear light, 1.0 is paper white, and more is brighter where the frame can hold it. The
alpha you return is coverage. The game supplies the vertex stage, the bindings and the conversion
to the frame, so a file that declares a binding or an entry point is refused. Names that start with
`horizon_` belong to the game.

## 2. Register the pipeline

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.pipelines.push(RenderPipeline::new(
        Identifier::parse("example:glow").unwrap(), // the pipeline's id
        Identifier::parse("example:glow").unwrap(), // assets/example/shaders/pipeline/glow.wgsl
        PipelineBlend::Additive,
        WorldDepth::Tested,
    ));
}
```

`PipelineBlend::Translucent` lays the result over the frame by its alpha, and `Additive` adds it,
scaled by its alpha. `WorldDepth::Tested` hides the quads behind nearer world surfaces, and
`ThroughWalls` draws them over everything. No pipeline writes depth.

The id must be in a namespace your mod owns. There is no per-mod pipeline count cap. A registration
that breaks either rule, or reuses an id, is logged with the mod's id and dropped.

### Required or optional

A pipeline is required unless you register it with `.optional()`, as NeoForge's `registerPipeline`
and `registerOptionalPipeline` are:

```rust,ignore
event.pipelines.push(
    RenderPipeline::new(id, fragment, PipelineBlend::Translucent, WorldDepth::Tested).optional(),
);
```

| Policy | When a resource load cannot build it |
| --- | --- |
| Required (`RenderPipeline::new`) | The load fails with a message naming each required pipeline and why. At startup the game does not start. A resource reload fails and keeps the previous resources, so handles from before it still draw; then, as vanilla does with any failed reload, the selected resource packs are dropped and the reload runs again. |
| Optional (`.optional()`) | The log has one line naming the pipeline and why. It resolves to `RenderPipelineError::Invalid`, and every other pipeline builds and draws. |

A load cannot build a pipeline when its shader file is missing, the file is refused, or the GPU
turns the built shader down. The game never panics over a pipeline's shader.

## 3. Resolve and draw

```rust,ignore
fn client_setup(&self, host: &dyn ClientHost, _event: &mut ClientSetupEvent<'_>) {
    let id = Identifier::parse("example:glow").unwrap();
    *self.glow.lock().unwrap() = host.render_assets().render_pipeline(&id).ok();
}

fn submit_custom_geometry(&self, _host: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
    let Some(glow) = *self.glow.lock().unwrap() else { return };
    let quad = PipelineQuad::spanning(corners, 0xFFFF_C040);
    if let Err(error) = event.draw.pipeline_quads(glow, &[quad]) {
        log::warn!("glow not drawn: {error:?}");
    }
}
```

Resolve again in `client_resources_reloaded`: every resource reload builds the pipelines again
from the new resources, so a resource pack can replace your shader, and a handle from before the
reload answers `Stale`.

| Error | When |
| --- | --- |
| `RenderPipelineError::Unknown` | No mod registered that id. |
| `RenderPipelineError::Invalid` | The pipeline is optional and its shader file is missing, was refused or was turned down by the GPU. The log has a line `render pipeline <id> from mod <owner> refused: <file>: <reason>`, with the file's line number for a syntax or type error, or `render pipeline <id> from mod <owner> refused by the GPU: <reason>`. |
| `RenderPipelineError::Stale` | The handle is from before a resource reload. |
| `PipelineDrawError::NonFinite` | A corner or texture coordinate is not finite. |
| `PipelineDrawError::CapacityExceeded` | The view's 4,096 quads, shared with `WorldDrawContext::quads`, are used up. |

A refused batch draws nothing. An `Invalid` pipeline never stops the game from starting; a required
pipeline that cannot be built does.

## What a pipeline draws, and when

`pipeline_quads` works wherever a `WorldDrawContext` is given: `submit_custom_geometry` and each
`render_level_stage`. Within a stage, pipeline quads draw after that stage's plain quads and
lines, in the order submitted. Under a shader pack they follow the plain quads: a stage the pack
draws itself is drawn with the next stage the game reaches.

A pipeline cannot sample textures, take values of its own from the mod, or replace the vertex
stage yet.
