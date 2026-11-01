use std::sync::atomic::{AtomicBool, Ordering};
use horizon_client::mod_api::{Flow, Hooks, Host, HudBatch, ItemStack, NativeScreenView};

struct Proof;
static TICK: AtomicBool = AtomicBool::new(false);
static SCREEN: AtomicBool = AtomicBool::new(false);
impl Hooks for Proof {
    fn level_tick_pre(&self, _: &mut dyn Host, dimension: &str) {
        if !TICK.swap(true, Ordering::Relaxed) { eprintln!("SYNC2 level_tick_pre {dimension}"); }
    }
    fn use_item_on(&self, _: &mut dyn Host, _: &ItemStack, pos: [i32; 3], _: [i32; 3]) -> Flow {
        eprintln!("SYNC2 use_item_on canceled {pos:?}");
        Flow::Cancel
    }
    fn screen_render_post(&self, _: &dyn Host, batch: &mut HudBatch, screen: &NativeScreenView<'_>) {
        if !SCREEN.swap(true, Ordering::Relaxed) { eprintln!("SYNC2 screen_render_post {}", screen.id); }
        let uv = batch.vertices.chunks_exact(6).find(|v| v.iter().all(|p| p.uv == v[0].uv)).map(|v| v[0].uv).unwrap_or([0.0, 0.0]);
        batch.quad([[12.0,12.0],[112.0,12.0],[112.0,52.0],[12.0,52.0]], [uv;4], 0xff00ffff);
    }
}
pub fn hooks() -> Box<dyn Hooks> { Box::new(Proof) }
