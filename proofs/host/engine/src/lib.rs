//! One small hook for each method of the generated `Hooks` trait.
//!
//! One `MachineHooks` value serves the window's game (client) and the
//! integrated server's game (server). Each hook runs on one side only, so the
//! state each hook uses is named for that side. Hooks take `&self`, so the
//! state uses interior mutability.
use horizon_client::mod_api::{BlockStateId, Flow, Hooks, Host, ItemStack, NativeZoom, Text};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Server ticks between two clock lines in the log (10 seconds).
const CLOCK_INTERVAL: u64 = 200;
/// The tool that reads a machine, and the block it reads.
const PROBE_ITEM: &str = "minecraft:stick";
const MACHINE_BLOCK: &str = "machine_sample:machine";

pub struct MachineHooks {
    /// Server state: ticks seen by each server level.
    server_ticks: Mutex<HashMap<String, u64>>,
    /// Server state: machine reads so far.
    server_probes: AtomicU64,
    /// Client state: the lens zoom, from `MACHINE_SAMPLE_ZOOM` at start.
    client_lens: Option<NativeZoom>,
}

impl MachineHooks {
    /// Read the client-only lens setting from the environment.
    pub fn from_env() -> Self {
        let setting = std::env::var("MACHINE_SAMPLE_ZOOM").ok();
        Self::new(lens_zoom(setting.as_deref()))
    }

    pub fn new(client_lens: Option<NativeZoom>) -> Self {
        Self {
            server_ticks: Mutex::new(HashMap::new()),
            server_probes: AtomicU64::new(0),
            client_lens,
        }
    }
}

/// The lens zoom for a setting such as `2.5`; `None` leaves the view alone.
/// Factors at or below 1 and values that are not finite numbers are ignored.
pub fn lens_zoom(setting: Option<&str>) -> Option<NativeZoom> {
    let factor: f32 = setting?.trim().parse().ok()?;
    (factor.is_finite() && factor > 1.0).then_some(NativeZoom {
        factor,
        smooth: true,
        turn: 1.0 / factor,
    })
}

/// True when a level's tick count is a multiple of the clock interval.
pub fn clock_due(ticks: u64) -> bool {
    ticks.is_multiple_of(CLOCK_INTERVAL)
}

/// The recipe note for an item that the sample machine uses, if any.
pub fn recipe_note(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:cobblestone" => Some("Machine input: becomes stone in 40 ticks"),
        "minecraft:stone" => Some("Machine output: made from cobblestone"),
        _ => None,
    }
}

/// True when the stick is used on a sample machine block.
pub fn is_machine_probe(item: &str, block: &str) -> bool {
    item == PROBE_ITEM && block.contains(MACHINE_BLOCK)
}

impl Hooks for MachineHooks {
    /// Server: a machine clock. Count each level's ticks and log every 200th.
    fn level_tick_pre(&self, _host: &mut dyn Host, dimension: &str) {
        let mut ticks = self.server_ticks.lock().unwrap();
        let count = match ticks.get_mut(dimension) {
            Some(count) => count,
            None => ticks.entry(dimension.to_owned()).or_insert(0),
        };
        *count += 1;
        if clock_due(*count) {
            eprintln!("MACHINE_SAMPLE server clock: {count} ticks in {dimension}");
        }
    }

    /// Client: an inspection lens. Zoom only when the player started the
    /// client with `MACHINE_SAMPLE_ZOOM`; the pinned `Host` has no key state.
    fn zoom(&self, _host: &dyn Host) -> Option<NativeZoom> {
        self.client_lens
    }

    /// Client: add a recipe note to cobblestone and stone tooltips.
    fn tooltip(&self, _host: &dyn Host, stack: &ItemStack, lines: &mut Vec<(Text, u32)>) {
        if let Some(note) = recipe_note(&stack.item) {
            lines.push((Text::literal(format!("[client] {note}")), u32::MAX));
        }
    }

    /// Server: a stick used on a machine reads it. The reply goes to the
    /// acting player's action bar, and the stick does nothing else.
    fn use_item_on(
        &self,
        host: &mut dyn Host,
        stack: &ItemStack,
        pos: [i32; 3],
        _face: [i32; 3],
    ) -> Flow {
        if stack.item.as_ref() == PROBE_ITEM {
            let Some(player) = host.acting_player() else { return Flow::Continue; };
            let before = host.block(pos);
            let changed = host.set_block(&player.dimension, pos, BlockStateId(0), 3);
            let after = host.block(pos);
            let given = host.give_items("minecraft:diamond", 1);
            let message = host.action_bar(player.uuid, Text::literal("Host proof: block removed, diamond given"));
            eprintln!("HOST_PROOF before={before} changed={changed} after={after} diamonds={given} message={message}");
            return Flow::Cancel;
        }
        let block = host.block(pos);
        let machine = is_machine_probe(&stack.item, &block);
        if stack.item.as_ref() == PROBE_ITEM {
            eprintln!(
                "MACHINE_SAMPLE server stick use at {} {} {}: {block} machine={machine}",
                pos[0], pos[1], pos[2]
            );
        }
        if !machine {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let reads = self.server_probes.fetch_add(1, Ordering::Relaxed) + 1;
        eprintln!("MACHINE_SAMPLE server read #{reads}");
        // `tell` does not reach the acting player inside their own hook.
        host.action_bar(
            player.uuid,
            Text::literal(format!(
                "[server] read #{reads} at {} {} {}: {block} (input west, output east)",
                pos[0], pos[1], pos[2]
            )),
        );
        Flow::Cancel
    }
}

pub fn hooks() -> Box<dyn Hooks> { Box::new(MachineHooks::from_env()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_needs_a_factor_above_one() {
        assert_eq!(lens_zoom(None), None);
        assert_eq!(lens_zoom(Some("1")), None);
        assert_eq!(lens_zoom(Some("nan")), None);
        assert_eq!(lens_zoom(Some("abc")), None);
        let lens = lens_zoom(Some(" 4 ")).unwrap();
        assert_eq!((lens.factor, lens.turn), (4.0, 0.25));
    }

    #[test]
    fn clock_runs_every_interval() {
        assert!(!clock_due(199));
        assert!(clock_due(200));
        assert!(clock_due(400));
    }

    #[test]
    fn notes_cover_only_the_recipe_items() {
        assert!(recipe_note("minecraft:cobblestone").is_some());
        assert!(recipe_note("minecraft:stone").is_some());
        assert_eq!(recipe_note("minecraft:dirt"), None);
    }

    #[test]
    fn only_a_stick_on_a_machine_probes() {
        let machine = "machine_sample:machine[facing=north]";
        assert!(is_machine_probe("minecraft:stick", machine));
        assert!(!is_machine_probe("minecraft:stick", "minecraft:stone"));
        assert!(!is_machine_probe("minecraft:dirt", machine));
    }
}
