//! ServerPlayer.tick precedes the connection phase, including across levels.
use mod_api::{HostSide, ServerHost};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use sunrise_world::registry::BlockRegistry;
#[cfg(test)]
use {
    crate::game::levels::ServerTickPhase, crate::game::levels::server_tick_phases,
    sunrise_gameplay_common::game::Dim,
};

/// A stub for two levels, each with one connection mining one block. Drive
/// the production dispatcher without a Game or the prepared resource tables.
#[test]
fn every_level_ticks_before_the_connection_phase() {
    let blocks = BlockRegistry::from_facts_json(
        &br#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"]]}"#[..],
    )
    .unwrap();
    let stone = blocks.default_state("minecraft:stone").unwrap();
    let air = blocks.default_state("minecraft:air").unwrap();
    let dims = [Dim::Overworld, Dim::Nether];
    let mut dig = [0; 2];
    let mut heard = Vec::new();
    for phase in server_tick_phases(&dims, &dims) {
        match phase {
            ServerTickPhase::Level(i, dim) => {
                assert_eq!(dim, dims[i]);
                if stone != air {
                    dig[i] += 1;
                    heard.push(("dig", dim, dig[i]));
                }
            }
            ServerTickPhase::IdleLevel(dim) => panic!("unexpected idle level {dim:?}"),
            ServerTickPhase::Connections => {
                for (dim, progress) in dims.into_iter().zip(dig) {
                    heard.push(("pre", dim, progress));
                    heard.push(("post", dim, progress));
                }
            }
        }
    }
    assert_eq!(
        heard,
        [
            ("dig", Dim::Overworld, 1),
            ("dig", Dim::Nether, 1),
            ("pre", Dim::Overworld, 1),
            ("post", Dim::Overworld, 1),
            ("pre", Dim::Nether, 1),
            ("post", Dim::Nether, 1),
        ],
    );
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    EntityPre(u32, Option<i32>),
    EntityPost(u32),
    Dig(u128, i32),
    LevelPost(&'static str),
    PlayerPre(u128, &'static str, Option<i32>, Option<i32>),
    PlayerPost(u128),
    Criterion(u128, String),
}

pub struct Observe {
    pub events: Arc<Mutex<Vec<Event>>>,
    pub cancel: u32,
    pub check_delayed_destroy: bool,
}

pub const DELAYED: [i32; 3] = [9, 63, 9];

impl Observe {
    pub(in crate::game) fn assert_delayed_block_is_air(&self, host: &dyn ServerHost, uuid: u128) {
        if self.check_delayed_destroy && uuid == 123 {
            let level = host.level("minecraft:overworld").unwrap();
            let air = host.block_state_id("minecraft:air").unwrap();
            assert_eq!(
                level.block_state(host, DELAYED),
                Some(air),
                "delayed destroy finished before the player body"
            );
        }
    }
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for Observe {
    fn entity_tick_pre(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        let HostSide::Server(host) = host else {
            return mod_api::Flow::Continue;
        };
        let id = event.entity;
        let immunity = host
            .entity(id)
            .and_then(|entity| entity.invulnerable_time(host));
        self.events
            .lock()
            .unwrap()
            .push(Event::EntityPre(id, immunity));
        if id == self.cancel {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
    fn entity_tick_post(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        self.events
            .lock()
            .unwrap()
            .push(Event::EntityPost(event.entity));
    }
    fn block_break_progress_update(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::BlockBreakProgressUpdateEvent<'_>,
    ) {
        self.events
            .lock()
            .unwrap()
            .push(Event::Dig(event.player, event.progress));
    }
    fn player_advancement_progress(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerAdvancementProgressEvent<'_>,
    ) {
        if matches!(event.advancement, "test:phase" | "test:staged") {
            self.events
                .lock()
                .unwrap()
                .push(Event::Criterion(event.player, event.criterion.into()));
        }
    }
    fn level_tick_post(&self, host: HostSide<'_>, _: &mut mod_api::LevelTickPostEvent) {
        if let HostSide::Server(host) = host {
            self.events
                .lock()
                .unwrap()
                .push(Event::LevelPost(host.dimension()));
        }
    }
    fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
        let HostSide::Server(host) = host else { return };
        self.assert_delayed_block_is_air(host, event.player.uuid());
        let entity = event.player.entity();
        self.events.lock().unwrap().push(Event::PlayerPre(
            event.player.uuid(),
            host.dimension(),
            entity.invulnerable_time(host),
            entity.damage_cooldown_time(host),
        ));
    }
    fn player_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPostEvent) {
        let HostSide::Server(host) = host else { return };
        self.assert_delayed_block_is_air(host, event.player.uuid());
        self.events
            .lock()
            .unwrap()
            .push(Event::PlayerPost(event.player.uuid()));
    }
}
