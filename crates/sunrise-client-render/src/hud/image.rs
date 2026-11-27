use super::*;

pub fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    decode_bytes(&sunrise_core::prepared_files::read(path)?)
}
/// The image formats [`decode_image_bytes`] reads, told apart by their own
/// leading bytes rather than by a file name: what a server sends has no name,
/// and a name would not be trustworthy anyway.
///
/// The game's own resources are PNG throughout, and so are a resource pack's
/// (vanilla's `NativeImage.read` takes PNG only), so [`decode_bytes`] stays
/// PNG-only. WebP and JPEG are for images fetched from the network (Modrinth
/// project icons in the resource pack browser), which are commonly either.
pub(super) enum ImageFormat {
    Png,
    /// `RIFF....WEBP`.
    WebP,
    /// Starts with the SOI marker `FF D8`.
    Jpeg,
}

impl ImageFormat {
    /// The format `data` begins with, or `None` for anything else.
    pub(super) fn sniff(data: &[u8]) -> Option<Self> {
        const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        if data.starts_with(&PNG_MAGIC) {
            return Some(Self::Png);
        }
        if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
            return Some(Self::WebP);
        }
        if data.starts_with(&[0xFF, 0xD8]) {
            return Some(Self::Jpeg);
        }
        None
    }
}

/// The most pixels a fetched image may claim: its header alone sizes the
/// buffer, and a hostile one could ask for gigabytes. 4096 x 4096 RGBA is 64 MiB.
const MAX_FETCHED_PIXELS: u64 = 4096 * 4096;

pub(super) fn check_fetched_size(width: u64, height: u64) -> Result<()> {
    if width * height > MAX_FETCHED_PIXELS {
        return Err(format!("a {width}x{height} image is too large to decode").into());
    }
    Ok(())
}

