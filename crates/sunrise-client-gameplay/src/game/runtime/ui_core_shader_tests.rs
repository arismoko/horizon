use sunrise_assets::resource_source::{Resource, ResourceSource};

const SCREENQUAD: &str = "assets/minecraft/shaders/core/screenquad.vsh";
const LIGHTMAP: &str = "assets/minecraft/shaders/core/lightmap.fsh";

/// A lightmap fragment stage writing `SkyFactor` in `channel`.
fn lightmap_fsh(channel: &str) -> String {
    format!(
        "#version 330\nlayout(std140) uniform LightmapInfo {{\n    float SkyFactor;\n}} info;\n\
         layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
         void main() {{\n    fragColor = vec4(0.0);\n    fragColor.{channel} = info.SkyFactor;\n}}\n"
    )
}

#[test]
fn a_packs_lightmap_is_told_from_the_games_own_at_the_vanilla_folder() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "sunrise-ui-core-shaders-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    let core = scratch.0.join("assets/minecraft/shaders/core");
    std::fs::create_dir_all(&core).unwrap();
    std::fs::write(
        scratch.0.join(SCREENQUAD),
        "#version 330\nlayout(location = 0) out vec2 texCoord;\nvoid main() {\n    \
         vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n    \
         gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n    texCoord = uv;\n}\n",
    )
    .unwrap();
    std::fs::write(scratch.0.join(LIGHTMAP), lightmap_fsh("r")).unwrap();
    let vanilla = ResourceSource::directory(scratch.0.clone());
    // The launch's stack: the game's screen quad, and the lightmap from `lightmap`.
    let stack = |lightmap: Resource| {
        ResourceSource::indexed(
            [
                (
                    SCREENQUAD.to_owned(),
                    Resource::file(vanilla.clone(), SCREENQUAD.to_owned()),
                ),
                (LIGHTMAP.to_owned(), lightmap),
            ]
            .into_iter()
            .collect(),
        )
    };
    let pack = stack(Resource::bytes(lightmap_fsh("g").into_bytes()));
    let gui = crate::game::UiResources::load(&pack, &scratch.0).unwrap();
    let program = gui.atlas.lightmap_shader.expect("the pack's lightmap");
    assert_eq!(program.other, ["LightmapInfo"]);
    // Without a pack, the game's own lightmap is no pack's: the native texture stays.
    let plain = stack(Resource::file(vanilla.clone(), LIGHTMAP.to_owned()));
    let gui = crate::game::UiResources::load(&plain, &scratch.0).unwrap();
    assert!(gui.atlas.lightmap_shader.is_none());
}
