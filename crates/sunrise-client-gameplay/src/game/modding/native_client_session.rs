//! Read-only projections of the connected client's existing state.

use crate::game::client_side::remote_client::Remote;
use core::ops::ControlFlow;
#[cfg(test)]
use mod_api::prelude::*;
use mod_api::{
    BlockBreakingState, BlockPos, ClientFrameStats, ClientServer, ClientSessionReadError,
    ClientTabList, ClientTabListEntry, GameMode, PlayerRef, Text, TextFormError,
};
use std::sync::Arc;
use sunrise_client::tab_list::{name_for_display, sorted_players};
use sunrise_game_data::scoreboard::Scoreboard;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;
use sunrise_level::profile_lookup::TabPlayer;

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn native_session(&self) -> Option<&Remote> {
        self.common.native_window().then_some(())?;
        self.common.native_lifecycle.epoch()?;
        self.client
            .remote
            .as_ref()
            .filter(|remote| remote.logged_in)
    }

    /// The connected server, not a stale reconnect target. None outside a
    /// logged-in client world. External display names are not retained; memory
    /// connections have no address. Friend-invite secrets are redacted.
    pub(in crate::game) fn native_client_server(&self) -> Option<ClientServer<'_>> {
        let remote = self.native_session()?;
        let integrated = self.client.integrated.as_ref();
        Some(ClientServer {
            name: integrated.map(|server| server.world_name.as_str()),
            address: remote.display_address.as_deref(),
            integrated_local: integrated.is_some(),
            save_folder: integrated.map(|server| server.save_folder.as_str()),
        })
    }

    /// A snapshot of the applied language code, not a pending options selection.
    /// None outside a logged-in client world.
    pub(in crate::game) fn native_client_language(&self) -> Option<Arc<str>> {
        self.native_session()?;
        Some(sunrise_game_data::lang::code_snapshot())
    }

    /// `PlayerInfo.getLatency` of the local player in milliseconds, 0 before the server's first
    /// measurement; None outside a logged-in client world or before the server lists the player.
    pub(in crate::game) fn native_client_latency(&self) -> Option<u32> {
        self.native_session()?;
        let latency = self.own_latency()?;
        Some(u32::try_from(latency).unwrap_or(0))
    }

    /// The locator bar's tracked waypoints; None outside a logged-in client world.
    pub(in crate::game) fn native_client_waypoints(&self) -> Option<mod_api::ClientWaypoints> {
        self.native_session()?;
        Some(waypoints(
            &self.client.waypoints,
            self.camera_entity_position(),
        ))
    }

    /// The listed players' skins as the Tab overlay draws them; None outside a logged-in
    /// client world.
    pub(in crate::game) fn native_client_player_skins(&self) -> Option<mod_api::ClientPlayerSkins> {
        let remote = self.native_session()?;
        Some(player_skins(&remote.listed_player_skins()))
    }

    /// The local player's item use (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`,
    /// `getUseItemRemainingTicks`); None when no item is in use or outside a logged-in client
    /// world.
    pub(in crate::game) fn local_item_use(&self) -> Option<mod_api::ItemUse> {
        self.native_session()?;
        let (_, ticks) = self.common.using?;
        let ticks_used = i32::try_from(ticks).unwrap_or(i32::MAX);
        let hand = match self.common.used_hand()? {
            sunrise_sim::first_person_sim::Hand::Main => mod_api::Hand::Main,
            sunrise_sim::first_person_sim::Hand::Off => mod_api::Hand::Off,
        };
        Some(mod_api::ItemUse::new(
            hand,
            ticks_used,
            self.common.use_duration.saturating_sub(ticks_used),
        ))
    }

    /// The local player's fishing hook: its position and whether a fish is biting. None
    /// when no hook is out or outside a logged-in client world.
    pub(in crate::game) fn native_client_fishing_hook(&self) -> Option<mod_api::ClientFishingHook> {
        self.native_session()?;
        let (position, biting) = self.local_fishing_hook()?;
        Some(mod_api::ClientFishingHook::new(
            mod_api::Vec3::new(position.x, position.y, position.z),
            biting,
        ))
    }

    /// `Minecraft.rightClickDelay`, which `startUseItem` sets to 4 and the client tick counts
    /// down; None outside a client world.
    pub(in crate::game) fn right_click_delay(&self) -> Option<u32> {
        (self.native_session().is_some() && self.common.player.is_some())
            .then_some(self.common.use_delay)
    }

    /// Sets `Minecraft.rightClickDelay`; false, changing nothing, outside a client world.
    pub(in crate::game) fn set_right_click_delay(&mut self, ticks: u32) -> bool {
        if self.native_session().is_none() || self.common.player.is_none() {
            return false;
        }
        self.common.use_delay = ticks;
        true
    }

    /// `setYRot` / `setXRot` of the local player by a client mod: yaw as given, pitch `% 360`
    /// clamped to -90..90, the previous angles moved by as much (`Entity.turn`). Sends nothing:
    /// the tick's player sync (`remote_send_state`) carries the look.
    pub fn set_local_rotation(
        &mut self,
        yaw: f32,
        pitch: f32,
    ) -> Result<(), mod_api::LocalRotationError> {
        if self.native_session().is_none() || self.common.player.is_none() {
            return Err(mod_api::LocalRotationError::NoWorld);
        }
        if !(yaw.is_finite() && pitch.is_finite()) {
            return Err(mod_api::LocalRotationError::NotFinite);
        }
        let pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        let (turned_yaw, turned_pitch) = (yaw - self.common.yaw, pitch - self.common.pitch);
        self.client.rope_rot_old.0 += turned_yaw;
        self.client.rope_rot_old.1 += turned_pitch;
        self.common.yaw = yaw;
        self.common.pitch = pitch;
        if let Some(model) = self.client.player_model.as_mut() {
            model.previous_head_pitch += turned_pitch;
            model.head_pitch = pitch;
        }
        Ok(())
    }

    /// Existing local frame counters; None outside a logged-in client world.
    /// This read collects no history and sends nothing anywhere.
    pub(in crate::game) fn native_client_frame_stats(&self) -> Option<ClientFrameStats> {
        self.native_session()?;
        Some(ClientFrameStats {
            fps: self.client.fps,
            rendered_frames: self.client.frames,
            frame_time_ns: self
                .client
                .debug_charts
                .frame_times
                .latest()
                .and_then(|time| u64::try_from(time).ok()),
        })
    }

    /// The title, subtitle and action bar on screen now: a title or action bar whose time has
    /// run out reads as None, and a subtitle shows only with its title. None outside a
    /// logged-in client world. A text the API cannot carry is returned as an error, never
    /// as plain words.
    pub(in crate::game) fn native_client_titles(
        &self,
    ) -> Result<Option<mod_api::ClientTitles>, ClientSessionReadError> {
        if self.native_session().is_none() {
            return Ok(None);
        }
        let t = &self.client.titles;
        let title = t.title.clone().filter(|_| t.time > 0);
        let subtitle = title.as_ref().and(t.subtitle.clone());
        let action_bar = t.overlay.clone().filter(|_| t.overlay_time > 0);
        let text = |text: &sunrise_game_data::text::Text| text.try_into_api();
        Ok(Some(mod_api::ClientTitles::new(
            title.as_ref().map(text).transpose()?,
            subtitle.as_ref().map(text).transpose()?,
            action_bar.as_ref().map(text).transpose()?,
        )))
    }

    /// Every boss bar the client holds, in server order. The HUD draws only those that fit
    /// within `guiHeight / 3`. None outside a
    /// logged-in client world. A text the API cannot carry is returned as an error, never
    /// as plain words.
    pub(in crate::game) fn native_client_boss_bars(
        &self,
    ) -> Result<Option<Vec<mod_api::ClientBossBarEntry>>, ClientSessionReadError> {
        if self.native_session().is_none() {
            return Ok(None);
        }
        let bars = self
            .client
            .boss_overlay
            .bars(sunrise_client::boss_overlay::now_millis())
            .into_iter()
            .map(|bar| {
                Ok(mod_api::ClientBossBarEntry::new(
                    bar.name.try_into_api()?,
                    bar.progress,
                ))
            })
            .collect::<Result<Vec<_>, ClientSessionReadError>>()?;
        Ok(Some(bars))
    }

    pub(in crate::game) fn native_local_status(&self, player: PlayerRef) -> Option<()> {
        self.native_session()?;
        (PlayerIdentityGameExt1::local_player(self)? == player).then_some(())
    }

    /// The local player's predicted breaking state, including Idle. None for
    /// stale, nonlocal or fake handles and outside a logged-in client world.
    pub(in crate::game) fn player_block_breaking(
        &self,
        player: PlayerRef,
    ) -> Option<BlockBreakingState> {
        self.native_local_status(player)?;
        Some(
            self.common
                .mining
                .as_ref()
                .map_or(BlockBreakingState::Idle, |mining| {
                    BlockBreakingState::Breaking {
                        position: BlockPos::new(mining.pos[0], mining.pos[1], mining.pos[2]),
                        progress: mining.progress,
                        stage: mining.stage(),
                    }
                }),
        )
    }

    /// The local player's existing cave ambience mood, normally 0 through 1.
    /// None for stale, nonlocal or fake handles and outside a client world.
    pub(in crate::game) fn player_ambient_mood(&self, player: PlayerRef) -> Option<f32> {
        self.native_local_status(player)?;
        Some(self.client.ambience.moodiness)
    }
}

