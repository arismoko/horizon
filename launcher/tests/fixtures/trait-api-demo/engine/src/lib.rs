use horizon_client::mod_api::{Flow, Hooks, Host, ItemStack, NativeZoom, Text};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Proof {
    ticked: AtomicBool,
    tooltip_seen: AtomicBool,
    zoom_seen: AtomicBool,
}
impl Default for Proof {
    fn default() -> Self {
        Self {
            ticked: AtomicBool::new(false),
            tooltip_seen: AtomicBool::new(false),
            zoom_seen: AtomicBool::new(false),
        }
    }
}
impl Hooks for Proof {
    fn level_tick_pre(&self, _: &mut dyn Host, dimension: &str) {
        if !self.ticked.swap(true, Ordering::Relaxed) {
            eprintln!("TRAIT_PROOF tick {dimension}");
        }
    }
    fn use_item_on(&self, host: &mut dyn Host, _: &ItemStack, _: [i32; 3], _: [i32; 3]) -> Flow {
        eprintln!("TRAIT_PROOF use cancelled");
        host.feedback(Text::literal("Trait proof cancelled use"));
        Flow::Cancel
    }
    fn tooltip(&self, _: &dyn Host, _: &ItemStack, lines: &mut Vec<(Text, u32)>) {
        if !self.tooltip_seen.swap(true, Ordering::Relaxed) {
            eprintln!("TRAIT_PROOF tooltip");
        }
        lines.push((Text::literal("Trait proof tooltip"), u32::MAX));
    }
    fn zoom(&self, _: &dyn Host) -> Option<NativeZoom> {
        if std::env::var_os("PROOF_NO_ZOOM").is_some() {
            return None;
        }
        if !self.zoom_seen.swap(true, Ordering::Relaxed) {
            eprintln!("TRAIT_PROOF zoom factor 2");
        }
        Some(NativeZoom {
            factor: 2.0,
            smooth: false,
            turn: 1.0,
        })
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Proof::default())
}
