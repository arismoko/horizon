//! Instantiates the API's generic payload codec (`encode_payload::<T>`, `decode_payload::<T>`) on a
//! type of this mod, through the API's derive macro, so the mods library carries monomorphised API code.
use mod_api::Hooks;

#[derive(Debug, PartialEq, mod_api::PayloadCodec)]
struct Ping {
    count: u32,
    name: String,
    list: Vec<i64>,
}

struct Mod;

impl Default for Mod {
    fn default() -> Self {
        let ping = Ping { count: 7, name: "slice".into(), list: vec![-1, 2, 3] };
        let body = mod_api::encode_payload(&ping, 1024).expect("encodes");
        let back: Ping = mod_api::decode_payload(&body).expect("decodes");
        eprintln!("slice: slice_generic round trip {} bytes, equal {}", body.len(), back == ping);
        Mod
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {}
