use horizon_client::mod_api::{Hooks, Host};
use std::sync::atomic::{AtomicUsize, Ordering};

pub static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn counter() -> usize {
    COUNTER.load(Ordering::SeqCst)
}

struct Base;
impl Hooks for Base {
    fn server_about_to_start(&self, _: &mut dyn Host, dedicated: bool) {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
        eprintln!("deps-proof base dedicated={dedicated} counter={count}");
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Base)
}
