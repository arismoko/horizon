//! What a mod's presence costs per call: a hook fired with no listener, or with the one native
//! listener `sunrise-hot-paths-game` installs, and an attachment read and written.
use super::Recorder;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::residency_tick::ResidencyTickGameExt as _;

/// The attachment the attachment workloads read and write: plain data, not saved or synced.
static COUNTER: sunrise_mod_runtime::attachments::AttachmentType<u32> =
    sunrise_mod_runtime::attachments::AttachmentType {
        namespace: "hot_paths",
        name: "counter",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };

const CALLS: u64 = 100_000;

impl crate::game::ServerGame<'_> {
    pub(super) fn hot_mods(&mut self, recorder: &mut Recorder, listening: bool) -> Result<()> {
        let hook = if listening {
            "mod.hook.native_listener"
        } else {
            "mod.hook.no_listener"
        };
        if recorder.wants(hook) {
            self.hot_level(5, 2)?;
            self.hot_mobs(128, 2)?;
            let dimension = self.common.dim.id().to_owned();
            let entities: Vec<u32> = self.common.mobs.iter().map(|mob| mob.id).collect();
            recorder.measure(hook, CALLS, || {
                let mut count = 0u64;
                for call in 0..CALLS as usize {
                    let mut canceled = false;
                    self.native_entity_tick_pre(
                        &dimension,
                        entities[call % entities.len()],
                        "minecraft:cow",
                        &mut canceled,
                    );
                    count += u64::from(canceled);
                }
                count
            })?;
        }
        if listening {
            return Ok(());
        }
        if recorder.wants("mod.attachment") {
            self.hot_level(6, 2)?;
            self.hot_mobs(128, 2)?;
            let holders: Vec<_> = self
                .common
                .mobs
                .iter()
                .map(|mob| mod_api::Holder::Entity(mob.id))
                .collect();
            // A mod reads and writes attachments from a hook, inside the server's tick.
            self.with_residency_tick(|game| {
                let mut failed: std::result::Result<(), String> = Ok(());
                for holder in &holders {
                    COUNTER
                        .set(game, *holder, 1)
                        .map_err(|error| format!("mod.attachment: first write: {error:?}"))?;
                }
                const ROUNDS: u64 = 100;
                let operations = ROUNDS * holders.len() as u64;
                recorder.measure("mod.attachment.read", operations, || {
                    let mut sum = 0u64;
                    for _ in 0..ROUNDS {
                        for holder in &holders {
                            match COUNTER.get(game, *holder) {
                                Ok(value) => sum += u64::from(value),
                                Err(error) => failed = Err(format!("{error:?}")),
                            }
                        }
                    }
                    sum
                })?;
                recorder.measure("mod.attachment.write", operations, || {
                    for round in 0..ROUNDS {
                        for holder in &holders {
                            if let Err(error) = COUNTER.set(game, *holder, round as u32) {
                                failed = Err(format!("{error:?}"));
                            }
                        }
                    }
                    holders
                        .iter()
                        .map(|holder| COUNTER.get(game, *holder).map_or(u64::MAX, u64::from))
                        .sum::<u64>()
                })?;
                failed?;
                Ok(())
            })?;
        }
        Ok(())
    }
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;
