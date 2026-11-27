use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Connecting / loading screens; true when it covered the frame.
    pub fn remote_overlay(&mut self, batch: &mut HudBatch) -> bool {
        if !self.remote_screen_up() {
            return false;
        }
        let Some(r) = &self.client.remote else {
            return false;
        };
        let loading = r.level_load.is_active();
        if self.common.player.is_some() && !loading {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        if let Some(since) = r.reconfiguring {
            // ServerReconfigScreen narrates its title.
            sunrise_client::widgets::begin_screen(|| {
                sunrise_client::widgets::ScreenInfo::new(
                    "reconfiguring",
                    super::super::screen_info::tr("connect.reconfiguring", &[]),
                )
            });
            self.draw_reconfiguring(batch, since);
            return true;
        }
        if r.joined || loading {
            // LevelLoadingScreen: the world's terrain comes in. A server's
            // chunk status view and progress are the integrated server's alone
            // (`LevelLoadTracker.statusView`, `hasProgress`), and so is what
            // the narrator says of it.
            let progress = self
                .client
                .integrated
                .as_ref()
                .map(|i| i.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            let begin_loading_screen = || {
                sunrise_client::widgets::begin_screen(|| {
                    super::super::screen_info::level_loading(
                        progress,
                        &super::super::screen_info::tr,
                    )
                });
            };
            begin_loading_screen();
            if r.level_load.transition_screen().is_some() {
                if self.draw_dimension_transition_screen(
                    batch,
                    &r.level_load,
                    progress.map(|_| self.client.loading_progress),
                ) {
                    return true;
                }
                // A failed presentation can also have registered widgets or narration.
                begin_loading_screen();
            }
            self.draw_level_loading_background(batch, r.level_load.reason);
            let atlas = &self.client.gui.atlas;
            match self.client.integrated.as_ref().map(|i| i.load_view()) {
                Some(view) => {
                    let status = |x: i32, z: i32| view.status(x, z);
                    let progress = view.has_progress().then_some(self.client.loading_progress);
                    sunrise_client::level_loading::draw(batch, atlas, Some(&status), progress);
                }
                None => sunrise_client::level_loading::draw(batch, atlas, None, None),
            }
            return true;
        }
        let cancel = self.connect_cancel((w, h)).is_some();
        // ConnectScreen.extractRenderState: "Joining the world" every 2 s (a
        // server's: single player's isn't joined through ConnectScreen).
        if self.client.integrated.is_none()
            && let Some(r) = self.client.remote.as_mut()
            && r.joining_narrated
                .is_none_or(|t| t.elapsed() > Duration::from_millis(2000))
        {
            r.joining_narrated = Some(Instant::now());
            self.say_system_now(&sunrise_game_data::text::Text::translate(
                "narrator.joining",
                &[],
            ));
        }
        let Some(r) = &self.client.remote else {
            return false;
        };
        // ConnectScreen: `GameNarrator.NO_TITLE`, then its Cancel; single
        // player's server is joined under its LevelLoadingScreen.
        let single_player = self.client.integrated.is_some();
        sunrise_client::widgets::begin_screen(|| {
            if single_player {
                super::super::screen_info::level_loading(None, &super::super::screen_info::tr)
            } else {
                sunrise_client::widgets::ScreenInfo::new("connect", "")
            }
        });
        let atlas = &self.client.gui.atlas;
        let status = sunrise_game_data::text::Text::untrusted(r.status.as_str());
        sunrise_client::level_loading::draw_connecting(
            batch,
            atlas,
            &status,
            self.client.cursor,
            cancel,
        );
        true
    }
    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too.
    /// For a mob `hurtOrSimulate` fails here (`Entity.hurtClient`), so the
    /// hit has no attacker side; but `RemotePlayer.hurtClient` is true, so a
    /// hit on another player runs the attacker's side: a critical hit's
    /// particles and `causeExtraKnockback` (`remote_attacker_knockback`).
    pub(in crate::game) fn remote_player_attack(&mut self, mob: usize) {
        // Java dispatches this through the target's RemotePlayer type. The
        // level_players map is profile bookkeeping and can lag the entity;
        // mobs must stay out because their hurtClient returns false.
        if self.common.mobs[mob].kind.id != "minecraft:player" {
            return;
        }
        let (damage, strength) = self.remote_attack_damage();
        // attackVisualEffects: a RemotePlayer's hurtClient is true, so a
        // critical hit (full strength, canCriticalAttack) with damage left
        // after baseDamageScaleFactor runs LocalPlayer.crit, this client's
        // own CRIT emitter beside the one the server's Animate brings.
        // magicBoost is 0 on a client, so no ENCHANTED_HIT one.
        if damage * (0.2 + strength * strength * 0.8) > 0.0 && self.common.critical_attack(strength)
        {
            use sunrise_level::particle_options::{ParticleOptions, ParticleType};
            let m = &self.common.mobs[mob];
            self.client.particles.0.tracking_emitter(
                ParticleOptions::simple(ParticleType::Crit),
                TrackingTarget::Entity(m.id as i32),
                mob_tracking_body(m),
                None,
            );
        }
        self.remote_attacker_knockback(damage, strength);
    }
    /// The inventory key when the vehicle owns its screen.
    pub(in crate::game) fn remote_request_mount_inventory(&self) {
        if let Some(r) = &self.client.remote {
            self.send(S::PlayerCommand(play::PlayerCommand {
                id: r.entity_id,
                action: 5,
                data: 0,
            }));
        }
    }
}