/// A WebP image as RGBA8. Still frames and animations both decode; an
/// animation gives its first frame, as an animated PNG would.
fn decode_webp(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder =
        image_webp::WebPDecoder::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    check_fetched_size(u64::from(width), u64::from(height))?;
    // Four bytes a pixel whether or not the image carries alpha: the caller
    // always wants RGBA.
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    if decoder.has_alpha() {
        decoder.read_image(&mut rgba).map_err(|e| e.to_string())?;
    } else {
        let mut rgb = vec![0u8; (width as usize) * (height as usize) * 3];
        decoder.read_image(&mut rgb).map_err(|e| e.to_string())?;
        for (out, pixel) in rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(rgb.as_chunks::<3>().0)
        {
            out.copy_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    Ok((width, height, rgba))
}

/// A JPEG image as RGBA8. JPEG has no alpha, so every pixel is opaque.
fn decode_jpeg(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = zune_jpeg::JpegDecoder::new(data);
    // The size comes from the headers, before any pixel buffer is made.
    decoder.decode_headers().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("the JPEG states no size")?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    check_fetched_size(u64::from(width), u64::from(height))?;
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let count = (width as usize) * (height as usize);
    // The decoder gives RGB for a colour image and one byte a pixel for a
    // greyscale one; both become RGBA here.
    let channels = pixels.len().checked_div(count).unwrap_or(0);
    let mut rgba = Vec::with_capacity(count * 4);
    match channels {
        3 => {
            for p in pixels.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        1 => {
            for p in &pixels {
                rgba.extend_from_slice(&[*p, *p, *p, 255]);
            }
        }
        _ => return Err("the JPEG decoded to an unexpected channel count".into()),
    }
    Ok((width, height, rgba))
}

/// An image fetched from the network (PNG, WebP or JPEG) as RGBA pixels. WebP
/// and JPEG go to their own decoders; anything else goes to the PNG reader, so
/// its error is the one reported.
pub fn decode_image_bytes(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    match ImageFormat::sniff(data) {
        Some(ImageFormat::WebP) => decode_webp(data),
        Some(ImageFormat::Jpeg) => decode_jpeg(data),
        // A PNG's IHDR states the size before the decoder sizes its pixel
        // buffer, so it is checked there like the WebP and JPEG headers: the
        // PNG reader alone would trust the header and allocate it.
        Some(ImageFormat::Png) | None => {
            if let Some((width, height)) = png_bytes_size(data) {
                check_fetched_size(u64::from(width), u64::from(height))?;
            }
            decode_bytes(data)
        }
    }
}

/// A PNG in memory as RGBA pixels.
pub fn decode_bytes(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    decode_bytes_within(data, MAX_IMAGE_BYTES)
}

/// [`decode_bytes`], each of its buffers held to `budget` bytes.
pub(super) fn decode_bytes_within(data: &[u8], budget: usize) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    check_pack_size(reader.info().width, reader.info().height)?;
    let size = reader.output_buffer_size().unwrap_or(usize::MAX);
    let mut bytes = image_buffer(&[size], budget)?;
    bytes.resize(size, 0);
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    let mut rgba = image_buffer(&[info.width as usize, info.height as usize, 4], budget)?;
    for p in bytes[..info.buffer_size()].chunks_exact(channels) {
        rgba.extend(match channels {
            4 => [p[0], p[1], p[2], p[3]],
            3 => [p[0], p[1], p[2], 255],
            2 => [p[0], p[0], p[0], p[1]],
            _ => [p[0], p[0], p[0], 255],
        });
    }
    Ok((info.width, info.height, rgba))
}
/// The most texels of OptiFine random entity variants the entity atlas packs: 2048 rows of
/// its 4096, which keeps the default atlas (4096x5120) under the 8192 rows many GPUs take.
pub(super) const RANDOM_TEXELS: u64 = 4096 * 2048;
/// The variant images (and `overlays`) the atlas packs: every variant before any overlay, by
/// name, while their sizes add up to at most `budget` texels. A full budget drops glows before
/// looks.
pub(super) fn budgeted<'a>(
    source: &ResourceSource,
    files: impl Iterator<Item = &'a String>,
    overlays: &HashSet<String>,
    budget: u64,
) -> HashSet<String> {
    let mut files: Vec<&String> = files.collect();
    files.sort_by_key(|p| (overlays.contains(*p), *p));
    within_budget(source, &files, budget)
}
/// The `files` (in order) whose sizes add up to at most `budget` texels.
pub(super) fn within_budget(
    source: &ResourceSource,
    files: &[&String],
    budget: u64,
) -> HashSet<String> {
    let mut texels = 0u64;
    files
        .iter()
        .filter(|p| {
            let (w, h) = png_size(source, p).unwrap_or((0, 0));
            texels += u64::from(w) * u64::from(h);
            texels <= budget
        })
        .map(|p| (*p).clone())
        .collect()
}
/// A PNG's width and height from its first 24 bytes (`IHDR`), before any
/// pixel buffer is made.
pub fn png_bytes_size(data: &[u8]) -> Option<(u32, u32)> {
    let header = data.get(..24)?;
    if header[..8] != *b"\x89PNG\r\n\x1a\n" || header[12..16] != *b"IHDR" {
        return None;
    }
    let field = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap_or_default());
    Some((field(16), field(20)))
}
/// A PNG's width and height, from its header alone.
pub(super) fn png_size(source: &ResourceSource, path: &str) -> Option<(u32, u32)> {
    png_bytes_size(&source.read_head(path, 24).ok()?)
}
pub(super) fn walk(source: &ResourceSource, directory: &str, out: &mut Vec<String>) {
    out.extend(
        source
            .list(&format!("{directory}/"))
            .into_iter()
            .filter(|name| name.ends_with(".png")),
    );
}
/// `MissingTextureAtlasSprite.generateMissingImage`: 16x16, magenta and black quarters.
pub(crate) fn missing_image() -> (u32, u32, Vec<u8>) {
    let pixels = (0..256)
        .flat_map(|i| match (i / 16 < 8) ^ (i % 16 < 8) {
            true => [248, 0, 248, 255],
            false => [0, 0, 0, 255],
        })
        .collect();
    (16, 16, pixels)
}
pub(crate) fn decode_resource(source: &ResourceSource, name: &str) -> Result<(u32, u32, Vec<u8>)> {
    decode_bytes(&source.read(name)?)
}
/// A packed atlas image: sprite name, size and RGBA pixels.
pub(super) type PackedImage = (String, u32, u32, Vec<u8>);
/// A frame at `x, y`, sampled as if squared to its longer side and then resized
/// to `output_side`, without making the intermediate square. The output is held
/// to `budget` bytes before it is made.
pub(crate) fn strip_frame(
    pixels: &[u8],
    stride: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    output_side: u32,
    budget: usize,
) -> Result<Vec<u8>> {
    let side = w.max(h);
    let mut out = image_buffer(&[output_side as usize, output_side as usize, 4], budget)?;
    // Keep the two nearest-neighbour roundings when shrinking a rectangular frame.
    let index = |i| {
        if output_side == side {
            i
        } else {
            i * side / output_side
        }
    };
    for row in 0..output_side {
        let source_row = y + index(row) * h / side;
        for column in 0..output_side {
            let at = ((source_row * stride + x + index(column) * w / side) * 4) as usize;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
    Ok(out)
}
/// The `animation` section of a texture's `.mcmeta`.
pub(crate) fn animation_of(mcmeta: Option<&str>) -> Option<AnimationMetadata> {
    let meta: serde_json::Value = crate::packs::first_json(mcmeta?.as_bytes()).ok()?;
    AnimationMetadata::from_json(&meta.get("animation")?.to_string()).ok()
}
/// An animated strip's first frame: the region the mcmeta's frame size picks
/// (`AnimationMetadataSection.frameSize`), which is the top-left square unless
/// it names other dimensions or the frames run along a row.
pub(super) fn first_frame(
    (w, h, pixels): (u32, u32, Vec<u8>),
    animation: Option<&AnimationMetadata>,
) -> Result<(u32, u32, Vec<u8>)> {
    let [fw, fh] = animation.map_or([w.min(h) as i32; 2], |meta| {
        meta.frame_size(w as i32, h as i32)
    });
    match (u32::try_from(fw), u32::try_from(fh)) {
        (Ok(fw), Ok(fh))
            if fw > 0 && fh > 0 && w % fw == 0 && h % fh == 0 && (fw, fh) != (w, h) =>
        {
            let side = fw.max(fh);
            let frame = strip_frame(&pixels, w, 0, 0, fw, fh, side, MAX_IMAGE_BYTES)?;
            Ok((side, side, frame))
        }
        _ => Ok((w, h, pixels)),
    }
}
/// `image` inside a one-texel border copied from its opposite edges, the
/// texels repeat addressing reads past each edge.
pub(super) fn wrap_border((name, w, h, data): &PackedImage) -> Result<PackedImage> {
    let invalid = "sprite border exceeds the RGBA8 address space";
    let (bw, bh) = (
        w.checked_add(2).ok_or(invalid)?,
        h.checked_add(2).ok_or(invalid)?,
    );
    let len = rgba_len([bw, bh]).ok_or(invalid)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)?;
    let w = *w as usize;
    let h = *h as usize;
    for y in 0..bh as usize {
        let row = (y + h - 1) % h * w;
        for x in 0..bw as usize {
            let at = (row + (x + w - 1) % w) * 4;
            out.extend_from_slice(&data[at..at + 4]);
        }
    }
    Ok((name.clone(), bw, bh, out))
}

/// Raw RGBA8 `w x h` image padded by a 1-texel wrap border copied from opposite edges.
pub(super) fn wrap_border_pixels(data: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (bw, bh) = (w + 2, h + 2);
    let mut out = Vec::with_capacity((bw * bh * 4) as usize);
    for y in 0..bh {
        let row = (y + h - 1) % h;
        for x in 0..bw {
            let at = ((row * w + (x + w - 1) % w) * 4) as usize;
            out.extend_from_slice(&data[at..at + 4]);
        }
    }
    out
}

/// `image` inside a one-texel border clamped from its own edge texels, matching
/// vanilla 26.3's `Stitcher` padding + `ANIMATE_SPRITE_BLIT` `CLAMP_TO_EDGE` so
/// boundary UV samples on GPUs never read the transparent atlas gutter and discard.
pub(super) fn clamp_border((name, w, h, data): &PackedImage) -> Result<PackedImage> {
    let invalid = "sprite border exceeds the RGBA8 address space";
    let (bw, bh) = (
        w.checked_add(2).ok_or(invalid)?,
        h.checked_add(2).ok_or(invalid)?,
    );
    let len = rgba_len([bw, bh]).ok_or(invalid)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)?;
    let w = *w as usize;
    let h = *h as usize;
    for y in 0..bh as usize {
        let row = y.saturating_sub(1).min(h - 1) * w;
        for x in 0..bw as usize {
            let col = x.saturating_sub(1).min(w - 1);
            let at = (row + col) * 4;
            out.extend_from_slice(&data[at..at + 4]);
        }
    }
    Ok((name.clone(), bw, bh, out))
}

