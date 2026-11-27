use super::*;
use sunrise_core::nbt::Tag;
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

fn load() -> LoadContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}

fn parse(source: &str, items: &Items) -> Ingredient {
    Ingredient::parse(&serde_json::from_str(source).unwrap(), items).unwrap()
}

fn resources() -> Items {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources"),
    );
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    Items::catalog(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap()
}

mod families {
    use super::*;
    mod axolotl_variant;
    mod chicken_variant;
    mod colour;
    mod custom_data;
    mod decoder_4203;
    mod decoder_4206;
    mod decoder_4209;
    mod decoder_4212;
    mod decoder_4217;
    mod decoder_4218;
    mod decoder_4219;
    mod decoder_contracts;
    mod enums;
    mod fuel;
    mod member_4092;
    mod member_4095;
    mod member_4100;
    mod member_4105;
    mod member_4109;
    mod member_4112;
    mod member_4119;
    mod member_4124;
    mod member_4221;
    mod member_4222;
    mod mooshroom_variant;
    mod numeric;
    mod pet_contracts;
    mod pet_variants;
    mod record_contracts;
    mod records;
    mod rest;
    mod salmon_size;
}