impl ClientTabList for crate::game::ClientGame<'_> {
    fn visit_entries(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTabListEntry) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        visit_tab_entries(
            self.client
                .remote
                .iter()
                .flat_map(Remote::listed_tab_player_refs),
            &self.common.score.board,
            visit,
        )
    }

    fn copy_header(&self) -> Result<Option<Text>, TextFormError> {
        self.client
            .tab_list
            .0
            .header
            .as_ref()
            .map(TryIntoApi::try_into_api)
            .transpose()
    }

    fn copy_footer(&self) -> Result<Option<Text>, TextFormError> {
        self.client
            .tab_list
            .0
            .footer
            .as_ref()
            .map(TryIntoApi::try_into_api)
            .transpose()
    }
}

// Keep the overlay's ordering and cap, lending one stack-scoped row at a time.
fn visit_tab_entries<'a>(
    players: impl IntoIterator<Item = &'a TabPlayer>,
    board: &'a Scoreboard,
    visit: &mut dyn FnMut(&dyn ClientTabListEntry) -> ControlFlow<()>,
) -> ControlFlow<()> {
    sorted_players(players, board).try_for_each(|player| visit(&TabEntryView { player, board }))
}

struct TabEntryView<'a> {
    player: &'a TabPlayer,
    board: &'a Scoreboard,
}

