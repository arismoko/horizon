//! Shared listener and resource fixture for independent projectile impact tests.
use mod_api::{Flow, Hooks, HostTransaction, ServerHost, TransactionError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// The listener action exercised by a tick.
#[derive(Clone, Copy)]
pub(crate) enum Action {
    Allow,
    SaveOnImpact,
    ReadSectionPosition,
    #[cfg(feature = "client")]
    ClearBlockOnImpact,
    ClearBlockOnShoot([i32; 3]),
    Cancel,
    CancelTarget(u32),
    Discard,
    DiscardAndCancel,
    AbortDiscard,
    DiscardTarget(u32),
    Remove,
}

/// The independently observable event payload.
#[derive(Debug)]
pub(crate) struct Seen {
    pub projectile: u32,
    pub kind: String,
    pub owner: u32,
    pub dimension: String,
    pub entity: Option<u32>,
    pub entity_kind: String,
    pub block: [i32; 3],
    pub face: [i32; 3],
    pub at: [f64; 3],
    pub projectile_pos: [f64; 3],
}

pub(crate) type SectionCrossing = (u32, [i32; 3], [i32; 3]);

/// Listener observations, including any incorrectly triggered catch.
#[derive(Default)]
pub(crate) struct Observed {
    pub hits: Mutex<Vec<Seen>>,
    pub catches: AtomicUsize,
    pub lands: AtomicUsize,
    pub bursts: AtomicUsize,
    pub game_events: Mutex<Vec<(&'static str, [f64; 3])>>,
    pub sections: Mutex<Vec<SectionCrossing>>,
    pub section_positions: Mutex<Vec<Option<[f64; 3]>>>,
    pub deaths: AtomicUsize,
    pub leaves: Mutex<Vec<u32>>,
}

struct Probe {
    action: Action,
    observed: Arc<Observed>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn projectile_impact(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ProjectileImpactEvent<'_>,
    ) -> Flow {
        let projectile = host
            .entity(event.projectile)
            .expect("impact projectile resolves");
        self.observed.hits.lock().unwrap().push(Seen {
            projectile: event.projectile,
            kind: event.kind.to_owned(),
            owner: event.owner,
            dimension: event.dimension.to_owned(),
            entity: event.entity,
            entity_kind: event.entity_kind.to_owned(),
            block: event.block,
            face: event.face,
            at: event.at,
            projectile_pos: projectile.position(host).unwrap(),
        });
        match self.action {
            Action::Allow | Action::ReadSectionPosition | Action::ClearBlockOnShoot(_) => {
                Flow::Continue
            }
            Action::SaveOnImpact => {
                assert!(host.commands().run_command("save-all flush"));
                Flow::Cancel
            }
            #[cfg(feature = "client")]
            Action::ClearBlockOnImpact => {
                let level = host.level(event.dimension).unwrap();
                assert!(level.remove_block(host, event.block, false));
                Flow::Continue
            }
            Action::Cancel => Flow::Cancel,
            Action::CancelTarget(id) if event.entity == Some(id) => Flow::Cancel,
            Action::CancelTarget(_) => Flow::Continue,
            Action::DiscardTarget(id) => {
                if event.entity == Some(id) {
                    assert!(host.entity(id).unwrap().discard(host));
                }
                Flow::Continue
            }
            Action::Discard | Action::DiscardAndCancel | Action::AbortDiscard | Action::Remove => {
                let remove = |host: &mut dyn ServerHost| {
                    let removed = match self.action {
                        Action::Remove => projectile.remove(host, mod_api::RemovalReason::Killed),
                        _ => projectile.discard(host),
                    };
                    assert!(removed);
                    assert!(host.entity(event.projectile).is_none());
                    assert!(!projectile.discard(host));
                };
                if matches!(self.action, Action::AbortDiscard) {
                    let aborted = host.transaction::<(), ()>(|host, _| {
                        remove(host);
                        Err(())
                    });
                    assert_eq!(aborted, Err(TransactionError::Aborted(())));
                    assert_eq!(host.entity(event.projectile), Some(projectile));
                } else {
                    remove(host);
                }
                match self.action {
                    Action::DiscardAndCancel | Action::AbortDiscard => Flow::Cancel,
                    _ => Flow::Continue,
                }
            }
        }
    }

    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        match event.event {
            "minecraft:projectile_land" => {
                self.observed.lands.fetch_add(1, Ordering::Relaxed);
                self.observed
                    .game_events
                    .lock()
                    .unwrap()
                    .push(("land", event.pos));
            }
            "minecraft:entity_die" => {
                self.observed.deaths.fetch_add(1, Ordering::Relaxed);
            }
            "minecraft:explode" => {
                self.observed.bursts.fetch_add(1, Ordering::Relaxed);
            }
            "minecraft:projectile_shoot" => {
                self.observed
                    .game_events
                    .lock()
                    .unwrap()
                    .push(("shoot", event.pos));
                if let Action::ClearBlockOnShoot(block) = self.action {
                    assert!(!self.observed.hits.lock().unwrap().is_empty());
                    let level = host.level(event.dimension).unwrap();
                    assert!(level.remove_block(host, block, false));
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn entity_entering_section(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityEnteringSectionEvent,
    ) {
        if matches!(self.action, Action::ReadSectionPosition) && event.entity.id() == 71 {
            let position = match host {
                mod_api::HostSide::Server(host) => event.entity.position(host),
                mod_api::HostSide::Client(host) => event.entity.position(host),
            };
            self.observed
                .section_positions
                .lock()
                .unwrap()
                .push(position);
        }
        self.observed.sections.lock().unwrap().push((
            event.entity.id(),
            event.old_section,
            event.new_section,
        ));
    }

    fn entity_leave_level(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::EntityLeaveLevelEvent<'_>,
    ) {
        self.observed.leaves.lock().unwrap().push(event.entity);
    }

    fn item_fished(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::ItemFishedEvent<'_, '_>,
    ) -> Flow {
        self.observed.catches.fetch_add(1, Ordering::Relaxed);
        Flow::Continue
    }
}

struct UnsubscribedProbe(Probe);

#[mod_api::__subscriptions]
impl Hooks for UnsubscribedProbe {
    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        self.0.vanilla_game_event(host, event)
    }

    fn entity_entering_section(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::EntityEnteringSectionEvent,
    ) {
        self.0.entity_entering_section(host, event);
    }

    fn entity_leave_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EntityLeaveLevelEvent<'_>,
    ) {
        self.0.entity_leave_level(host, event);
    }

    fn item_fished(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ItemFishedEvent<'_, '_>,
    ) -> Flow {
        self.0.item_fished(host, event)
    }
}

/// Run a real server tick fixture with a live player and listener installed.
pub(crate) fn with_probe(
    action: Action,
    subscribed: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Observed),
) {
    let observed = Arc::new(Observed::default());
    let probe = Probe {
        action,
        observed: observed.clone(),
    };
    let entry = if subscribed {
        mod_api::ModEntry::new("impact-probe", Box::new(probe))
    } else {
        mod_api::ModEntry::new("impact-probe", Box::new(UnsubscribedProbe(probe)))
    };
    let mods = vec![entry].into();
    mod_platform::testing::with_mods(mods, || {
        let check = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 8.5)).unwrap());
            bind_local_player(game, 7, 7);
            game.selected = 0;
            game.inventory[0] = Some(ItemStack::new("minecraft:fishing_rod", 1));
            game.server.next_entity = 100;
            test(game, &observed);
        };
        if matches!(action, Action::SaveOnImpact) {
            let save = tempfile::tempdir().unwrap();
            crate::game::server_side::visibility_unwind_tests::with_save_game(save.path(), check);
        } else {
            with_game(check);
        }
    });
}

/// Compare a geometric point against an independently specified expectation.
pub(crate) fn point(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() < 1.0e-6,
            "{actual:?} != {expected:?}"
        );
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
