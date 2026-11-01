use horizon_client::mod_api::{Hooks, Host, NativeLifecycleContext, ServerTickPhase};
use std::sync::Once;

struct Proof;
impl Hooks for Proof {
    fn server_tick(&self, _host: &mut dyn Host, _context: &NativeLifecycleContext, _phase: ServerTickPhase) {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| eprintln!("INJECT_PROOF catalog server tick fired"));
    }
}
pub fn hooks() -> Box<dyn Hooks> { Box::new(Proof) }
