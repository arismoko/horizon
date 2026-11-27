//! Custom boss bar handles for native server mods: the `/bossbar` store.
use crate::game::ServerGame;
use mod_api::__private::game::*;
use mod_api::{BossBarColor, BossBarOverlay, BossBarRef, PlayerRef};
use sunrise_core::identifier::Identifier;
use sunrise_game_data::{bossbar::BossBar, scoreboard::comp_text, text::Text};
use sunrise_sim::lan::playerdata::{ints_uuid, uuid_ints};

impl ServerGame<'_> {
    pub fn boss_bar_state(&self, bar: BossBarRef) -> Option<&BossBar> {
        if !self.common.is_logical_server()
            || self.common.native_lifecycle.epoch() != Some(bar.fields().epoch)
        {
            return None;
        }
        self.server
            .bosses
            .bars
            .bars
            .iter()
            .find(|b| b.uuid == bar.fields().uuid)
    }

    fn boss_bar_edit(&mut self, bar: BossBarRef, edit: impl FnOnce(&mut BossBar)) -> bool {
        if self.boss_bar_state(bar).is_none() {
            return false;
        }
        if let Some(state) = self
            .server
            .bosses
            .bars
            .bars
            .iter_mut()
            .find(|b| b.uuid == bar.fields().uuid)
        {
            edit(state);
        }
        self.bossbars_changed();
        true
    }

    fn boss_bar_handle(&self, bar: &BossBar) -> Option<BossBarRef> {
        Some(mod_api::__private::game::boss_bar_ref(
            self.common.native_lifecycle.epoch()?,
            bar.uuid,
        ))
    }

    /// An existing custom boss bar by id (`minecraft:` when no namespace), or
    /// None. Saved bars are there from the world's start, as vanilla's
    /// `CustomBossEvents`. Boss bars of mobs and raids are not custom boss
    /// events.
    pub fn boss_bar(&self, id: &str) -> Option<BossBarRef> {
        let id = Identifier::parse(id).ok()?;
        let id = format!("{}:{}", id.namespace(), id.path());
        self.boss_bar_handle(self.boss_bar_state_by_id(&id)?)
    }

    fn boss_bar_state_by_id(&self, id: &str) -> Option<&BossBar> {
        if !self.common.is_logical_server() {
            return None;
        }
        self.server.bosses.bars.get(id)
    }

    /// Every custom boss bar, in the order `/bossbar list` shows them.
    pub fn boss_bars(&self) -> Vec<BossBarRef> {
        if !self.common.is_logical_server() {
            return Vec::new();
        }
        self.server
            .bosses
            .listed_bars()
            .into_iter()
            .filter_map(|bar| self.boss_bar_handle(bar))
            .collect()
    }

    /// `/bossbar add`: a new bar, visible, white, `progress`, at 0 of 100, with
    /// nobody added. None if the id is taken or invalid, or outside a server.
    pub(in crate::game) fn create_boss_bar(&mut self, id: &str, name: Text) -> Option<BossBarRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let id = Identifier::parse(id).ok()?;
        let id = format!("{}:{}", id.namespace(), id.path());
        if !self.common.is_logical_server() {
            return None;
        }
        let uuid = self.add_custom_bossbar(&id, name.component())?;
        self.bossbars_changed();
        Some(mod_api::__private::game::boss_bar_ref(epoch, uuid))
    }

    /// The bar's id (`CustomBossEvent.customId`), or None for a stale handle.
    pub(in crate::game) fn boss_bar_id(&self, bar: BossBarRef) -> Option<String> {
        self.boss_bar_state(bar).map(|b| b.id.clone())
    }

    /// `BossEvent.getName`.
    pub(in crate::game) fn boss_bar_name(&self, bar: BossBarRef) -> Option<Text> {
        self.boss_bar_state(bar).map(|b| comp_text(&b.name))
    }

    /// `ServerBossEvent.setName`: viewers are sent the new name.
    pub(in crate::game) fn boss_bar_set_name(&mut self, bar: BossBarRef, name: Text) -> bool {
        self.boss_bar_edit(bar, |b| b.name = name.component())
    }

    /// `BossEvent.getColor`.
    pub(in crate::game) fn boss_bar_color(&self, bar: BossBarRef) -> Option<BossBarColor> {
        let b = self.boss_bar_state(bar)?;
        Some(BossBarColor::from_name(&b.color).unwrap_or(BossBarColor::White))
    }

    /// `ServerBossEvent.setColor`.
    pub(in crate::game) fn boss_bar_set_color(
        &mut self,
        bar: BossBarRef,
        color: BossBarColor,
    ) -> bool {
        self.boss_bar_edit(bar, |b| b.color = color.name().to_owned())
    }

    /// `BossEvent.getOverlay`.
    pub(in crate::game) fn boss_bar_overlay(&self, bar: BossBarRef) -> Option<BossBarOverlay> {
        let b = self.boss_bar_state(bar)?;
        Some(BossBarOverlay::from_name(&b.style).unwrap_or(BossBarOverlay::Progress))
    }

    /// `ServerBossEvent.setOverlay`.
    pub(in crate::game) fn boss_bar_set_overlay(
        &mut self,
        bar: BossBarRef,
        overlay: BossBarOverlay,
    ) -> bool {
        self.boss_bar_edit(bar, |b| b.style = overlay.name().to_owned())
    }

    /// `BossEvent.shouldDarkenScreen`.
    pub(in crate::game) fn boss_bar_darken_screen(&self, bar: BossBarRef) -> Option<bool> {
        self.boss_bar_state(bar).map(|b| b.darken)
    }

    /// `ServerBossEvent.setDarkenScreen`.
    pub(in crate::game) fn boss_bar_set_darken_screen(
        &mut self,
        bar: BossBarRef,
        darken: bool,
    ) -> bool {
        self.boss_bar_edit(bar, |b| b.darken = darken)
    }

    /// `BossEvent.shouldPlayBossMusic`.
    pub(in crate::game) fn boss_bar_play_boss_music(&self, bar: BossBarRef) -> Option<bool> {
        self.boss_bar_state(bar).map(|b| b.music)
    }

    /// `ServerBossEvent.setPlayBossMusic`.
    pub(in crate::game) fn boss_bar_set_play_boss_music(
        &mut self,
        bar: BossBarRef,
        music: bool,
    ) -> bool {
        self.boss_bar_edit(bar, |b| b.music = music)
    }

    /// `BossEvent.shouldCreateWorldFog`.
    pub(in crate::game) fn boss_bar_create_world_fog(&self, bar: BossBarRef) -> Option<bool> {
        self.boss_bar_state(bar).map(|b| b.fog)
    }

    /// `ServerBossEvent.setCreateWorldFog`.
    pub(in crate::game) fn boss_bar_set_create_world_fog(
        &mut self,
        bar: BossBarRef,
        fog: bool,
    ) -> bool {
        self.boss_bar_edit(bar, |b| b.fog = fog)
    }

    /// `CustomBossEvent.value`.
    pub(in crate::game) fn boss_bar_value(&self, bar: BossBarRef) -> Option<i32> {
        self.boss_bar_state(bar).map(|b| b.value)
    }

    /// `CustomBossEvent.setValue`: stored as given, as vanilla does; only the
    /// progress shown is kept between empty and full.
    pub(in crate::game) fn boss_bar_set_value(&mut self, bar: BossBarRef, value: i32) -> bool {
        let Some(id) = self.boss_bar_id(bar) else {
            return false;
        };
        self.set_bossbar_value(&id, true, value);
        true
    }

    /// `CustomBossEvent.max`.
    pub(in crate::game) fn boss_bar_max(&self, bar: BossBarRef) -> Option<i32> {
        self.boss_bar_state(bar).map(|b| b.max)
    }

    /// `CustomBossEvent.setMax`: stored as given, and the value is kept. Only
    /// `/bossbar set max` limits it to 1 or more.
    pub(in crate::game) fn boss_bar_set_max(&mut self, bar: BossBarRef, max: i32) -> bool {
        let Some(id) = self.boss_bar_id(bar) else {
            return false;
        };
        self.set_bossbar_value(&id, false, max);
        true
    }

    /// `ServerBossEvent.isVisible`.
    pub(in crate::game) fn boss_bar_visible(&self, bar: BossBarRef) -> Option<bool> {
        self.boss_bar_state(bar).map(|b| b.visible)
    }

    /// `ServerBossEvent.setVisible`: a hidden bar is taken off its players'
    /// screens and keeps them.
    pub(in crate::game) fn boss_bar_set_visible(&mut self, bar: BossBarRef, visible: bool) -> bool {
        self.boss_bar_edit(bar, |b| b.visible = visible)
    }

    /// `ServerBossEvent.getPlayers`: the bar's players who are online.
    pub(in crate::game) fn boss_bar_players(&self, bar: BossBarRef) -> Option<Vec<PlayerRef>> {
        let b = self.boss_bar_state(bar)?;
        Some(
            b.players
                .iter()
                .filter_map(|ints| self.player(ints_uuid(*ints)))
                .collect(),
        )
    }

    /// `CustomBossEvent.addPlayer`. False for a stale bar or an offline or
    /// stale player. The bar is kept for the player across sessions.
    pub(in crate::game) fn boss_bar_add_player(
        &mut self,
        bar: BossBarRef,
        player: PlayerRef,
    ) -> bool {
        if self.player(player.uuid()) != Some(player) {
            return false;
        }
        let ints = uuid_ints(player.uuid());
        self.boss_bar_edit(bar, |b| {
            if !b.players.contains(&ints) {
                b.players.push(ints);
            }
        })
    }

    /// `CustomBossEvent.removePlayer`. False for a stale bar or an offline or
    /// stale player.
    pub(in crate::game) fn boss_bar_remove_player(
        &mut self,
        bar: BossBarRef,
        player: PlayerRef,
    ) -> bool {
        if self.player(player.uuid()) != Some(player) {
            return false;
        }
        let ints = uuid_ints(player.uuid());
        self.boss_bar_edit(bar, |b| b.players.retain(|p| *p != ints))
    }

    /// `/bossbar remove` (`CustomBossEvents.remove`): the bar's viewers lose it
    /// and its handle goes stale.
    pub(in crate::game) fn boss_bar_remove(&mut self, bar: BossBarRef) -> bool {
        let Some(id) = self.boss_bar_id(bar) else {
            return false;
        };
        self.server.bosses.remove_bar(&id);
        self.bossbars_changed();
        true
    }
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
