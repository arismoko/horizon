//! `screenshot-sequence FRAMES STEP_MS PATH`: the running shader pack starts over, draws its
//! first FRAMES frames on a set clock, and the last is written as `screenshot PATH` writes it
//! (docs/agent-control.md, Repeatable screenshots). The reply comes once the PNG is written.

use super::Screen;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc::Sender};
use sunrise_client::gfx::{ScreenshotRequest, pack_uniforms::FrameTiming};
use sunrise_game_data::Result;

const MAX_FRAMES: u32 = 10_000;

/// The commands a running sequence lets through: they change nothing it draws.
const READ_ONLY: [&str; 5] = ["screenshot-sequence", "hello", "help", "status", "state"];

pub(in crate::game) struct Sequence {
    frames: u32,
    step_ms: f64,
    /// The frame drawn next, from 1.
    next: u32,
    path: PathBuf,
    reply: Sender<String>,
    /// What must stay as it was at the command start (`Game::sequence_scene`).
    scene: String,
    /// The camera comes from the first actual frame extraction, not a preview call.
    camera: Option<SequenceCamera>,
    pack: Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SequenceCamera {
    position: [u64; 3],
    rotation: [u32; 3],
    camera_type: sunrise_client::camera::CameraType,
}

impl Sequence {
    pub(in crate::game::client_side) fn timing(&self) -> FrameTiming {
        FrameTiming::nth(self.next, self.step_ms)
    }
}

/// `FRAMES STEP_MS PATH`; the path is the rest of the line and may hold spaces.
fn parse(args: &str) -> Result<(u32, f64, PathBuf)> {
    let usage = "screenshot-sequence FRAMES STEP_MS PATH, or screenshot-sequence cancel";
    let (frames, rest) = args.trim().split_once(char::is_whitespace).ok_or(usage)?;
    let (step_ms, path) = rest
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(usage)?;
    let frames: u32 = frames.parse().map_err(|_| usage)?;
    let step_ms: f64 = step_ms.parse().map_err(|_| usage)?;
    if !(1..=MAX_FRAMES).contains(&frames) {
        return Err(format!("FRAMES must be 1 to {MAX_FRAMES}").into());
    }
    if !step_ms.is_finite() || step_ms <= 0.0 {
        return Err("STEP_MS must be a number of milliseconds more than 0".into());
    }
    Ok((frames, step_ms, path.trim().into()))
}

impl crate::game::ClientGame<'_> {
    /// Why a sequence can't start (or go on) now, if it can't.
    fn sequence_refusal(&self) -> Option<&'static str> {
        let g = &self.client.settings.sunrise;
        if !self.common.clock.still() {
            Some("screenshot-sequence needs SUNRISE_STILL=1")
        } else if !self.common.clock.frozen()
            || self.common.clock.stepping()
            || self.common.clock.sprinting()
        {
            Some("screenshot-sequence needs the world frozen (/tick freeze), not stepping")
        } else if self.client.title.0.is_some()
            || self.client.screen != Screen::Playing
            || self.common.player.is_none()
            || self.client.integrated_starting.is_some()
            || self.client.pending_entry.0.is_some()
            || self.client.reload_overlay.is_some()
            || self.client.pending_save.0.is_some()
            || self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.level_load.is_active())
            || self.client.flight.0.is_some()
            || self.common.world.chunks.is_empty()
            || self.client.streaming.pending() != 0
            || !self.client.pending_uploads.0.is_empty()
            || self.remote_backlog() != 0
            || !self.common.world.dirty_light.is_empty()
            || !self.common.world.dirty_sections.is_empty()
        {
            Some("screenshot-sequence needs a loaded world in play, with nothing left to load")
        } else if self.client.lod.0.is_some() || self.far_terrain_choice().0 > 0 {
            // Far tiles load and fade in on the wall clock.
            Some("screenshot-sequence needs far terrain off (LOD Distance 0)")
        } else if self.client.settings.chunk_fade != 0.0 || g.shadow_cache {
            Some("screenshot-sequence needs chunk fade 0 and the shadow cache off")
        } else if g.taa || g.frame_generation != sunrise_client::graphics::FrameGeneration::Off {
            // Both build on earlier frames the pack's reset doesn't reach.
            Some("screenshot-sequence needs TAA and frame generation off")
        } else {
            None
        }
    }

    /// What a sequence's frames must share apart from the extracted camera.
    fn sequence_scene(&self, renderer: &sunrise_client::gfx::Renderer) -> String {
        format!(
            "{:?}",
            (
                (self.dim.id(), self.world.game_time, self.world.day_time),
                (self.world.revision, self.world.chunks.len()),
                self.world.chunks.membership_revision(),
                &self.client.settings,
                (
                    &self.client.native_shaders.picked,
                    renderer.shader_pack_kind()
                ),
                [renderer.config.width, renderer.config.height],
            )
        )
    }

    /// Check the camera that this frame will actually draw. On failure, render the same frame
    /// normally (the sequence's queued screenshot and clock are cleared), without firing hooks
    /// again to extract another camera.
    pub(in crate::game::client_side) fn check_screenshot_sequence_camera(
        &mut self,
        view: Option<(glam::DVec3, f32, f32, f32)>,
    ) {
        let Some(sequence) = &mut self.client.screenshot_sequence.0 else {
            return;
        };
        let Some((position, yaw, pitch, roll)) = view else {
            self.end_screenshot_sequence(Err("no camera".into()));
            return;
        };
        let camera = SequenceCamera {
            position: position.to_array().map(f64::to_bits),
            rotation: [yaw, pitch, roll].map(f32::to_bits),
            camera_type: self.client.camera_type,
        };
        if *sequence.camera.get_or_insert(camera) != camera {
            self.end_screenshot_sequence(Err("the camera changed while it ran".into()));
        }
    }

    /// `screenshot-sequence ...`; `None` while the reply waits for the PNG.
    pub(in crate::game::client_side) fn screenshot_sequence_command(
        &mut self,
        line: &str,
        reply: &Sender<String>,
    ) -> Result<Option<Value>> {
        let args = line
            .trim_start()
            .strip_prefix("screenshot-sequence")
            .unwrap_or("");
        if args.trim() == "cancel" {
            let running = self.client.screenshot_sequence.0.is_some();
            self.end_screenshot_sequence(Err("cancelled".into()));
            return Ok(Some(json!({ "ok": true, "cancelled": running })));
        }
        let (frames, step_ms, path) = parse(args)?;
        if self.client.screenshot_sequence.0.is_some() {
            return Err("a screenshot sequence is already running".into());
        }
        let renderer = self.client.renderer.0.as_ref().ok_or("no renderer yet")?;
        if self.client.pending_screenshot.0.is_some() || renderer.screenshot.is_some() {
            return Err("a screenshot is already pending".into());
        }
        if let Some(refusal) = self.sequence_refusal() {
            return Err(refusal.into());
        }
        let picked = self.client.native_shaders.picked.clone();
        let kind = renderer.shader_pack_kind();
        if !picked.is_empty() && kind.is_none() {
            return Err("the shader pack isn't running (still loading, or it failed)".into());
        }
        let scene = self.sequence_scene(renderer);
        let renderer = self.client.renderer.0.as_mut().ok_or("no renderer yet")?;
        renderer.reset_shader_pack_history();
        self.client.screenshot_sequence.0 = Some(Sequence {
            frames,
            step_ms,
            next: 1,
            path,
            reply: reply.clone(),
            scene,
            camera: None,
            pack: json!({
                "kind": kind.unwrap_or("none"),
                "name": (!picked.is_empty()).then_some(picked),
            }),
        });
        Ok(None)
    }

    /// While a sequence runs, the control commands that would change what it draws are
    /// refused; `quit` ends it first.
    pub(in crate::game::client_side) fn screenshot_sequence_gate(
        &mut self,
        command: Option<&str>,
    ) -> Result<()> {
        if self.client.screenshot_sequence.0.is_none()
            || command.is_none_or(|c| READ_ONLY.contains(&c))
        {
            return Ok(());
        }
        if command == Some("quit") {
            self.end_screenshot_sequence(Err("cancelled: the game is quitting".into()));
            return Ok(());
        }
        Err(
            "a screenshot sequence is running: wait for its reply, or screenshot-sequence cancel"
                .into(),
        )
    }

    /// Ends the running sequence, if any, and sends its reply.
    pub(in crate::game::client_side) fn end_screenshot_sequence(&mut self, outcome: Result<Value>) {
        let Some(sequence) = self.client.screenshot_sequence.0.take() else {
            return;
        };
        if let Some(renderer) = &mut self.client.renderer.0
            && renderer
                .screenshot
                .as_ref()
                .is_some_and(|r| r.path == sequence.path)
        {
            renderer.screenshot = None;
        }
        let value = outcome.unwrap_or_else(
            |e| json!({ "error": format!("screenshot-sequence: {e}"), "frame": sequence.next }),
        );
        let _ = sequence.reply.send(value.to_string());
    }

    /// One frame of the running sequence: the next one, on its clock. A frame the surface
    /// refused is drawn again; the last is saved, and the reply sent.
    pub(in crate::game::client_side) fn render_sequence_frame(&mut self) -> Result<bool> {
        let sequence = self
            .client
            .screenshot_sequence
            .0
            .as_ref()
            .expect("a running sequence");
        let refusal = self.sequence_refusal().or_else(|| {
            let Some(renderer) = &self.client.renderer.0 else {
                return Some("no renderer");
            };
            (self.sequence_scene(renderer) != sequence.scene)
                .then_some("the world, settings, shader pack or window size changed while it ran")
        });
        if let Some(reason) = refusal {
            self.end_screenshot_sequence(Err(reason.into()));
            return self.render_frame();
        }
        let last = sequence.next == sequence.frames;
        if last && let Some(renderer) = &mut self.client.renderer.0 {
            renderer.screenshot = Some(ScreenshotRequest {
                path: sequence.path.clone(),
                size: None,
                scaled: false,
                background: false,
            });
            renderer.screenshot_result = None;
        }
        let drawn = self.render_frame();
        if self.client.screenshot_sequence.0.is_none() {
            // The extracted camera changed; its frame was drawn outside the sequence.
            return drawn;
        }
        let renderer = self.client.renderer.0.as_mut().ok_or("no renderer")?;
        // Still queued when the frame wasn't drawn, or when the PNG is to be taken
        // again smaller (out of memory): not this sequence's frame.
        let queued = last && renderer.screenshot.take().is_some();
        let failed = drawn.is_err();
        let done = match drawn {
            Ok(false) => {
                let sequence = self
                    .client
                    .screenshot_sequence
                    .0
                    .as_mut()
                    .expect("a running sequence");
                if sequence.next == 1 {
                    // The surface did not draw this attempt. The first drawn frame sets the baseline.
                    sequence.camera = None;
                }
                return Ok(false);
            }
            Err(e) => Err(e.to_string()),
            Ok(true) => renderer.finish_sequence_frame().map_err(|e| e.to_string()),
        };
        if let Err(e) = done {
            self.end_screenshot_sequence(Err(e.into()));
            return Ok(!failed);
        }
        let sequence = self
            .client
            .screenshot_sequence
            .0
            .as_mut()
            .expect("a running sequence");
        if !last {
            sequence.next += 1;
            return Ok(true);
        }
        let outcome = match (queued, renderer.screenshot_result.take()) {
            (false, Some(Ok(pending))) => {
                let saved = pending.save_with_details().map_err(|e| e.to_string())?;
                let [w, h] = saved.size;
                let timing = sequence.timing();
                let mut value = json!({
                    "ok": true,
                    "path": sequence.path,
                    "frames": sequence.frames,
                    "step_ms": sequence.step_ms,
                    "frame": {
                        "counter": timing.counter,
                        "elapsed": timing.elapsed,
                        "delta": timing.delta,
                    },
                    "size": [w, h],
                    "pack": sequence.pack.clone(),
                });

                if let Some(hdr) = saved.hdr_path {
                    value["hdr_path"] = json!(hdr);
                }
                Ok(value)
            }
            (_, Some(Err(e))) => Err(e.into()),
            _ => Err("the last frame's PNG wasn't written".into()),
        };
        self.end_screenshot_sequence(outcome);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::{Screen, parse};
    use crate::game::{
        native_entity_refs::tests::with_client_game,
        native_hooks::particle_count_tests::frame_scene,
    };
    use mod_api::{
        CameraPositionEvent, ClientHost, ComputeCameraAnglesEvent, Hooks, ModEntry,
        RenderViewTransformEvent,
    };
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
    };
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases_with_env;

    struct FrameEdits(Arc<AtomicUsize>, Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for FrameEdits {
        fn compute_camera_angles(&self, _: &dyn ClientHost, event: &mut ComputeCameraAnglesEvent) {
            event.yaw = if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                45.0
            } else {
                90.0
            };
        }
        fn camera_position(&self, _: &dyn ClientHost, event: &mut CameraPositionEvent<'_>) {
            self.1.fetch_add(1, Ordering::Relaxed);
            assert!(event.position.set([Some(0.25), None, None]));
        }
        fn render_view_transform(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderViewTransformEvent<'_, '_>,
        ) {
            assert!(event.transform.translate(2.0, 0.0, 0.0));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn sequence_checks_the_extracted_camera_and_paused_worlds_still_apply_view_transforms() {
        if run_fresh_cases_with_env(
            &format!(
                "{}::sequence_checks_the_extracted_camera_and_paused_worlds_still_apply_view_transforms",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_SEQUENCE_CAMERA_CHILD",
            &["1"],
            1,
            &[("SUNRISE_STILL", "1")],
        ) {
            return;
        }
        let angles = Arc::new(AtomicUsize::new(0));
        let positions = Arc::new(AtomicUsize::new(0));
        let mods = vec![ModEntry::new(
            "frame-edits",
            Box::new(FrameEdits(angles.clone(), positions.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.client.screen = Screen::Playing;
                game.client.settings.view_bobbing = false;
                // This scene freezes before the camera tick that initializes the FOV scale.
                game.client.view.fov_modifier = 1.0;
                game.client.view.old_fov_modifier = 1.0;
                game.client.settings.chunk_fade = 0.0;
                game.client.settings.sunrise.lod_distance = 0;
                game.client.settings.sunrise.taa = false;
                game.client.settings.sunrise.shadow_cache = false;
                game.client.settings.sunrise.frame_generation =
                    sunrise_client::graphics::FrameGeneration::Off;
                game.common.clock.set_frozen(true);
                let (reply, result) = mpsc::channel();
                let scratch = tempfile::tempdir().unwrap();
                let target = scratch.path().join("final.png");
                assert!(
                    game.screenshot_sequence_command(
                        &format!("screenshot-sequence 2 16 {}", target.display()),
                        &reply,
                    )
                    .unwrap()
                    .is_none()
                );
                assert_eq!(angles.load(Ordering::Relaxed), 0);
                assert_eq!(positions.load(Ordering::Relaxed), 0);

                assert!(game.render_sequence_frame().unwrap());
                assert_eq!(angles.load(Ordering::Relaxed), 1);
                assert_eq!(positions.load(Ordering::Relaxed), 1);
                assert_eq!(game.client.screenshot_sequence.0.as_ref().unwrap().next, 2);
                assert!(result.try_recv().is_err());

                assert!(game.render_sequence_frame().unwrap());
                assert_eq!(angles.load(Ordering::Relaxed), 2);
                assert_eq!(positions.load(Ordering::Relaxed), 2);
                assert!(game.client.screenshot_sequence.0.is_none());
                assert!(
                    game.client
                        .renderer
                        .0
                        .as_ref()
                        .unwrap()
                        .screenshot
                        .is_none()
                );
                assert!(
                    game.client
                        .renderer
                        .0
                        .as_ref()
                        .unwrap()
                        .screenshot_result
                        .is_none()
                );
                assert!(!target.exists());
                let response: serde_json::Value =
                    serde_json::from_str(&result.try_recv().unwrap()).unwrap();
                assert_eq!(response["frame"].as_u64(), Some(2));
                assert!(
                    response["error"]
                        .as_str()
                        .unwrap()
                        .contains("camera changed"),
                    "{response}"
                );

                game.client.screen = Screen::Paused;
                game.render_with(|_, frame| {
                    assert_eq!(frame.bob, glam::Mat4::IDENTITY);
                    assert_eq!(
                        frame.view_transform,
                        glam::Mat4::from_translation(glam::Vec3::new(2.0, 0.0, 0.0))
                    );
                    Ok(true)
                })
                .unwrap();
            });
        });
    }

    #[test]
    fn the_path_is_the_rest_of_the_line_and_the_counts_are_bounded() {
        assert_eq!(
            parse(" 64  50 /tmp/a b.png ").unwrap(),
            (64, 50.0, PathBuf::from("/tmp/a b.png"))
        );
        assert_eq!(parse("10000 1 x.png").unwrap().0, 10_000);
        assert_eq!(parse("64 16.666667 x.png").unwrap().1, 16.666667);
        for bad in [
            "0 50 x.png",
            "10001 50 x.png",
            "4 0 x.png",
            "4 -1 x.png",
            "4 0.0 x.png",
            "4 NaN x.png",
            "4 inf x.png",
            "4 50",
            "x 50 a",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
