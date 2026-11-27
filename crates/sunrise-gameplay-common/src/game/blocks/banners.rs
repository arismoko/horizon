//! Placed banners (`BannerBlockEntity`): the pattern layers move from the
//! banner item to the block when placed and back to the dropped item when
//! broken (`copy_components` in the banner loot tables).
use super::maps::MapBanner;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::banner::{self, BannerLayer};

/// `MapBanner.fromWorld`: the banner whose block entity is `entity` at `pos`,
/// with the base colour of its block (`BannerBlockEntity.getBaseColor` is
/// `AbstractBannerBlock.getColor`) and its custom name; none when the block
/// entity is not a banner.
pub fn map_banner(pos: [i32; 3], entity: Option<&BlockEntity>, block: &str) -> Option<MapBanner> {
    let Some(BlockEntity::Banner(banner)) = entity else {
        return None;
    };
    let block = block.strip_prefix("minecraft:").unwrap_or(block);
    let color = block
        .strip_suffix("_wall_banner")
        .or_else(|| block.strip_suffix("_banner"))?;
    Some(MapBanner {
        pos,
        color: color.to_owned(),
        name: banner.name.clone(),
    })
}

/// Texture names and tints of banner layers (`Sheets.getBannerSprite`).
pub fn layer_textures(banners: &banner::Banners, layers: &[BannerLayer]) -> Vec<(String, u32)> {
    layers
        .iter()
        .map(|l| {
            (
                banners.banner_texture(&l.pattern),
                banner::dye_rgb(&l.color),
            )
        })
        .collect()
}
