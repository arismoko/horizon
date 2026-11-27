//! The server's run/stop boundary, shared by integrated and dedicated servers.

use crate::game::ServerGame;
use std::panic::{AssertUnwindSafe, catch_unwind};
use sunrise_game_data::Result;

#[cfg(test)]
mod interrupted_turn_tests;

pub(in crate::game) use sunrise_gameplay::crash_save::CrashSave;

pub enum ServerRunOutcome {
    /// Startup succeeded and the run stopped normally.
    Orderly,
    /// The dedicated server failed to bind its listener.
    FailedInit(std::io::Error),
}

impl ServerGame<'_> {
    /// `MinecraftServer.runServer`'s finally: once startup begins, every exit
    /// stops and saves the server, including a failed init or tick.
    pub fn run_server_lifecycle(
        &mut self,
        dedicated: bool,
        run: impl FnOnce(&mut Self) -> Result<ServerRunOutcome>,
    ) -> Result<()> {
        let run = catch_unwind(AssertUnwindSafe(|| run(self)));
        let crashed = run.is_err();
        let (orderly, result) = match run {
            Ok(Ok(ServerRunOutcome::Orderly)) => (true, Ok(())),
            Ok(Ok(ServerRunOutcome::FailedInit(error))) => {
                let message = format!("Failed to initialize server: {error}");
                log::error!("{message}");
                (false, Err(message.into()))
            }
            Ok(Err(error)) => (false, Err(error)),
            Err(payload) => {
                let message = super::dedicated::panic_message(payload.as_ref());
                log::error!("The server thread panicked: {message}");
                self.prepare_server_crash();
                (
                    false,
                    Err(format!("the server thread panicked: {message}").into()),
                )
            }
        };
        let shutdown = self.shutdown_started_server(dedicated, crashed, orderly);
        result.and(shutdown)
    }

    /// Vanilla's nested `stopServer` finally: a listener panic still reaches
    /// cleanup, storage release and `server_stopped`, and the error is kept.
    fn shutdown_started_server(
        &mut self,
        dedicated: bool,
        crashed: bool,
        orderly: bool,
    ) -> Result<()> {
        // There will be no next server tick; do not keep logs for another world.
        sunrise_level::logging::stop_tap();
        let mut failure = None;
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        self.native_lifecycle.begin_close();
        if orderly {
            if let Err(payload) =
                catch_unwind(AssertUnwindSafe(|| self.native_server_stopping(dedicated)))
            {
                remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()).into());
            }
            self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        }
        match catch_unwind(AssertUnwindSafe(|| {
            if dedicated {
                self.stop_dedicated(crashed)
            } else {
                self.stop_integrated(crashed)
            }
        })) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                log::error!("Exception stopping the server: {error}");
                failure.get_or_insert(error);
            }
            Err(payload) => {
                remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()).into());
            }
        }
        if let Err(payload) =
            catch_unwind(AssertUnwindSafe(|| self.native_server_stopped(dedicated)))
        {
            remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()).into());
        }
        // ServerLifecycleHooks.handleServerStopped posts the event before
        // unloading the world's SYNCED configs, even on a failed run.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.native_configs_server_stop())) {
            remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()).into());
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The crash entry's part of the decision: the interrupted players are excluded (see
    /// [`Self::exclude_interrupted_players`]); a lost hub also says players are not saved.
    fn prepare_server_crash(&mut self) {
        self.exclude_interrupted_players(true, |message| log::warn!("{message}"));
    }

    /// The one decision of a crash save, shared by the dedicated server, single player's server
    /// thread and the stop path, which applies it again after each shutdown callback panic it
    /// catches, before players are saved (#2704).
    ///
    /// A player's turn counts as interrupted when it is recorded (`interrupted_turns`: the turn
    /// helpers record the player on the way out of a panic, which also restores the outer
    /// player and clears the acting marker), when a swap that is still open displaces them, or
    /// when they are still swapped in as the acting player. Such a player is not saved: their
    /// last saved file, stats and advancements stay, for the half-done turn can hold an item
    /// taken but not yet placed. The player stays online (`placed`), so their logout still
    /// fires; only the saves read the exclusion. When that is the single-player owner, or the hub is lost
    /// (`crashed` and no hub, or an interrupted turn and no hub), `level.dat` is written without
    /// a new player file, keeping a legacy `Data.Player`.
    pub fn exclude_interrupted_players(&mut self, crashed: bool, warn: impl Fn(&str)) {
        let mut conns = self.server.interrupted_turns.clone();
        conns.extend(self.server.lan_guest_swaps.iter().copied());
        conns.extend(self.server.lan_acting);
        conns.sort_unstable();
        conns.dedup();
        self.server
            .crash_save
            .note_acting(self.server.lan_acting.is_some());
        let Some(host) = self.server.lan.as_mut() else {
            if (crashed || !conns.is_empty()) && self.server.crash_save.lose_hub() {
                warn(
                    "The panic interrupted the network tick: players are not saved, only the world",
                );
            }
            return;
        };
        for conn in conns {
            let owner = host.owner.as_ref().map(|o| o.uuid);
            let Some((name, is_owner)) = host.hub.guests.update_connection(conn, |guest| {
                (
                    guest.placed.then(|| guest.name.clone()),
                    Some(guest.uuid) == owner,
                )
            }) else {
                continue;
            };
            if is_owner {
                self.server.crash_save.keep_owner_file = true;
            }
            if let Some(name) = name
                && self.server.crash_save.exclude(conn)
            {
                warn(&format!(
                    "The panic interrupted a player's turn ({name}): that player keeps the last saved file"
                ));
            }
        }
    }

    /// Complete teardown even if freeing beds or the final save fails. A failed
    /// open must never overwrite a world the server did not successfully adopt.
    /// The session lock is not that marker: it is installed before the prepared
    /// level, so tag reads work, and a save in that gap would write the placeholder.
    pub fn finish_server_save(&mut self, dedicated: bool, crashed: bool) -> Result<()> {
        let before = self.server.save.0.record_save_failures();
        // 1. Network admission and published-state teardown (`getConnection().stop()`).
        self.stop_serving();
        self.stop_network_transport();

        // 2. Player save & removal (`PlayerList.saveAll`, `PlayerList.removeAll`).
        let mut result: Result<()> = Ok(());
        if dedicated && self.server.lan.is_some() {
            crate::game::server_side::dedicated::log("INFO", "Saving players");
        }
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.native_players_leaving()))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        if !crashed
            && !dedicated
            && let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.award_leave_game_to_all()))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        match catch_unwind(AssertUnwindSafe(|| {
            if crashed {
                Ok(())
            } else {
                self.free_player_beds()
            }
        })) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if result.is_ok() {
                    result = Err(error);
                }
            }
            Err(payload) => {
                if result.is_ok() {
                    result = Err(shutdown_panic(payload.as_ref()).into());
                }
            }
        }
        // `free_player_beds` can enter another level and trigger callbacks before
        // players are written.
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        let save_started = if self.server.world_adopted
            && self.server.session_lock.0.is_some()
            && mod_api::__private::enabled_listened(mod_api::HookId::WorldSaveStart)
        {
            let started = match catch_unwind(AssertUnwindSafe(|| self.begin_world_save(true))) {
                Ok(started) => started,
                Err(payload) => Err(shutdown_panic(payload.as_ref()).into()),
            };
            self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
            started
        } else {
            Ok(())
        };
        let owner_saved = match catch_unwind(AssertUnwindSafe(|| self.save_leaving_guests())) {
            Ok(saved) => saved,
            Err(payload) => {
                if result.is_ok() {
                    result = Err(shutdown_panic(payload.as_ref()).into());
                }
                false
            }
        };
        self.exclude_interrupted_players(crashed, |m| log::warn!("{m}"));
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.lan_disconnect(false)))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }

        let save_world = self.server.world_adopted && self.server.session_lock.0.is_some();
        if dedicated && save_world && save_started.is_ok() {
            crate::game::server_side::dedicated::log("INFO", "Saving worlds");
        }
        let saved = match save_started {
            Err(error) => Err(error),
            Ok(()) => match catch_unwind(AssertUnwindSafe(|| {
                if save_world {
                    // PlayerList already saved the live owner; the shell is not a player.
                    self.save_world_after_players(owner_saved)
                } else {
                    Ok(())
                }
            })) {
                Ok(result) => result,
                Err(payload) => Err(shutdown_panic(payload.as_ref()).into()),
            },
        };
        let save_ok = saved.is_ok();
        let failed = self.server.save.0.record_save_failures() - before;
        if crashed && failed != 0 {
            log::warn!(
                "Partial save during crash shutdown: {failed} chunk records could not be saved"
            );
        }
        if dedicated && save_world && save_ok {
            self.log_saved_levels();
        }
        result = result.and(saved);

        // A successful world save permits the per-level close loop.
        if save_ok
            && (self.server.world_adopted || self.server.levels.0.has_loaded_levels())
            && let Err(payload) =
                catch_unwind(AssertUnwindSafe(|| self.close_loaded_levels_for_stop()))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }

        // Retire the session even when a save or close fails.
        if let Err(payload) =
            catch_unwind(AssertUnwindSafe(|| self.server_lifecycle_world_closed()))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        self.server.session_lock.0 = None;
        self.server.interrupted_turns.clear();
        self.server.crash_save = Default::default();
        // Clear world maps even if startup failed after publishing them.
        if let Some(rest) = self.common.res.rest.get() {
            rest.behaviours.set_block_maps(None);
        }
        super::cauldron_interactions::clear();
        self.clear_world_parts();
        self.server.world_adopted = false;
        self.server.startup_ticket_restore_pending = false;
        result
    }
}

fn shutdown_panic(payload: &(dyn std::any::Any + Send)) -> String {
    format!(
        "the server shutdown panicked: {}",
        super::dedicated::panic_message(payload)
    )
}

fn remember_shutdown(
    failure: &mut Option<Box<dyn std::error::Error>>,
    error: Box<dyn std::error::Error>,
) {
    log::error!("{error}");
    failure.get_or_insert(error);
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
