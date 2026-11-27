use super::loading_frame;
use std::time::{Duration, Instant};
use sunrise_client::hud::HudBatch;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::saving::ClientQuit;
use sunrise_gameplay_common::game::saving::ClosingSave;
use sunrise_gameplay_common::game::saving::Stage;

impl crate::game::ClientGame<'_> {
    pub fn begin_closing_save(&mut self, quit: bool) -> Result<()> {
        if quit {
            self.client.client_quit.set(ClientQuit::Stopping);
            self.close_client_tasks();
        }
        if let Some(save) = &mut self.client.pending_save.0 {
            save.quit |= quit;
            return Ok(());
        }
        // A world still loading has no client in it yet: its server stops and is joined
        // through the same fallible path as a running one, before the loop may end.
        if quit && let Some(starting) = self.client.integrated_starting.take() {
            starting.handle.request_stop();
            self.client.integrated = Some(starting.handle);
            self.client.pending_save.0 = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Server,
            });
            return Ok(());
        }
        if self.client.saved_for_exit {
            self.client.pending_save.0 = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Quit,
            });
            return Ok(());
        }
        if self.client.title.0.is_some() {
            if quit {
                self.client_logging_out();
                self.client.saved_for_exit = true;
            }
            self.client.pending_save.0 = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Quit,
            });
            return Ok(());
        }
        if self.client.integrated.is_some() {
            // Minecraft.disconnect: the client logs out before its server halts.
            self.client_logging_out();
        }
        // Stop client-side terrain work before the integrated server is shut down. The
        // server's own stop path aborts its workers, but the window's streaming registration
        // is dropped only after the event loop exits and would otherwise wait for queued work.
        self.client.streaming.abort_generation();
        if let Some(integrated) = &self.client.integrated {
            integrated.request_stop();
            self.client.pending_save.0 = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Server,
            });
        } else if self.client.remote.is_some() {
            if !quit {
                return self.leave_server(None);
            }
            self.connection_left(sunrise_hooks::connection::Left::Quit);
            self.client_logging_out();
            if let Some(mut remote) = self.client.remote.take() {
                remote.session.disconnect();
            }
            self.client.saved_for_exit = true;
            self.client.pending_save.0 = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Quit,
            });
        } else {
            self.client.pending_save.0 = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Quit,
            });
        }
        self.capture(false);
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_seed_pending_server_save(&mut self, reply: std::sync::mpsc::Sender<String>) {
        self.client.pending_save.0 = Some(ClosingSave {
            quit: false,
            close: false,
            reply: Some(reply),
            started: Instant::now(),
            stage: Stage::Server,
        });
    }
    /// `true` means the window can exit. The session lock stays in Game until
    /// every snapshot has been queued and every writer has synced its files.
    pub fn poll_closing_save(&mut self) -> Result<bool> {
        let Some(mut save) = self.client.pending_save.0.take() else {
            return Ok(false);
        };
        let result = (|| -> Result<bool> {
            match &mut save.stage {
                Stage::Quit => {
                    if save.quit && !self.client.saved_for_exit {
                        self.client_logging_out();
                    }
                    Ok(true)
                }
                Stage::Server => {
                    let server = self
                        .client
                        .integrated
                        .as_ref()
                        .ok_or("the integrated server stopped with an error")?;
                    if !server.stopped() {
                        return Ok(false);
                    }
                    // Logout already ran before the integrated server was asked to stop.
                    if save.quit {
                        self.client.integrated.take().unwrap().stop()?;
                    } else {
                        self.finish_leave_server(None)?;
                    }
                    Ok(true)
                }
                Stage::Generation | Stage::Chunks(_) | Stage::Entities(_) | Stage::Writes(_) => {
                    Err("local disk save stages only run on the server".into())
                }
            }
        })();
        match result {
            Ok(true) => {
                if let Some(reply) = save.reply.take() {
                    let _ = reply.send(serde_json::json!({ "ok": true }).to_string());
                }
                if save.quit && !save.close {
                    // A window close during a control save still has to close
                    // containers and save the resulting inventory changes.
                    self.begin_closing_save(true)?;
                    return Ok(false);
                }
                if save.quit {
                    self.client.saved_for_exit = true;
                    Ok(true)
                } else {
                    if save.close && self.client.title.0.is_none() {
                        self.finish_perf();
                        self.finish_close_world()?;
                    }
                    self.common.clock.reset();
                    self.common.previous = Instant::now();
                    Ok(false)
                }
            }
            other => {
                if let Err(error) = &other
                    && let Some(reply) = save.reply.take()
                {
                    let _ =
                        reply.send(serde_json::json!({ "error": error.to_string() }).to_string());
                }
                // Keep ownership (and the lock) on errors too. Native shutdown must
                // finish a durable barrier before it can declare the save completed.
                self.client.pending_save.0 = Some(save);
                other
            }
        }
    }
    pub(in crate::game::client_side) fn render_closing_save(&mut self) -> Result<()> {
        if matches!(
            self.client.pending_save.0.as_ref().map(|save| &save.stage),
            Some(Stage::Quit)
        ) {
            return Ok(());
        }
        let waiting = matches!(
            self.client.pending_save.0.as_ref().map(|save| &save.stage),
            Some(Stage::Server | Stage::Writes(_))
        );
        if waiting {
            thread_local! {
                static LAST_FRAME: std::cell::Cell<Option<Instant>> =
                    const { std::cell::Cell::new(None) };
            }
            const FRAME: Duration = Duration::from_millis(33);
            if let Some(last) = LAST_FRAME.with(|c| c.get()) {
                let since = last.elapsed();
                if since < FRAME {
                    std::thread::sleep(FRAME - since);
                }
            }
            LAST_FRAME.with(|c| c.set(Some(Instant::now())));
        }
        // disconnectWithSavingScreen: a GenericMessageScreen, its title and
        // no navigation.
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
                "saving",
                super::screen_info::tr("menu.savingLevel", &[]),
            )
            .without_navigation()
        });
        let Some(renderer) = &mut self.client.renderer.0 else {
            return Ok(());
        };
        let mut hud = HudBatch::new(
            [renderer.config.width, renderer.config.height],
            self.client.settings.gui_scale_inputs(),
        );
        let progress = match self.client.pending_save.0.as_ref().map(|save| &save.stage) {
            Some(Stage::Chunks(_)) => 0.25,
            Some(Stage::Entities(_)) => 0.5,
            Some(Stage::Writes(_)) => 0.9,
            _ => 0.0,
        };
        sunrise_client::level_loading::draw_progress(
            &mut hud,
            &self.client.gui.atlas,
            "Saving world...",
            progress,
        );
        renderer.render(&loading_frame(&hud), None)?;
        Ok(())
    }
    /// `Minecraft.stop`: record the request without reentering the client from a hook.
    pub fn request_client_quit(&self) -> bool {
        if !self.common.native_window()
            || self.client.client_quit.get() != ClientQuit::Running
            || sunrise_threads::stop_requests() > 0
        {
            return false;
        }
        self.client.client_quit.set(ClientQuit::Requested);
        true
    }
    /// `GameRenderer.tryTakeScreenshotIfNeeded`: while a local world without a
    /// saved icon is on the screen, grab the rendered frame once a second until
    /// the save has one. The Select World list shows it as the world's preview.
    pub(in crate::game::client_side) fn tick_world_icon(&mut self) {
        // Its grab would take a sequence's frame.
        if self.client.screenshot_sequence.0.is_some() {
            return;
        }
        let Some(icon) = self.client.world_icon.as_ref() else {
            return;
        };
        if icon
            .last_attempt
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        if icon.path.is_file() {
            // tryTakeScreenshotIfNeeded's hasWorldScreenshot: it has one, stop asking.
            self.client.world_icon = None;
            return;
        }
        // takeAutoScreenshot only saves with every loaded section rendered, or
        // the icon would picture the holes the streamer has not filled yet.
        if self.client.streaming.pending() != 0
            || !self.client.pending_uploads.0.is_empty()
            || self.remote_backlog() != 0
        {
            return;
        }
        let Some(renderer) = self.client.renderer.0.as_mut() else {
            return;
        };
        // countRenderedSections() > 10: a client that joined its server
        // thread has nothing pending before the first chunks arrive, and the
        // icon would be only sky.
        if renderer.drawn_sections <= 10 {
            return;
        }
        if renderer.screenshot.is_some() {
            // F2's or an agent's screenshot owns the frame this time; ours
            // comes around again with the next attempt.
            return;
        }
        let path = icon.path.clone();
        if let Some(icon) = self.client.world_icon.as_mut() {
            icon.last_attempt = Some(Instant::now());
        }
        renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
            path,
            size: Some([64, 64]),
            scaled: false,
            background: true,
        });
    }
}
