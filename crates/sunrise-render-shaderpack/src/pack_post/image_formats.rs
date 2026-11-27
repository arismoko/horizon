//! Image formats: a storage image's format qualifier as wgpu's, which texture formats an image
//! binds in place of its qualifier's, and the format a module's storage images then declare.

/// Makes each storage image of `module` in group 0 declare the format its layout entry binds,
/// where `rebinds` takes the texture's format for the image's qualifier: a `layout(rgba16)` image
/// whose texture is `RGBA16F` (`bindings`). A new type for each such image, so another image
/// sharing the declared type keeps it.
pub fn storage_formats_of(module: &mut naga::Module, entries: &[wgpu::BindGroupLayoutEntry]) {
    let globals: Vec<_> = module.global_variables.iter().map(|(h, _)| h).collect();
    for handle in globals {
        let var = &module.global_variables[handle];
        let Some(naga::ResourceBinding { group: 0, binding }) = var.binding else {
            continue;
        };
        let naga::TypeInner::Image {
            dim,
            arrayed,
            class: naga::ImageClass::Storage { format, access },
        } = module.types[var.ty].inner
        else {
            continue;
        };
        let bound = entries.iter().find_map(|e| match e.ty {
            wgpu::BindingType::StorageTexture { format, .. } if e.binding == binding => {
                Some(format)
            }
            _ => None,
        });
        let Some((bound, declared)) = bound.zip(texture_format_of(format)) else {
            continue;
        };
        let Some(format) = rebinds(declared, bound)
            .then(|| storage_format_of(bound))
            .flatten()
        else {
            continue;
        };
        let ty = module.types.insert(
            naga::Type {
                name: module.types[var.ty].name.clone(),
                inner: naga::TypeInner::Image {
                    dim,
                    arrayed,
                    class: naga::ImageClass::Storage { format, access },
                },
            },
            naga::Span::UNDEFINED,
        );
        module.global_variables[handle].ty = ty;
    }
}

/// How program `program` binds its image `image`, declared `layout(<qualifier>)` (wgpu format
/// `declared`), to its texture of format `texture`: `Ok(None)` in the qualifier's format when
/// they match; `Ok(Some(note))` in the texture's format when `rebinds` takes it, the note saying
/// so (decision D-3: a store of 2.0 through `layout(rgba16)` stays 2.0 in an `RGBA16F` texture,
/// where an `RGBA16` one clamps it to 1.0); `Err(note)` when the program is refused.
pub fn image_binding(
    program: &str,
    image: &str,
    qualifier: &str,
    declared: wgpu::TextureFormat,
    texture: wgpu::TextureFormat,
) -> Result<Option<String>, String> {
    if declared == texture {
        Ok(None)
    } else if rebinds(declared, texture) {
        Ok(Some(format!(
            "{program}: `{image}` is {qualifier} but its texture is {texture:?}; bound as \
             {texture:?}, so its loads and stores convert by {texture:?}, not by {qualifier}"
        )))
    } else {
        Err(format!(
            "{program}: `{image}` is {qualifier} but its texture is {texture:?}"
        ))
    }
}

/// Whether program `program` may read image `image` of texture format `texture` through its
/// sampler `sampler`, declared as a `ty` (`usampler2D`...): `Err(note)` refuses the program when
/// the sampler's class differs from the texture's (decision D-6: a `usampler2D` over an `R32F`
/// image), naming the program, the image, the sampler's type and the texture's format. wgpu
/// would otherwise refuse the program's pipeline.
pub fn sampled_image(
    program: &str,
    image: &str,
    ty: &str,
    sampler: &str,
    texture: wgpu::TextureFormat,
    sample_type: wgpu::TextureSampleType,
) -> Result<(), String> {
    if super::fits(ty, sample_type) {
        Ok(())
    } else {
        Err(format!(
            "{program}: `{ty} {sampler}` samples image `{image}`, but its texture is \
             {texture:?}; skipped"
        ))
    }
}

