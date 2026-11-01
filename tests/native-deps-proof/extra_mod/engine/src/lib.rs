use horizon_client::mod_api::Hooks;

pub fn message() -> &'static str {
    "on"
}

struct Extra;
impl Hooks for Extra {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Extra)
}
