use horizon_client::mod_api::{Hooks, Host};

struct User;
impl Hooks for User {
    fn server_about_to_start(&self, _: &mut dyn Host, dedicated: bool) {
        let counter = base_mod::counter();
        #[cfg(feature = "extra_mod")]
        let extra = extra_mod::message();
        #[cfg(not(feature = "extra_mod"))]
        let extra = "off";
        eprintln!("deps-proof user dedicated={dedicated} counter={counter} extra={extra}");
    }
}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(User)
}