/// Whether an image whose qualifier is `declared` binds its texture of format `texture` in that
/// texture's format (decision D-3): a 16-bit normalized qualifier (`rgba16`, `rg16_snorm`...)
/// on the 16-bit float format of the same channels, which the pack's `RGBA16F` buffer or
/// Sunrise's stand-in for a 16-bit normalized one is; and `r11f_g11f_b10f` on Sunrise's `RGBA16F`
/// stand-in for an `R11F_G11F_B10F` buffer or image. Other differing pairs are refused.
pub fn rebinds(declared: wgpu::TextureFormat, texture: wgpu::TextureFormat) -> bool {
    use wgpu::TextureFormat as T;
    matches!(
        (declared, texture),
        (
            T::Rgba16Unorm | T::Rgba16Snorm | T::Rg11b10Ufloat,
            T::Rgba16Float
        ) | (T::Rg16Unorm | T::Rg16Snorm, T::Rg16Float)
            | (T::R16Unorm | T::R16Snorm, T::R16Float)
    )
}

/// The wgpu format of a naga storage format `rebinds` may replace.
fn texture_format_of(format: naga::StorageFormat) -> Option<wgpu::TextureFormat> {
    use naga::StorageFormat as S;
    use wgpu::TextureFormat as T;
    Some(match format {
        S::Rgba16Unorm => T::Rgba16Unorm,
        S::Rgba16Snorm => T::Rgba16Snorm,
        S::Rg16Unorm => T::Rg16Unorm,
        S::Rg16Snorm => T::Rg16Snorm,
        S::R16Unorm => T::R16Unorm,
        S::R16Snorm => T::R16Snorm,
        S::Rg11b10Ufloat => T::Rg11b10Ufloat,
        _ => return None,
    })
}

/// The naga storage format of a texture format `rebinds` binds.
fn storage_format_of(format: wgpu::TextureFormat) -> Option<naga::StorageFormat> {
    use naga::StorageFormat as S;
    use wgpu::TextureFormat as T;
    Some(match format {
        T::Rgba16Float => S::Rgba16Float,
        T::Rg16Float => S::Rg16Float,
        T::R16Float => S::R16Float,
        _ => return None,
    })
}

/// The wgpu format of an image's format qualifier.
pub fn image_format(qualifier: &str) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as T;
    Some(match qualifier {
        "rgba32f" => T::Rgba32Float,
        "rgba16f" => T::Rgba16Float,
        "rg32f" => T::Rg32Float,
        "rg16f" => T::Rg16Float,
        "r11f_g11f_b10f" => T::Rg11b10Ufloat,
        "r32f" => T::R32Float,
        "r16f" => T::R16Float,
        "rgba16" => T::Rgba16Unorm,
        "rgb10_a2" => T::Rgb10a2Unorm,
        "rgba8" => T::Rgba8Unorm,
        "rg16" => T::Rg16Unorm,
        "rg8" => T::Rg8Unorm,
        "r16" => T::R16Unorm,
        "r8" => T::R8Unorm,
        "rgba16_snorm" => T::Rgba16Snorm,
        "rgba8_snorm" => T::Rgba8Snorm,
        "rg16_snorm" => T::Rg16Snorm,
        "rg8_snorm" => T::Rg8Snorm,
        "r16_snorm" => T::R16Snorm,
        "r8_snorm" => T::R8Snorm,
        "rgba32i" => T::Rgba32Sint,
        "rgba16i" => T::Rgba16Sint,
        "rgba8i" => T::Rgba8Sint,
        "rg32i" => T::Rg32Sint,
        "rg16i" => T::Rg16Sint,
        "rg8i" => T::Rg8Sint,
        "r32i" => T::R32Sint,
        "r16i" => T::R16Sint,
        "r8i" => T::R8Sint,
        "rgba32ui" => T::Rgba32Uint,
        "rgba16ui" => T::Rgba16Uint,
        "rgb10_a2ui" => T::Rgb10a2Uint,
        "rgba8ui" => T::Rgba8Uint,
        "rg32ui" => T::Rg32Uint,
        "rg16ui" => T::Rg16Uint,
        "rg8ui" => T::Rg8Uint,
        "r32ui" => T::R32Uint,
        "r16ui" => T::R16Uint,
        "r8ui" => T::R8Uint,
        _ => return None,
    })
}
