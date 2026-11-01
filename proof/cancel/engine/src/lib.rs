use horizon_client::mod_api::{Flow, Hooks};
use std::sync::Once;

struct Proof;
impl Hooks for Proof {
    fn inject_cancel_proof_server_head(&self) -> Flow {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| eprintln!("INJECT_PROOF cancel HEAD fired"));
        Flow::Cancel
    }
}
pub fn hooks() -> Box<dyn Hooks> { Box::new(Proof) }