impl ClientTabListEntry for TabEntryView<'_> {
    fn name(&self) -> &str {
        &self.player.name
    }

    fn uuid(&self) -> u128 {
        self.player.uuid
    }

    fn game_mode(&self) -> GameMode {
        // `GameType.byId` returns survival for an unknown id.
        GameMode::from_id(self.player.game_mode).unwrap_or(GameMode::Survival)
    }

    fn latency(&self) -> i32 {
        self.player.latency
    }

    fn copy_display(&self) -> Result<Text, TextFormError> {
        name_for_display(self.player, self.board).try_into_api()
    }
}

/// `ClientWaypointManager`'s table, in `forEachWaypoint`'s read order: farthest first by
/// `distanceSquared` with a camera entity, no promised order without one, so the read cannot
/// drift from what the bar draws.
fn waypoints(
    waypoints: &sunrise_client::locator_bar::ClientWaypoints,
    camera_entity_pos: Option<[f64; 3]>,
) -> mod_api::ClientWaypoints {
    use sunrise_protocol::packets::play::{WaypointId, WaypointTarget};
    let rows = waypoints.for_read(camera_entity_pos);
    let entries = rows
        .iter()
        .filter_map(|w| {
            let id = match &w.id {
                WaypointId::Uuid(uuid) => mod_api::ClientWaypointId::Uuid(*uuid),
                WaypointId::Name(name) => mod_api::ClientWaypointId::Name(name.clone()),
            };
            let target = match w.target {
                WaypointTarget::Empty => mod_api::ClientWaypointTarget::Empty,
                WaypointTarget::Block([x, y, z]) => {
                    mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(x, y, z))
                }
                WaypointTarget::Chunk([x, z]) => mod_api::ClientWaypointTarget::Chunk([x, z]),
                WaypointTarget::Azimuth(angle) => mod_api::ClientWaypointTarget::Azimuth(angle),
            };
            // A style the wire validator did not accept skips the entry instead of
            // panicking on peer data.
            let style = mod_api::Identifier::parse(&w.style).ok()?;
            Some(mod_api::ClientWaypointEntry::new(
                id,
                target,
                style,
                // The packet stores the colour as ARGB; the API takes RGB, as
                // `living_set_waypoint_color` does.
                w.color.map(|c| (c as u32) & 0x00FF_FFFF),
            ))
        })
        .collect();
    mod_api::ClientWaypoints::new(entries)
}