/// Raw RGBA8 `w x h` image padded by a 1-texel clamp-to-edge border on all four sides.
pub(super) fn clamp_border_pixels(data: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (bw, bh) = (w + 2, h + 2);
    let mut out = Vec::with_capacity((bw * bh * 4) as usize);
    for y in 0..bh {
        let row = y.saturating_sub(1).min(h - 1);
        for x in 0..bw {
            let col = x.saturating_sub(1).min(w - 1);
            let at = ((row * w + col) * 4) as usize;
            out.extend_from_slice(&data[at..at + 4]);
        }
    }
    out
}

/// Strip a 1-texel border from `(w, h, data)` back to `(w - 2, h - 2)`.
pub(super) fn strip_border(w: u32, h: u32, data: &[u8]) -> (u32, u32, Vec<u8>) {
    let (iw, ih) = (w - 2, h - 2);
    let mut out = Vec::with_capacity((iw * ih * 4) as usize);
    for y in 0..ih {
        let src = (((y + 1) * w + 1) * 4) as usize;
        out.extend_from_slice(&data[src..src + (iw * 4) as usize]);
    }
    (iw, ih, out)
}

/// Populate the 1-texel border around a sprite placed at `(x, y)` of interior size `(w, h)`
/// inside `pixels` (`atlas_w` wide), either wrapping opposite edges (`wrap`) or clamping.
pub(super) fn write_atlas_border(
    pixels: &mut [u8],
    atlas_w: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    wrap: bool,
) {
    if x == 0 || y == 0 || w == 0 || h == 0 {
        return;
    }
    let copy_px = |pixels: &mut [u8], sx: u32, sy: u32, dx: u32, dy: u32| {
        let s = ((sy * atlas_w + sx) * 4) as usize;
        let d = ((dy * atlas_w + dx) * 4) as usize;
        let px = [pixels[s], pixels[s + 1], pixels[s + 2], pixels[s + 3]];
        pixels[d..d + 4].copy_from_slice(&px);
    };
    let (src_left, src_right) = if wrap { (x + w - 1, x) } else { (x, x + w - 1) };
    let (src_top, src_bottom) = if wrap { (y + h - 1, y) } else { (y, y + h - 1) };
    for dy in 0..h {
        copy_px(pixels, src_left, y + dy, x - 1, y + dy);
        copy_px(pixels, src_right, y + dy, x + w, y + dy);
    }
    for dx in 0..w {
        copy_px(pixels, x + dx, src_top, x + dx, y - 1);
        copy_px(pixels, x + dx, src_bottom, x + dx, y + h);
    }
    copy_px(pixels, src_left, src_top, x - 1, y - 1);
    copy_px(pixels, src_right, src_top, x + w, y - 1);
    copy_px(pixels, src_left, src_bottom, x - 1, y + h);
    copy_px(pixels, src_right, src_bottom, x + w, y + h);
}

