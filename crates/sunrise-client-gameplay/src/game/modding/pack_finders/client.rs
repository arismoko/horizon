use mod_api::PackType;
use std::sync::Arc;
use sunrise_client_render::{
    pack_screen::load_icon,
    packs::{Pack, pack_from_meta},
};
use sunrise_gameplay_common::game::{
    mods::ModsState,
    pack_discovery::{DiscoveredPack, data_pack, discover},
};
use sunrise_level::{
    datapack::DataPack,
    pack_meta::{PackInfo, PackMeta},
};

pub fn into_resource_pack(mut pack: DiscoveredPack) -> Pack {
    fill_icon(&mut pack.meta);
    pack_from_meta(pack.meta, &pack.metadata)
}

pub(in crate::game) fn selection_with_icon(pack: &PackMeta) -> Option<PackInfo> {
    let mut selection = pack.selection.clone()?;
    selection.icon = pack.icon_png().as_deref().and_then(load_icon);
    Some(selection)
}

pub(in crate::game) fn data_packs_with_icons(mods: &ModsState) -> Vec<Arc<DataPack>> {
    discover(PackType::ServerData, mods)
        .into_iter()
        .map(|mut pack| {
            fill_icon(&mut pack.meta);
            data_pack(pack.meta)
        })
        .collect()
}

fn fill_icon(pack: &mut PackMeta) {
    if let Some(mut selection) = pack.selection.take() {
        selection.icon = pack.icon_png().as_deref().and_then(load_icon);
        pack.selection = Some(selection);
    }
}