/// `Remote::listed_player_skins` as `ClientPlayerSkin`s, so the read cannot drift from what the
/// tab list draws.
fn player_skins(skins: &[(u128, String, bool)]) -> mod_api::ClientPlayerSkins {
    let entries = skins
        .iter()
        .map(|(uuid, skin, slim)| mod_api::ClientPlayerSkin::new(*uuid, skin.clone(), *slim))
        .collect();
    mod_api::ClientPlayerSkins::new(entries)
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(test)]
    use super::super::native_entity_refs::tests::with_client_game;
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use sunrise_core::nbt::Tag;
    #[cfg(test)]
    use sunrise_game_data::scoreboard::tag_to_json;
    #[cfg(test)]
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;

    /// Plain text of a display value: the literal words plus the children's, in order.
    #[cfg(test)]
    fn plain(text: &Text) -> String {
        let own = match &text.content {
            mod_api::text::TextContent::Literal(words) => words.as_str(),
            _ => "",
        };
        text.children
            .iter()
            .map(plain)
            .fold(own.to_owned(), |all, c| all + &c)
    }

    /// The Tab overlay's borrowed rows, order, cap and rich names.
    #[cfg(test)]
    #[test]
    fn the_tab_list_reads_as_the_overlay_draws_it() {
        let mut board = Scoreboard::default();
        let team = board.add_team("red");
        team.prefix = serde_json::json!("[R] ");
        team.suffix = serde_json::json!(" *");
        board.add_to_team("Alex", "red");
        let mut named = TabPlayer::new(1, "Steve");
        named.display_name = Some(serde_json::json!({
            "text": "Captain",
            "test:opaque": [null], // Unknown component fields aren't part of the display name.
        }));
        named.latency = 42;
        let teamed = TabPlayer::new(2, "Alex");
        let mut watcher = TabPlayer::new(3, "Bob");
        watcher.game_mode = 3;
        let mut first = TabPlayer::new(4, "Zed");
        first.list_order = 5;
        let mut wrapping = TabPlayer::new(5, "Wrap");
        wrapping.list_order = i32::MIN;
        let players = [watcher, named, teamed, first, wrapping];
        let mut names = Vec::new();
        assert_eq!(
            visit_tab_entries(&players, &board, &mut |row| {
                names.push(row.name().to_owned());
                match row.uuid() {
                    1 => {
                        assert_eq!(row.latency(), 42);
                        assert_eq!(row.copy_display(), Ok(Text::literal("Captain")));
                    }
                    2 => assert_eq!(plain(&row.copy_display().unwrap()), "[R] Alex *"),
                    3 => {
                        assert_eq!(row.game_mode(), GameMode::Spectator);
                        assert_eq!(row.copy_display().unwrap().style.italic, Some(true));
                    }
                    4 => assert_eq!(row.game_mode(), GameMode::Survival),
                    _ => {}
                }
                ControlFlow::Continue(())
            }),
            ControlFlow::Continue(())
        );
        assert_eq!(names, ["Wrap", "Zed", "Steve", "Alex", "Bob"]);

        let crowd: Vec<_> = (0..100)
            .rev()
            .map(|i| TabPlayer::new(i, format!("p{i:03}")))
            .collect();
        let mut ids = Vec::new();
        assert_eq!(
            visit_tab_entries(&crowd, &board, &mut |row| {
                ids.push(row.uuid());
                ControlFlow::Continue(())
            }),
            ControlFlow::Continue(())
        );
        assert!(
            ids.into_iter().eq(0_u128..80),
            "only the first 80 in drawn order"
        );

        let ties: Vec<_> = (0..81)
            .rev()
            .map(|i| TabPlayer::new(i, if i % 2 == 0 { "Alex" } else { "alex" }))
            .collect();
        let no_teams = Scoreboard::default();
        let mut ids = Vec::new();
        assert_eq!(
            visit_tab_entries(&ties, &no_teams, &mut |row| {
                ids.push(row.uuid());
                ControlFlow::Continue(())
            }),
            ControlFlow::Continue(())
        );
        assert!(
            ids.into_iter().eq((1_u128..81).rev()),
            "even the cutoff preserves ties"
        );

        let mut uncopyable = TabPlayer::new(42, "A");
        // The hover item retains its custom data, but the packet's NBT-to-JSON
        // projection turns the non-finite number into a nested null.
        uncopyable.display_name = Some(tag_to_json(&Tag::compound([
            ("text", Tag::String("Hover for details".into())),
            (
                "hover_event",
                Tag::compound([
                    ("action", Tag::String("show_item".into())),
                    ("id", Tag::String("minecraft:stone".into())),
                    (
                        "components",
                        Tag::compound([(
                            "minecraft:custom_data",
                            Tag::compound([("example", Tag::Double(f64::NAN.to_bits()))]),
                        )]),
                    ),
                ]),
            ),
        ])));
        let mut seen = Vec::new();
        assert_eq!(
            visit_tab_entries([&uncopyable, &players[2]], &board, &mut |row| {
                seen.push(row.uuid());
                if row.uuid() == 42 {
                    assert_eq!(row.name(), "A");
                    assert_eq!(row.copy_display(), Err(TextFormError::NestedNull));
                    return ControlFlow::Continue(());
                }
                assert_eq!(plain(&row.copy_display().unwrap()), "[R] Alex *");
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
        assert_eq!(seen, [42, 2], "a text refusal does not stop the visit");
        assert_eq!(
            visit_tab_entries(&players, &board, &mut |row| {
                assert_eq!(row.name(), "Wrap", "no calls after Break");
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
    }

    /// `LocalPlayer.permissionLevel` (#5529): on a client the local player's level is the one the
    /// server's entity events 24 to 28 last set, and another player's is unknown.
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn the_local_players_permission_level_follows_the_servers_events() {
        use mod_api::{EntityRef, WorldEpoch};
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        with_client_game(|game| {
            let _server = logged_client(game);
            let player = PlayerIdentityGameExt1::local_player(game).unwrap();
            assert_eq!(player.permission_level(game), Some(0));
            for (event, level) in [(26, 2), (28, 4), (24, 0)] {
                game.handle_packet(C::EntityEvent(play::EntityEvent { id: 7, event }))
                    .unwrap();
                assert_eq!(player.permission_level(game), Some(level), "event {event}");
            }
            // Another entity's event leaves the local player's level alone.
            game.handle_packet(C::EntityEvent(play::EntityEvent { id: 8, event: 28 }))
                .unwrap();
            assert_eq!(player.permission_level(game), Some(0));
            let epoch: WorldEpoch = player.entity().epoch();
            let other = mod_api::__private::game::player_ref(EntityRef::new(epoch, 99), 5);
            assert_eq!(other.permission_level(game), None);
        });
    }

    /// The waypoint table as `client_waypoints` reads it: every tracked entry, in sent order.
    #[cfg(test)]
    #[test]
    fn the_waypoints_read_as_the_bar_holds_them() {
        use sunrise_client::locator_bar::ClientWaypoints;
        use sunrise_protocol::packets::play::{
            TrackedWaypoint, WaypointId, WaypointTarget, waypoint_operation,
        };
        let tracked = |id, style: &str, color, target| TrackedWaypoint {
            operation: waypoint_operation::TRACK,
            id,
            style: style.into(),
            color,
            target,
        };
        let mut table = ClientWaypoints::default();
        // Sent nearest-first, with the azimuth in the middle: nothing here depends on sent order.
        table.apply(&tracked(
            WaypointId::Uuid(7),
            "minecraft:default",
            Some(0xFF10_2030u32 as i32),
            WaypointTarget::Block([2, 64, 3]), // nearest
        ));
        table.apply(&tracked(
            WaypointId::Name("home".into()),
            "minecraft:default",
            None,
            WaypointTarget::Azimuth(0.5),
        ));
        table.apply(&tracked(
            WaypointId::Uuid(9),
            "minecraft:default",
            Some(0xFF00_FF00u32 as i32),
            WaypointTarget::Block([0, 64, 20]), // middle
        ));
        table.apply(&tracked(
            WaypointId::Uuid(11),
            "minecraft:default",
            Some(0xFF10_2040u32 as i32),
            WaypointTarget::Block([30, 64, 0]), // farthest
        ));
        let list = waypoints(&table, Some([0.0, 64.0, 0.0]));
        assert_eq!(
            list.entries.len(),
            4,
            "every tracked waypoint, in the bar's read order"
        );
        // With a camera at [0, 64, 0] the read is farthest first by squared distance from the
        // block centres: the azimuth (no distance at all) first, then [30, 64, 0] (centre
        // 930.75), [0, 64, 20] (420.75) and [2, 64, 3] (18.75) — the reverse of sent order.
        let order: Vec<_> = list.entries.iter().map(|e| e.id.clone()).collect();
        assert_eq!(
            order,
            [
                mod_api::ClientWaypointId::Name("home".into()),
                mod_api::ClientWaypointId::Uuid(11),
                mod_api::ClientWaypointId::Uuid(9),
                mod_api::ClientWaypointId::Uuid(7),
            ],
            "the read shares the locator bar's farthest-first sort"
        );
        let azimuth = &list.entries[0];
        assert_eq!(azimuth.target, mod_api::ClientWaypointTarget::Azimuth(0.5));
        assert_eq!(azimuth.color, None, "no colour sent stays none");
        let far = &list.entries[1];
        assert_eq!(
            far.target,
            mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(30, 64, 0))
        );
        assert_eq!(far.style.to_string(), "minecraft:default");
        assert_eq!(far.color, Some(0x10_2040), "ARGB on the wire reads as RGB");
        let mid = &list.entries[2];
        assert_eq!(
            mid.target,
            mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(0, 64, 20))
        );
        assert_eq!(mid.color, Some(0x00_FF00));
        let near = &list.entries[3];
        assert_eq!(
            near.target,
            mod_api::ClientWaypointTarget::Block(mod_api::BlockPos::new(2, 64, 3))
        );
        assert_eq!(near.color, Some(0x10_2030));
        // Untracking drops the entry; without a camera entity no order is promised.
        table.apply(&TrackedWaypoint {
            operation: waypoint_operation::UNTRACK,
            id: WaypointId::Uuid(11),
            style: "minecraft:default".into(),
            color: None,
            target: WaypointTarget::Empty,
        });
        let list = waypoints(&table, None);
        assert_eq!(list.entries.len(), 3, "untracked waypoints leave the table");
        assert_eq!(
            list.entries[1].id,
            mod_api::ClientWaypointId::Name("home".into())
        );
    }

    /// The listed players' skins as `client_player_skins` reads them: the resolved path and the
    /// slim flag, joined on UUID.
    #[cfg(test)]
    #[test]
    fn the_player_skins_read_as_the_tab_list_draws_them() {
        use sunrise_client::skins::ResolvedSkin;
        // Listed uuids need a tab entry to be drawn; 4 has none and must not appear.
        let listed: [u128; 4] = [1, 2, 3, 4];
        let tab_players: std::collections::HashMap<u128, TabPlayer> = [
            (1, TabPlayer::new(1, "Slim")),
            (2, TabPlayer::new(2, "Wide")),
            (3, TabPlayer::new(3, "Default")),
        ]
        .into();
        let resolved: std::collections::HashMap<u128, ResolvedSkin> = [
            (
                1,
                ResolvedSkin {
                    custom_body: Some(("skins/abc123".into(), true)),
                    ..ResolvedSkin::default()
                },
            ),
            (
                2,
                ResolvedSkin {
                    custom_body: Some(("skins/wide42".into(), false)),
                    ..ResolvedSkin::default()
                },
            ),
        ]
        .into();
        let mut rows = crate::game::client_side::remote_client::listed_player_skins(
            listed.iter(),
            &tab_players,
            &resolved,
        );
        rows.sort_by_key(|row| row.0);
        assert_eq!(
            rows,
            vec![
                (1, "skins/abc123".to_owned(), true),
                (2, "skins/wide42".to_owned(), false),
                // No resolved skin: the UUID's default body and its own slim flag.
                (3, "entity/player/slim/kai".to_owned(), true),
            ],
            "every drawn player's skin, and no skin for the uuid without a tab entry"
        );
        let skins = player_skins(
            &rows
                .iter()
                .map(|(uuid, skin, slim)| (*uuid, skin.clone(), *slim))
                .collect::<Vec<_>>(),
        );
        assert_eq!(skins.entries.len(), 3);
        assert_eq!(skins.entries[0].uuid, 1);
        assert_eq!(skins.entries[0].skin, "skins/abc123");
        assert!(skins.entries[0].slim, "a downloaded slim skin stays slim");
        assert_eq!(skins.entries[1].uuid, 2);
        assert!(!skins.entries[1].slim, "a wide skin is not slim");
        assert_eq!(skins.entries[2].uuid, 3);
        assert_eq!(skins.entries[2].skin, rows[2].1.clone());
        assert_eq!(
            skins.entries[2].slim, rows[2].2,
            "the default path keeps its slim flag"
        );
    }

    /// The fishing hook a client reads is the local player's own, and its bite follows
    /// what the server sent.
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn fishing_hook_read_follows_the_local_players_bobber() {
        use sunrise_protocol::{
            entity_data::{Value, serializer},
            packets::play::{self, Clientbound as C},
        };

        with_client_game(|game| {
            assert_eq!(game.client_session().client_fishing_hook(), None);
            let _server = logged_client(game);
            // No hook out yet.
            assert_eq!(game.client_session().client_fishing_hook(), None);

            let kind = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .and_then(|ids| ids.id("minecraft:fishing_bobber"))
                .expect("the builtin registries list the fishing bobber");
            // The server's add packet for the local player's bobber: the owner's id
            // is the data, and the local player's entity id is 7 here.
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 11,
                uuid: 1,
                kind,
                position: [4.0, 5.0, 6.0],
                velocity: [0.0, 0.0, 0.0],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 7,
            }))
            .unwrap();
            let hook = game.client_session().client_fishing_hook().unwrap();
            assert_eq!(hook.position, mod_api::Vec3::new(4.0, 5.0, 6.0));
            assert!(!hook.biting, "no bite yet");

            // DATA_BITING from the server.
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 11,
                values: vec![(9, serializer::BOOLEAN, Value::Bool(true))],
            }))
            .unwrap();
            assert!(
                game.client_session().client_fishing_hook().unwrap().biting,
                "the server's bite shows"
            );
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 11,
                values: vec![(9, serializer::BOOLEAN, Value::Bool(false))],
            }))
            .unwrap();
            assert!(
                !game.client_session().client_fishing_hook().unwrap().biting,
                "the bite ends"
            );

            // Another player's bobber does not move the read.
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 12,
                uuid: 2,
                kind,
                position: [40.0, 50.0, 60.0],
                velocity: [0.0, 0.0, 0.0],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 8,
            }))
            .unwrap();
            let hook = game.client_session().client_fishing_hook().unwrap();
            assert_eq!(hook.position, mod_api::Vec3::new(4.0, 5.0, 6.0));
        });
    }

    pub fn logged_client(
        game: &mut crate::game::ClientGame<'_>,
    ) -> sunrise_protocol::connection::Pipe {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "SessionReadTest").unwrap();
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        server
    }

    /// Drive the server's side of a memory client login through to play, so a test can
    /// read packets from the ordinary client sender.
    pub fn finish_login(
        game: &mut crate::game::ClientGame<'_>,
        server: &mut sunrise_protocol::connection::Connection,
    ) {
        use sunrise_protocol::packets::{Empty, configuration as cfg, login};
        server.recv().unwrap(); // Intention.
        server.recv().unwrap(); // Hello.
        server
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "SessionReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        server.recv().unwrap(); // Login acknowledged.
        server.recv().unwrap(); // Brand.
        server.recv().unwrap(); // Client information.
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        game.acknowledge_test_configuration();
        server.recv().unwrap(); // Finish configuration.
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn client_session_reads_follow_connection_language_and_frame_state() {
        use core::ops::ControlFlow;
        use sunrise_core::nbt::Tag;
        use sunrise_game_data::lang::{self, Language};
        use sunrise_protocol::packets::play::{
            Clientbound, PlayerInfo, PlayerInfoUpdate, SetObjective, TabList,
        };

        with_client_game(|game| {
            assert!(matches!(game.client_session().client_tab_list(), Ok(None)));
            assert_eq!(game.client_session().client_server(), None);
            assert_eq!(game.client_session().client_language(), None);
            assert_eq!(game.client_session().client_frame_stats(), None);
            assert!(matches!(
                ScoreboardClientHost::client_scoreboard(game),
                Ok(None)
            ));
            let _server = logged_client(game);
            let empty_list = game.client_session().client_tab_list().unwrap().unwrap();
            assert_eq!(
                empty_list.visit_entries(&mut |_| panic!("new session's tab list is empty")),
                ControlFlow::Continue(())
            );
            assert_eq!(empty_list.copy_header(), Ok(None));
            assert_eq!(empty_list.copy_footer(), Ok(None));
            game.apply_packet(Clientbound::PlayerInfoUpdate(PlayerInfoUpdate {
                actions: 1 | 8,
                entries: vec![
                    PlayerInfo {
                        uuid: 1,
                        name: Some("Hidden".to_owned()),
                        listed: Some(false),
                        ..PlayerInfo::default()
                    },
                    PlayerInfo {
                        uuid: 2,
                        name: Some("Listed".to_owned()),
                        listed: Some(true),
                        ..PlayerInfo::default()
                    },
                ],
            }))
            .unwrap();
            let list = game.client_session().client_tab_list().unwrap().unwrap();
            let mut listed = Vec::new();
            assert_eq!(
                list.visit_entries(&mut |entry| {
                    listed.push(entry.uuid());
                    ControlFlow::Continue(())
                }),
                ControlFlow::Continue(())
            );
            assert_eq!(
                listed,
                [2],
                "only players the server marked listed are exposed"
            );
            game.apply_packet(Clientbound::TabList(TabList {
                header: Tag::String("Header".into()),
                footer: Tag::String("Footer".into()),
            }))
            .unwrap();
            let list = game.client_session().client_tab_list().unwrap().unwrap();
            assert_eq!(list.copy_header(), Ok(Some(Text::literal("Header"))));
            assert_eq!(list.copy_footer(), Ok(Some(Text::literal("Footer"))));
            game.apply_packet(Clientbound::TabList(TabList {
                header: Tag::String("".into()),
                footer: Tag::String("".into()),
            }))
            .unwrap();
            let list = game.client_session().client_tab_list().unwrap().unwrap();
            assert_eq!(list.copy_header(), Ok(None));
            assert_eq!(list.copy_footer(), Ok(None));

            let empty_board = ScoreboardClientHost::client_scoreboard(game)
                .unwrap()
                .unwrap();
            assert_eq!(
                empty_board.visit_objectives(&mut |_| panic!("new session's board is empty")),
                ControlFlow::Continue(())
            );
            game.remote_objective(SetObjective {
                name: "received".into(),
                method: 0,
                display: Some((Tag::String("Received".into()), 0, None)),
            });
            let live_board = ScoreboardClientHost::client_scoreboard(game)
                .unwrap()
                .unwrap();
            assert_eq!(
                live_board.visit_objectives(&mut |objective| {
                    assert_eq!(objective.name(), "received");
                    ControlFlow::Break(())
                }),
                ControlFlow::Break(())
            );
            let memory = game.client_session().client_server().unwrap();
            assert_eq!(memory.name, None);
            assert_eq!(memory.address, None);
            assert!(
                !memory.integrated_local,
                "a memory guest does not own the server"
            );

            // Exercise the same display-address calculation as Remote construction without
            // opening an extra network connection for either address.
            for (address, display) in [
                ("127.0.0.1:25565", "127.0.0.1:25565"),
                ("friend:private-invite", "Friend's world"),
            ] {
                let remote = game.client.remote.as_mut().unwrap();
                remote.session.address = address.into();
                remote.display_address = Remote::display_address_for(&remote.session.address);
                let external = game.client_session().client_server().unwrap();
                assert_eq!(external.address, Some(display));
                assert_eq!(
                    external.name, None,
                    "Session.name is the player, not the server"
                );
                assert!(
                    !external.integrated_local,
                    "an address does not determine ownership"
                );
            }

            game.client.settings.language = "fr_fr".into();
            let _publication = lang::isolate_publication(Language::of("es_es", []));
            let outer = game.client_session().client_language().unwrap();
            assert_eq!(outer.as_ref(), "es_es");
            let inner = {
                let _nested = lang::isolate_publication(Language::of("en_us", []));
                lang::set(Language::of("de_de", []));
                let inner = game.client_session().client_language().unwrap();
                assert_eq!(inner.as_ref(), "de_de");
                lang::scoped(Language::of("ja_jp", []), || {
                    assert_eq!(
                        game.client_session().client_language().as_deref(),
                        Some("ja_jp")
                    );
                });
                assert_eq!(lang::code(), "de_de");
                inner
            };
            assert_eq!(
                inner.as_ref(),
                "de_de",
                "the snapshot outlives its publication"
            );
            assert_eq!(
                game.client_session().client_language().as_deref(),
                Some("es_es")
            );
            assert_eq!(
                outer.as_ref(),
                "es_es",
                "publication does not mutate earlier snapshots"
            );
            game.client.debug_charts.frame_times.reset();
            assert_eq!(
                game.client_session()
                    .client_frame_stats()
                    .unwrap()
                    .frame_time_ns,
                None
            );
            game.client.fps = 63.5;
            game.client.frames = 99;
            game.client.debug_charts.frame_times.log(15_432_123);
            assert_eq!(
                game.client_session().client_frame_stats(),
                Some(ClientFrameStats {
                    fps: 63.5,
                    rendered_frames: 99,
                    frame_time_ns: Some(15_432_123),
                })
            );
            game.client.fps = 59.0;
            game.client.frames = 100;
            game.client.debug_charts.frame_times.log(17_123_456);
            assert_eq!(
                game.client_session().client_frame_stats(),
                Some(ClientFrameStats {
                    fps: 59.0,
                    rendered_frames: 100,
                    frame_time_ns: Some(17_123_456),
                })
            );
            game.pause = true;
            assert!(
                game.is_paused(),
                "the existing applied pause read remains canonical"
            );
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert!(matches!(game.client_session().client_tab_list(), Ok(None)));
            assert_eq!(game.client_session().client_server(), None);
            assert_eq!(game.client_session().client_language(), None);
            assert_eq!(game.client_session().client_frame_stats(), None);
            assert!(matches!(
                ScoreboardClientHost::client_scoreboard(game),
                Ok(None)
            ));
            game.client.remote = None;
            // A reconnect target must not be mistaken for the current session.
            game.connection_starts("stale.example:25565", "OldPlayer", false);
            assert_eq!(game.client_session().client_server(), None);
            assert!(matches!(
                ScoreboardClientHost::client_scoreboard(game),
                Ok(None)
            ));
        });
    }

    /// The titles and boss bars a client reads are the ones the server sent it.
    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn title_and_boss_bar_reads_follow_what_the_server_sent() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::packets::play::{self, Clientbound as C};

        fn s(text: &str) -> Tag {
            Tag::String(text.into())
        }

        with_client_game(|game| {
            assert!(matches!(game.client_session().client_titles(), Ok(None)));
            assert!(matches!(game.client_session().client_boss_bars(), Ok(None)));
            let _server = logged_client(game);
            // Nothing shown yet: an empty snapshot, not None.
            let titles = game.client_session().client_titles().unwrap().unwrap();
            assert!(titles.title.is_none());
            assert!(titles.subtitle.is_none());
            assert!(titles.action_bar.is_none());
            assert_eq!(
                game.client_session().client_boss_bars(),
                Ok(Some(Vec::new()))
            );

            // Titles and an action bar.
            game.handle_packet(C::SetSubtitleText(play::Text { text: s("sub") }))
                .unwrap();
            // A subtitle alone is not shown: the HUD draws it under a title.
            assert!(
                game.client_session()
                    .client_titles()
                    .unwrap()
                    .unwrap()
                    .subtitle
                    .is_none()
            );
            game.handle_packet(C::SetTitleText(play::Text { text: s("big") }))
                .unwrap();
            game.handle_packet(C::SetActionBarText(play::Text { text: s("bar") }))
                .unwrap();
            let titles = game.client_session().client_titles().unwrap().unwrap();
            assert_eq!(titles.title.as_ref().map(plain).as_deref(), Some("big"));
            assert_eq!(titles.subtitle.as_ref().map(plain).as_deref(), Some("sub"));
            assert_eq!(
                titles.action_bar.as_ref().map(plain).as_deref(),
                Some("bar")
            );
            // The timers run out through the game's own tick: the action bar's 60 ticks
            // end first, the title's 10 + 70 + 20 after them.
            for _ in 0..60 {
                game.client.titles.tick();
            }
            let titles = game.client_session().client_titles().unwrap().unwrap();
            assert!(titles.action_bar.is_none());
            assert!(titles.title.is_some());
            for _ in 0..40 {
                game.client.titles.tick();
            }
            let titles = game.client_session().client_titles().unwrap().unwrap();
            assert!(titles.title.is_none());
            assert!(titles.subtitle.is_none());
            assert!(titles.action_bar.is_none());

            // Boss bars, in the order the server added them, with their progress.
            for (id, name, progress) in [(1u128, "Dragon", 0.5f32), (2u128, "Wither", 0.25f32)] {
                game.handle_packet(C::BossEvent(play::BossEvent {
                    id,
                    action: play::BossAction::Add {
                        name: s(name),
                        progress,
                        color: 5,
                        overlay: 0,
                        darken: false,
                        music: false,
                        fog: false,
                    },
                }))
                .unwrap();
            }
            let bars = game.client_session().client_boss_bars().unwrap().unwrap();
            assert_eq!(bars.len(), 2);
            assert_eq!(plain(&bars[0].name), "Dragon");
            assert_eq!(bars[0].progress, 0.5);
            assert_eq!(plain(&bars[1].name), "Wither");
            assert_eq!(bars[1].progress, 0.25);
        });
    }

    /// Lists this client's own player with `latency`, as the server's player info update does.
    #[cfg(test)]
    fn list_self(remote: &mut Remote, latency: i32) {
        let mut me =
            sunrise_level::profile_lookup::TabPlayer::new(remote.profile, "SessionReadTest");
        me.latency = latency;
        remote.tab_players.insert(remote.profile, me);
        remote
            .tab_game_modes
            .entry(remote.profile)
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicI32::new(0)));
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn latency_and_item_use_read_the_local_player_as_the_client_sees_it() {
        with_client_game(|game| {
            assert_eq!(game.client_session().client_latency(), None);
            assert_eq!(game.local_item_use(), None);
            let _server = logged_client(game);
            assert_eq!(
                game.client_session().client_latency(),
                None,
                "not listed yet"
            );
            list_self(game.client.remote.as_mut().unwrap(), 0);
            assert_eq!(game.client_session().client_latency(), Some(0));
            list_self(game.client.remote.as_mut().unwrap(), 87);
            assert_eq!(game.client_session().client_latency(), Some(87));

            // A bow (use duration 72000) drawn in the main hand for 7 ticks.
            game.using = Some((game.selected, 7));
            game.use_duration = 72_000;
            assert_eq!(
                game.local_item_use(),
                Some(mod_api::ItemUse::new(mod_api::Hand::Main, 7, 71_993))
            );
            game.using = Some((sunrise_gameplay_common::game::held_items::OFFHAND_SLOT, 3));
            assert_eq!(
                game.local_item_use().map(|u| u.hand),
                Some(mod_api::Hand::Off)
            );
            game.using = None;
            assert_eq!(game.local_item_use(), None, "released");

            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(game.client_session().client_latency(), None);
            game.using = Some((game.selected, 7));
            assert_eq!(game.local_item_use(), None, "no world");
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn local_player_status_tracks_mining_mood_and_refuses_other_handles() {
        with_client_game(|game| {
            let _server = logged_client(game);
            let player = PlayerIdentityGameExt1::local_player(game).unwrap();
            assert_eq!(player.block_breaking(game), Some(BlockBreakingState::Idle));
            game.client.ambience.moodiness = 0.375;
            assert_eq!(player.ambient_mood(game), Some(0.375));
            for (progress, stage) in [(0.0, None), (0.45, Some(4)), (1.0, Some(9))] {
                game.set_client_mining(Some(super::super::Mining {
                    pos: [4, 65, -2],
                    progress,
                    per_tick: 0.0,
                    ticks: 3,
                }));
                assert_eq!(
                    player.block_breaking(game),
                    Some(BlockBreakingState::Breaking {
                        position: BlockPos::new(4, 65, -2),
                        progress,
                        stage,
                    })
                );
            }
            game.set_client_mining(None);
            game.client.ambience.moodiness = 0.0;
            assert_eq!(player.block_breaking(game), Some(BlockBreakingState::Idle));
            assert_eq!(player.ambient_mood(game), Some(0.0));
            let fields = mod_api::__private::game::player_ref;
            let epoch = player.entity().epoch();
            for other in [
                fields(mod_api::EntityRef::new(epoch, 7), 43),
                fields(mod_api::EntityRef::new(epoch, 8), 42),
                fields(mod_api::EntityRef::new(epoch, 99), 999),
            ] {
                assert_eq!(other.block_breaking(game), None);
                assert_eq!(other.ambient_mood(game), None);
            }
            game.native_lifecycle.begin_open(None);
            assert_eq!(player.block_breaking(game), None, "the old epoch is stale");
            assert_eq!(player.ambient_mood(game), None);
            let current = PlayerIdentityGameExt1::local_player(game).unwrap();
            assert_eq!(current.ambient_mood(game), Some(0.0));
            game.options.execution_role =
                super::super::execution_role::ExecutionRole::IntegratedServer;
            assert_eq!(current.block_breaking(game), None);
            assert_eq!(current.ambient_mood(game), None);
            assert_eq!(game.client_session().client_server(), None);
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(current.block_breaking(game), None);
            assert_eq!(current.ambient_mood(game), None);
            game.client.remote = None;
            assert_eq!(current.block_breaking(game), None);
        });
    }
}