/// The texture of every `bitmap` font provider. A provider's `file` may name
/// any texture, so a pack's glyph sheet need not live under `textures/font`.
pub(super) fn bitmap_font_textures(root: &ResourceSource) -> Vec<String> {
    let defs = root
        .namespaces("assets")
        .into_iter()
        .flat_map(|ns| root.list(&format!("assets/{ns}/font/")))
        .filter(|path| path.ends_with(".json"));
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for def in defs {
        let Ok(bytes) = root.read(&def) else { continue };
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            continue;
        };
        for p in json["providers"].as_array().into_iter().flatten() {
            if p["type"] != "bitmap" {
                continue;
            }
            let Some(file) = p["file"].as_str() else {
                continue;
            };
            let (ns, path) = file.split_once(':').unwrap_or(("minecraft", file));
            let path = format!("assets/{ns}/textures/{path}");
            if root.contains(&path) && seen.insert(path.clone()) {
                out.push(path);
            }
        }
    }
    out
}

/// Folders of `assets/minecraft/textures/` that [`HudAtlas::pack`] walks for
/// the full atlas. A pack may add others through `atlases/blocks.json`.
pub(super) const WALKED_TEXTURE_FOLDERS: &[&str] = &[
    "gui",
    "item",
    "block",
    "font",
    "environment",
    "misc",
    "painting",
    "entity",
    "particle",
    "mob_effect",
    "map",
];

pub(super) fn walked_minecraft_texture(name: &str) -> bool {
    WALKED_TEXTURE_FOLDERS
        .iter()
        .any(|folder| name.starts_with(folder) && name.as_bytes().get(folder.len()) == Some(&b'/'))
}

/// Whether the file a walk packed under its own name tiles into its animation's frames.
pub(super) fn walked_frames_fit(root: &ResourceSource, texture: &str) -> bool {
    let file = crate::atlas_sources::texture_resource(texture, "textures");
    let mcmeta = root.read_to_string(&format!("{file}.mcmeta")).ok();
    png_size(root, &file).is_some_and(|size| {
        crate::atlas_sources::frames_fit(size, animation_of(mcmeta.as_deref()).as_ref())
    })
}
