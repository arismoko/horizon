use horizon_client::mod_api::Hooks;
use std::sync::Once;

struct Proof;
impl Hooks for Proof {
    fn inject_inject_proof_server_head(&self) {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| eprintln!("INJECT_PROOF trait HEAD fired"));
    }
}
pub fn hooks() -> Box<dyn Hooks> { Box::new(Proof) }
