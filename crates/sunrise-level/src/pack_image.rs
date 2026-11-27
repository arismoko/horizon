//! Pack image size and allocation guards shared between biome colormaps and HUD/atlas decoders.
use crate::Result;

/// The widest or tallest sprite side accepted from a resource pack.
pub const MAX_SPRITE_SIDE: i64 = 32768;

/// The largest atlas side: the 2D texture limit of the GPUs Sunrise runs on
/// (Apple silicon, RADV and Intel allow 16384; NVIDIA 32768).
pub const MAX_ATLAS_SIZE: u32 = 16384;

/// The most bytes any one buffer of a pack image may take (the decoder's, its RGBA copy, its
/// frames made square): 8192x8192 RGBA, 256 MiB. That is far past the panorama faces and
/// animated strips packs ship, where the 16384x16384 pixels [`check_pack_size`] lets through
/// would hold 1 GiB per buffer, two buffers per image and several images loading in parallel.
pub const MAX_IMAGE_BYTES: usize = 256 << 20;

/// A pack image refused for its size: past a side, pixel or byte limit, or a buffer the
/// allocator would not give.
#[derive(Debug)]
pub struct OverSizeLimit(String);

impl std::fmt::Display for OverSizeLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OverSizeLimit {}

/// A pack image may claim no side an atlas would refuse, nor more pixels than
/// the largest atlas holds: the PNG reader alone would trust the header and
/// allocate it, as its own limit covers one row.
pub fn check_pack_size(width: u32, height: u32) -> Result<()> {
    if i64::from(width.max(height)) > MAX_SPRITE_SIDE {
        return Err(OverSizeLimit(format!(
            "a {width}x{height} image exceeds the {MAX_SPRITE_SIDE}-pixel sprite side"
        ))
        .into());
    }
    if u64::from(width) * u64::from(height) > u64::from(MAX_ATLAS_SIZE).pow(2) {
        return Err(OverSizeLimit(format!(
            "a {width}x{height} image has more pixels than a {MAX_ATLAS_SIZE}x{MAX_ATLAS_SIZE} atlas"
        ))
        .into());
    }
    Ok(())
}

/// The bytes of a buffer the product of `factors` long, refused (an overflow too) past
/// `budget`.
pub fn image_bytes(factors: &[usize], budget: usize) -> Result<usize> {
    factors
        .iter()
        .try_fold(1usize, |bytes, &factor| bytes.checked_mul(factor))
        .filter(|&bytes| bytes <= budget)
        .ok_or_else(|| {
            OverSizeLimit(format!(
                "an image buffer of {factors:?} is over the {budget}-byte size limit"
            ))
            .into()
        })
}

/// An empty buffer with room for the product of `factors` values of `T`, refused past `budget`
/// bytes or when the allocator refuses it: a load error rather than an abort.
pub fn image_buffer<T>(factors: &[usize], budget: usize) -> Result<Vec<T>> {
    let bytes = image_bytes(&[factors, &[size_of::<T>()]].concat(), budget)?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(bytes / size_of::<T>())
        .map_err(|e| OverSizeLimit(format!("an image buffer of {bytes} bytes: {e}")))?;
    Ok(buffer)
}
