//! Portable resource-pack and data-pack metadata, format compatibility, symlink validation,
//! resource filters, merge classification, and `PackSelectionModel` state.
use crate::resource_source::stays_inside;
use crate::text::{Style, Text};
use pattern::Pattern;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
pub use sunrise_game_data::pattern;

/// `PackCompatibility`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    Compatible,
    TooOld,
    TooNew,
    Unknown,
}

impl Compat {
    pub fn key(self) -> &'static str {
        match self {
            Compat::TooOld => "old",
            Compat::TooNew => "new",
            Compat::Unknown => "unknown",
            Compat::Compatible => "compatible",
        }
    }
}
/// `PackSource`: how the description is decorated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackSource {
    Default,
    BuiltIn,
    Feature,
    World,
    Server,
}

/// A pack as the screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PackInfo {
    pub id: String,
    pub title: Text,
    pub description: Text,
    /// The description component's own (root) style.
    pub description_style: Style,
    pub source: PackSource,
    pub compat: Compat,
    /// The pack's `pack.png` (decoded RGBA); none draws `unknown_pack`.
    pub icon: Option<Arc<[u8]>>,
    pub required: bool,
    pub position: Position,
    pub fixed: bool,
    /// Left out of both lists (a mod's `PackRegistration::hidden`): the selection model still
    /// holds it, so a selected hidden pack stays selected and loads.
    pub hidden: bool,
}
impl PackInfo {
    pub fn selectable(&self) -> Selectable {
        Selectable {
            id: self.id.clone(),
            required: self.required,
            position: self.position,
            fixed: self.fixed,
            compatible: self.compat == Compat::Compatible,
        }
    }
    /// `PackSource.decorate` merged with the entry's grey
    /// (`getExtendedDescription`, `ComponentUtils.mergeStyles`), and the style of
    /// its root, which the ellipsis of a cut description takes.
    pub fn extended_description(&self) -> (Text, Style) {
        const CHAT_GRAY: u32 = 0xAAAAAA;
        let gray = Style::color(0x808080);
        let key = match self.source {
            PackSource::Default => {
                let mut root = self.description_style.clone();
                root.inherit(&gray);
                return (self.description.clone().with_style(&gray), root);
            }
            PackSource::BuiltIn => "pack.source.builtin",
            PackSource::Feature => "pack.source.feature",
            PackSource::World => "pack.source.world",
            PackSource::Server => "pack.source.server",
        };
        let t = Text::translate(
            "pack.nameAndSource",
            &[self.description.clone(), Text::translate(key, &[])],
        )
        .with_color(CHAT_GRAY);
        (t, Style::color(CHAT_GRAY))
    }
}

/// SharedConstants resource pack format of 26.3 (major, minor).
pub const RESOURCE_FORMAT: (i64, i64) = sunrise_core::version::RESOURCE_PACK_FORMAT;

#[derive(Clone, Debug)]
pub enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
    Opened(crate::resource_source::ResourceSource),
}
impl Source {
    /// Open this layer once for a load/reload, sharing it across resource reads.
    pub fn open(&self) -> io::Result<crate::resource_source::ResourceSource> {
        match self {
            Self::Dir(root) => Ok(crate::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            Self::Zip(path) => crate::resource_source::ResourceSource::archive(path),
            Self::Opened(source) => Ok(source.clone()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    namespace: Option<Pattern>,
    path: Option<Pattern>,
}
impl Filter {
    pub fn blocks(&self, file: &str) -> bool {
        let Some(rest) = file.strip_prefix("assets/") else {
            return false;
        };
        let Some((namespace, path)) = rest.split_once('/') else {
            return false;
        };
        self.matches(namespace, path)
    }

    pub fn matches(&self, namespace: &str, path: &str) -> bool {
        self.namespace
            .as_ref()
            .is_none_or(|p| p.is_match(namespace))
            && self.path.as_ref().is_none_or(|p| p.is_match(path))
    }
}
/// A text component from JSON (strings, `text`, `translate`, arrays, `extra`
/// and the style fields), legacy `§` codes resolved as the font does.
pub fn component(v: &serde_json::Value) -> Text {
    component_root(v).0.resolve_legacy()
}
/// The component, its legacy codes left in place (each span is a
/// component's contents and style), and its root style (`Component.getStyle`).
pub fn component_root(v: &serde_json::Value) -> (Text, Style) {
    let mut out = Text::empty();
    let root = build(v, &Style::default(), &mut out);
    (out, root)
}
fn style_of(o: &serde_json::Map<String, serde_json::Value>) -> Style {
    let flag = |k: &str| o.get(k).and_then(|b| b.as_bool());
    Style {
        color: o.get("color").and_then(|c| c.as_str()).and_then(|c| {
            crate::text::named_color(c).or_else(|| {
                c.strip_prefix('#')
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
            })
        }),
        bold: flag("bold"),
        italic: flag("italic"),
        underlined: flag("underlined"),
        strikethrough: flag("strikethrough"),
        obfuscated: flag("obfuscated"),
        font: o.get("font").and_then(crate::text::font_id),
        ..Style::default()
    }
}
/// Appends `v` under `parent`'s style; returns the component's own style.
fn build(v: &serde_json::Value, parent: &Style, out: &mut Text) -> Style {
    match v {
        serde_json::Value::Array(a) => {
            let Some(first) = a.first() else {
                return Style::default();
            };
            let own = build(first, parent, out);
            let mut style = own.clone();
            style.inherit(parent);
            for e in &a[1..] {
                build(e, &style, out);
            }
            own
        }
        serde_json::Value::Object(o) => {
            let own = style_of(o);
            let mut style = own.clone();
            style.inherit(parent);
            if let Some(s) = o.get("text").and_then(|s| s.as_str()) {
                out.push(Text::styled(s, style.clone()));
            } else if let Some(k) = o.get("translate").and_then(|s| s.as_str()) {
                let args: Vec<Text> = o
                    .get("with")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().map(component).collect())
                    .unwrap_or_default();
                out.push(Text::translate(k, &args).with_style(&style));
            }
            match o.get("extra") {
                Some(serde_json::Value::Array(extra)) => {
                    for e in extra {
                        build(e, &style, out);
                    }
                }
                Some(e) => {
                    build(e, &style, out);
                }
                None => {}
            }
            own
        }
        serde_json::Value::String(s) => {
            out.push(Text::styled(s.as_str(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Number(n) => {
            out.push(Text::styled(n.to_string(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Bool(b) => {
            out.push(Text::styled(b.to_string(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Null => Style::default(),
    }
}
fn format_of(v: &serde_json::Value) -> Option<(i64, i64)> {
    match v {
        serde_json::Value::Number(n) => Some((n.as_i64()?, 0)),
        serde_json::Value::Array(a) => Some((
            a.first()?.as_i64()?,
            a.get(1).and_then(|m| m.as_i64()).unwrap_or(0),
        )),
        _ => None,
    }
}
/// `PackFormat`: major and minor.
pub type Format = (i64, i64);
/// `PackFormat.lastPreMinorVersion`: the last major without minor versions,
/// for the resource pack format (64) or the data pack one (81).
fn last_pre_minor(current: Format) -> i64 {
    if current.0 == sunrise_core::version::DATA_PACK_FORMAT.0 {
        81
    } else {
        64
    }
}
/// `PackFormat.fullCodec(default_minor)`: an int, or a list of 1 to 256
/// non-negative ints whose second is the minor. None: it does not parse.
fn format_codec(v: &serde_json::Value, default_minor: i64) -> Option<Format> {
    // `Codec.INT` reads any JSON number through `Number.intValue`: a decimal such as `97.1`
    // counts as its whole part (97), as it does for the game.
    let non_negative = |v: &serde_json::Value| {
        v.as_i64()
            .or_else(|| {
                v.as_f64()
                    .filter(|f| f.is_finite())
                    .map(|f| f.trunc() as i64)
            })
            .filter(|n| *n >= 0 && *n <= i64::from(i32::MAX))
    };
    match v {
        serde_json::Value::Array(a) if (1..=256).contains(&a.len()) => {
            let list: Option<Vec<i64>> = a.iter().map(non_negative).collect();
            let list = list?;
            Some((list[0], list.get(1).copied().unwrap_or(default_minor)))
        }
        serde_json::Value::Array(_) => None,
        v => Some((non_negative(v)?, default_minor)),
    }
}
/// `InclusiveRange.codec(Codec.INT)`: `[min, max]` or
/// `{min_inclusive, max_inclusive}`, min not above max.
fn int_range_codec(v: &serde_json::Value) -> Option<(i64, i64)> {
    let int = |v: &serde_json::Value| v.as_i64().filter(|n| i32::try_from(*n).is_ok());
    let (min, max) = match v {
        serde_json::Value::Array(a) if a.len() == 2 => (int(&a[0])?, int(&a[1])?),
        serde_json::Value::Object(o) => {
            (int(o.get("min_inclusive")?)?, int(o.get("max_inclusive")?)?)
        }
        _ => return None,
    };
    (min <= max).then_some((min, max))
}
/// An `optionalFieldOf`: absent is `Ok(None)`, one that does not parse an error.
fn optional<T>(
    v: &serde_json::Value,
    parse: impl Fn(&serde_json::Value) -> Option<T>,
) -> Result<Option<T>, ()> {
    if v.is_null() {
        Ok(None)
    } else {
        parse(v).map(Some).ok_or(())
    }
}
/// `PackFormat.IntermediaryFormat.validatePackFormatForRange`.
fn validate_pack_format_for_range(main_format: i64, min: i64, max: i64) -> Option<String> {
    if main_format < min || main_format > max {
        Some(format!(
            "Pack declared support for versions {min} to {max} but declared main format is {main_format}"
        ))
    } else if main_format < 15 {
        Some("Multi-version packs cannot support minimum version of less than 15, since this will leave versions in range unable to load pack.".into())
    } else {
        None
    }
}
/// `PackFormat` as `toString` prints it.
fn format_text(f: Format) -> String {
    if f.1 == i64::from(i32::MAX) {
        format!("{}.*", f.0)
    } else {
        format!("{}.{}", f.0, f.1)
    }
}
/// How `PackFormat.IntermediaryFormat.validate` is called: for a pack's `pack` section
/// (`hasPackFormatField`, no `requireOldField`) or for an overlay entry (neither flag set by
/// the entry, `requireOldField` from the whole list).
struct FormatRules<'a> {
    last_pre_minor: i64,
    has_pack_format: bool,
    require_old: bool,
    context: &'a str,
    old: &'a str,
}
/// `PackFormat.IntermediaryFormat.validate` for a pack's `pack` section
/// (`hasPackFormatField`, no `requireOldField`): the supported range, or why
/// vanilla refuses the metadata (it then falls back to a pack of unknown
/// format).
pub fn validate_pack_range(
    pack: &serde_json::Value,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    const CONTEXT: &str = "Pack";
    const OLD: &str = "supported_formats";
    let bad = |field: &str| format!("{CONTEXT} field {field} does not parse");
    let min =
        optional(&pack["min_format"], |v| format_codec(v, 0)).map_err(|()| bad("min_format"))?;
    let max = optional(&pack["max_format"], |v| {
        format_codec(v, i64::from(i32::MAX))
    })
    .map_err(|()| bad("max_format"))?;
    let format = optional(&pack["pack_format"], |v| {
        v.as_i64().filter(|n| i32::try_from(*n).is_ok())
    })
    .map_err(|()| bad("pack_format"))?;
    let supported = optional(&pack[OLD], int_range_codec).map_err(|()| bad(OLD))?;
    validate_format(
        min,
        max,
        format,
        supported,
        &FormatRules {
            last_pre_minor,
            has_pack_format: true,
            require_old: false,
            context: CONTEXT,
            old: OLD,
        },
    )
}
/// `OverlayMetadataSection.codecForPackType`: whether a pack's `overlays` section, when it has
/// one, is accepted. `Pack.readPackMetadata` refuses the pack when the codec fails: `entries`
/// missing or not a list, an entry whose `directory` fails `DIR_VALIDATOR` or whose formats
/// do not parse or validate (`PackFormat.validateHolderList`).
pub fn validate_overlays(meta: &serde_json::Value, current: Format) -> Result<(), String> {
    let last_pre_minor = last_pre_minor(current);
    let section = &meta["overlays"];
    if section.is_null() {
        return Ok(());
    }
    let entries = section["entries"]
        .as_array()
        .ok_or("overlays field entries does not parse")?;
    struct Entry<'a> {
        directory: &'a str,
        min: Option<Format>,
        max: Option<Format>,
        supported: Option<(i64, i64)>,
    }
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let directory = entry["directory"]
            .as_str()
            .filter(|d| is_overlay_directory(d))
            .ok_or("overlay directory is not accepted directory name")?;
        let bad = |field: &str| format!("Overlay \"{directory}\" field {field} does not parse");
        parsed.push(Entry {
            directory,
            min: optional(&entry["min_format"], |v| format_codec(v, 0))
                .map_err(|()| bad("min_format"))?,
            max: optional(&entry["max_format"], |v| {
                format_codec(v, i64::from(i32::MAX))
            })
            .map_err(|()| bad("max_format"))?,
            supported: optional(&entry["formats"], int_range_codec).map_err(|()| bad("formats"))?,
        });
    }
    // `IntermediaryFormat.effectiveMinMajorVersion`, minimum over the list.
    let min_version = parsed
        .iter()
        .map(|e| match (e.min, e.supported) {
            (Some(min), Some(s)) => min.0.min(s.0),
            (Some(min), None) => min.0,
            (None, Some(s)) => s.0,
            (None, None) => i64::from(i32::MAX),
        })
        .min()
        .unwrap_or(i64::from(i32::MAX));
    for e in &parsed {
        // An entry naming no format at all is only logged and skipped.
        if e.min.is_none() && e.max.is_none() && e.supported.is_none() {
            continue;
        }
        let context = format!("Overlay \"{}\"", e.directory);
        validate_format(
            e.min,
            e.max,
            e.min.map(|m| m.0),
            e.supported,
            &FormatRules {
                last_pre_minor,
                has_pack_format: false,
                require_old: min_version <= last_pre_minor,
                context: &context,
                old: "formats",
            },
        )?;
    }
    Ok(())
}
/// `PackFormat.IntermediaryFormat.validate`.
fn validate_format(
    min: Option<Format>,
    max: Option<Format>,
    format: Option<i64>,
    supported: Option<(i64, i64)>,
    rules: &FormatRules,
) -> Result<(Format, Format), String> {
    let FormatRules {
        last_pre_minor,
        has_pack_format,
        require_old,
        context,
        old,
    } = *rules;
    if min.is_some() != max.is_some() {
        return Err(format!(
            "{context} missing field, must declare both min_format and max_format"
        ));
    }
    if require_old && supported.is_none() {
        return Err(format!(
            "{context} missing required field {old}, must be present in all overlays for any overlays to work across game versions"
        ));
    }
    if let (Some(min), Some(max)) = (min, max) {
        return validate_new_format(min, max, format, supported, rules);
    }
    if let Some(supported) = supported {
        return validate_old_format(supported, format, rules);
    }
    match format {
        Some(main) if has_pack_format && main > last_pre_minor => Err(format!(
            "{context} declares support for version newer than {last_pre_minor}, but is missing mandatory fields min_format and max_format"
        )),
        Some(main) if has_pack_format => Ok(((main, 0), (main, 0))),
        _ => Err(format!(
            "{context} could not be parsed, missing format version information"
        )),
    }
}
/// `PackFormat.IntermediaryFormat.validateNewFormat`.
fn validate_new_format(
    min: Format,
    max: Format,
    format: Option<i64>,
    supported: Option<(i64, i64)>,
    rules: &FormatRules,
) -> Result<(Format, Format), String> {
    let FormatRules {
        last_pre_minor,
        has_pack_format,
        require_old,
        context,
        old,
    } = *rules;
    let (major_min, major_max) = (min.0, max.0);
    if min > max {
        return Err(format!(
            "{context} min_format ({}) is greater than max_format ({})",
            format_text(min),
            format_text(max)
        ));
    }
    if major_min > last_pre_minor && !require_old {
        if supported.is_some() {
            return Err(format!(
                "{context} key {old} is deprecated starting from pack format {}. Remove {old} from your pack.mcmeta.",
                last_pre_minor + 1
            ));
        }
        if has_pack_format
            && let Some(main) = format
            && let Some(error) = validate_pack_format_for_range(main, major_min, major_max)
        {
            return Err(error);
        }
    } else {
        let Some(declared) = supported else {
            return Err(format!(
                "{context} declares support for format {major_min}, but game versions supporting formats 15 to {last_pre_minor} require a {old} field. Add \"{old}\": [{major_min}, {last_pre_minor}] or require a version greater or equal to {}.0.",
                last_pre_minor + 1
            ));
        };
        if declared.0 != major_min {
            return Err(format!(
                "{context} version declaration mismatch between {old} (from {}) and min_format ({})",
                declared.0,
                format_text(min)
            ));
        }
        if declared.1 != major_max && declared.1 != last_pre_minor {
            return Err(format!(
                "{context} version declaration mismatch between {old} (up to {}) and max_format ({})",
                declared.1,
                format_text(max)
            ));
        }
        if has_pack_format {
            let Some(main) = format else {
                return Err(format!(
                    "{context} declares support for formats up to {last_pre_minor}, but game versions supporting formats 15 to {last_pre_minor} require a pack_format field. Add \"pack_format\": {major_min} or require a version greater or equal to {}.0.",
                    last_pre_minor + 1
                ));
            };
            if let Some(error) = validate_pack_format_for_range(main, major_min, major_max) {
                return Err(error);
            }
        }
    }
    Ok((min, max))
}
/// `PackFormat.IntermediaryFormat.validateOldFormat`.
fn validate_old_format(
    supported: (i64, i64),
    format: Option<i64>,
    rules: &FormatRules,
) -> Result<(Format, Format), String> {
    let FormatRules {
        last_pre_minor,
        has_pack_format,
        context,
        ..
    } = *rules;
    let (min, max) = supported;
    if max > last_pre_minor {
        return Err(format!(
            "{context} declares support for version newer than {last_pre_minor}, but is missing mandatory fields min_format and max_format"
        ));
    }
    if has_pack_format {
        let Some(main) = format else {
            return Err(format!(
                "{context} declares support for formats up to {last_pre_minor}, but game versions supporting formats 15 to {last_pre_minor} require a pack_format field. Add \"pack_format\": {min} or require a version greater or equal to {}.0.",
                last_pre_minor + 1
            ));
        };
        if let Some(error) = validate_pack_format_for_range(main, min, max) {
            return Err(error);
        }
    }
    Ok(((min, 0), (max, 0)))
}
/// PackCompatibility: inside the pack's declared range.
pub fn compatible(meta: &serde_json::Value) -> bool {
    compatibility(meta) == Compat::Compatible
}
/// `PackCompatibility.forVersion`: too old, too new, compatible, or unknown
/// when the metadata names no (valid) format.
pub fn compatibility(meta: &serde_json::Value) -> Compat {
    compatibility_for(meta, RESOURCE_FORMAT)
}
/// `compatibility` against `current`, the resource or the data pack format.
/// Metadata that fails `PackFormat.packCodec` parses as the fallback type,
/// whose format is `Integer.MAX_VALUE`: unknown.
pub fn compatibility_for(meta: &serde_json::Value, current: Format) -> Compat {
    match validate_pack_range(&meta["pack"], last_pre_minor(current)) {
        Err(_) => Compat::Unknown,
        Ok((_, max)) if max < current => Compat::TooOld,
        Ok((min, _)) if current < min => Compat::TooNew,
        Ok(_) => Compat::Compatible,
    }
}
/// Whether an `overlays.entries[]` item applies to the current resource format.
pub fn overlay_applies(entry: &serde_json::Value) -> bool {
    overlay_applies_for(entry, RESOURCE_FORMAT)
}
/// Whether an `overlays.entries[]` item applies to `current`
/// (OverlayMetadataSection: `formats` as an int, `[min, max]` or
/// `{min_inclusive, max_inclusive}`, or `min_format`/`max_format`).
pub fn overlay_applies_for(entry: &serde_json::Value, current: Format) -> bool {
    if let (Some(min), Some(max)) = (
        format_of(&entry["min_format"]),
        format_of(&entry["max_format"]),
    ) {
        return min <= current && current <= (max.0, if max.1 == 0 { i64::MAX } else { max.1 });
    }
    let formats = &entry["formats"];
    if let Some(n) = formats.as_i64() {
        return n == current.0;
    }
    if let Some(a) = formats.as_array() {
        return match (
            a.first().and_then(|v| v.as_i64()),
            a.get(1).and_then(|v| v.as_i64()),
        ) {
            (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
            _ => false,
        };
    }
    match (
        formats["min_inclusive"].as_i64(),
        formats["max_inclusive"].as_i64(),
    ) {
        (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
        _ => false,
    }
}
/// The overlay directories of a data pack's metadata that apply to the data pack format.
pub fn data_overlays(meta: &serde_json::Value) -> Vec<String> {
    overlay_directories(meta, |e| {
        overlay_applies_for(e, sunrise_core::version::DATA_PACK_FORMAT)
    })
}
pub fn overlay_directories(
    meta: &serde_json::Value,
    active: impl Fn(&serde_json::Value) -> bool,
) -> Vec<String> {
    meta["overlays"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| active(e))
        .filter_map(|e| e["directory"].as_str().map(str::to_owned))
        // OverlayMetadataSection's DIR_VALIDATOR: one folder of the pack, `[-_a-zA-Z0-9.]+`.
        .filter(|d| stays_inside(d) && is_overlay_directory(d))
        .collect()
}
/// `OverlayMetadataSection.DIR_VALIDATOR`: `[-_a-zA-Z0-9.]+`, so one folder of the pack.
pub fn is_overlay_directory(d: &str) -> bool {
    !d.is_empty()
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub fn filter_of(meta: &serde_json::Value) -> Vec<Filter> {
    meta["filter"]["block"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| Filter {
            namespace: b["namespace"].as_str().and_then(Pattern::new),
            path: b["path"].as_str().and_then(Pattern::new),
        })
        .collect()
}

pub fn read_meta(source: &Source) -> Option<serde_json::Value> {
    let bytes = source.open().ok()?.read("pack.mcmeta").ok()?;
    parse_meta(&bytes)
}
/// The first JSON value of a `pack.mcmeta`: what follows it is ignored, as some packs sign the
/// file with a comment after the closing brace. Gson reads past a UTF-8 byte order mark;
/// serde_json rejects it.
pub fn parse_meta(bytes: &[u8]) -> Option<serde_json::Value> {
    first_json(bytes).ok()
}
/// The first JSON value of a `.mcmeta` file, as `ResourceMetadata.fromJsonStream` reads it:
/// Gson's `TypeAdapter.read` stops after the value, so text after it (packs leave `//` notes
/// there) is never read, and a UTF-8 byte order mark is skipped.
pub fn first_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match serde_json::Deserializer::from_slice(bytes)
        .into_iter()
        .next()
    {
        Some(value) => value,
        None => Err(serde::de::Error::custom("empty metadata")),
    }
}

/// `LevelStorageSource.ALLOWED_SYMLINKS_CONFIG_NAME`: the game folder's list of
/// symbolic link targets the game accepts.
pub const ALLOWED_SYMLINKS_CONFIG_NAME: &str = "allowed_symlinks.txt";

/// `DirectoryValidator` over vanilla's `PathAllowList`: which symbolic link
/// targets the game accepts. With no entry, no link is, so a link planted in a
/// pack folder cannot point the game at the rest of the disk.
#[derive(Default)]
pub struct DirectoryValidator {
    /// `EntryType.PREFIX`: a target whose text starts with the entry matches.
    prefixes: Vec<String>,
}
impl DirectoryValidator {
    /// `LevelStorageSource.parseValidator`: the entries of `config`, none when
    /// it is missing, unreadable or unparseable. `[glob]` and `[regex]` entries
    /// are skipped with a warning: Sunrise has no Java path matcher to compile
    /// them with, so a skipped entry allows less than vanilla's, never more,
    /// except that vanilla denies every link when an entry fails to compile.
    pub fn parse(config: &Path) -> Self {
        let Ok(text) = fs::read_to_string(config) else {
            return Self::default();
        };
        let mut prefixes = Vec::new();
        for line in text.lines() {
            // `ConfigEntry.parse`: blank lines and comments, untrimmed otherwise.
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(rest) = line.strip_prefix('[') else {
                prefixes.push(line.to_owned());
                continue;
            };
            let Some((kind, pattern)) = rest.split_once(']') else {
                // `ConfigEntry.parse` throws on an unterminated type, and
                // `parseValidator` then keeps no entry at all.
                log::error!(
                    "Sunrise: {}: unterminated type in {line:?}",
                    config.display()
                );
                return Self::default();
            };
            match kind {
                "prefix" => prefixes.push(pattern.to_owned()),
                "glob" | "regex" => log::warn!(
                    "Sunrise: {}: {kind} entry {pattern:?} is not supported and allows nothing",
                    config.display()
                ),
                _ => {
                    // An unsupported type throws in `parse`, as an unterminated
                    // one does: the whole file then allows no link.
                    log::error!(
                        "Sunrise: {}: unsupported type in {line:?}",
                        config.display()
                    );
                    return Self::default();
                }
            }
        }
        Self { prefixes }
    }
    /// The game folder's list, as `Minecraft` reads it from
    /// `gameDirPath.resolve("allowed_symlinks.txt")`; the game folder is the
    /// working directory, as for `options.txt`.
    pub fn game() -> Self {
        Self::parse(
            &std::env::current_dir()
                .unwrap_or_default()
                .join(ALLOWED_SYMLINKS_CONFIG_NAME),
        )
    }
    /// `PathAllowList.matches`, on the target as written, not resolved.
    fn allows(&self, target: &Path) -> bool {
        target
            .to_str()
            .is_some_and(|t| self.prefixes.iter().any(|p| t.starts_with(p.as_str())))
    }
}
/// `DirectoryValidator.validateKnownDirectory`: every link under `dir`, never
/// following one, added to `issues` with its target unless the validator
/// allows the target.
fn forbidden_links(
    dir: &Path,
    validator: &DirectoryValidator,
    issues: &mut Vec<(PathBuf, PathBuf)>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            if !validator.allows(&target) {
                issues.push((path, target));
            }
        } else if kind.is_dir() {
            forbidden_links(&path, validator, issues)?;
        }
    }
    Ok(())
}
/// `FolderRepositorySource.discoverPacks`'s warning for a refused entry: the
/// entry, as vanilla names it, and every forbidden link in it.
fn refused(entry: &Path, issues: &[(PathBuf, PathBuf)]) {
    let list: Vec<String> = issues
        .iter()
        .map(|(link, target)| format!("{}->{}", link.display(), target.display()))
        .collect();
    log::warn!(
        "Ignoring potential pack entry: Failed to validate '{}'. Found forbidden symlinks: {}",
        entry.display(),
        list.join(", ")
    );
}
/// `PackDetector.detectPackResources`: the path a pack entry names, once the
/// validator accepts the entry and every link inside it; `None` when it is
/// missing or not a pack, `Err` when it cannot be read. A link is followed one
/// level, to its target as written rather than resolved, as vanilla does.
pub fn detected_pack_path(
    path: &Path,
    validator: &DirectoryValidator,
) -> io::Result<Option<PathBuf>> {
    let mut kind = match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None), // `NoSuchFileException`
        Err(err) => return Err(err),
    };
    let mut target = path.to_path_buf();
    if kind.is_symlink() {
        let link = fs::read_link(path)?;
        if !validator.allows(&link) {
            refused(path, &[(path.to_path_buf(), link)]);
            return Ok(None);
        }
        target = link;
        // One level only: the attributes read on the target do not follow a
        // second link, and such an entry is neither a directory nor a file.
        kind = fs::symlink_metadata(&target)?.file_type();
        if kind.is_symlink() {
            return Ok(None);
        }
    }
    if kind.is_dir() {
        let mut issues = Vec::new();
        forbidden_links(&target, validator, &mut issues)?;
        if !issues.is_empty() {
            refused(path, &issues);
            return Ok(None);
        }
    }
    Ok(Some(target))
}

/// Overlay directories active for the current resource format without Polytone condition gating.
pub fn vanilla_overlays_of(meta: &serde_json::Value) -> Vec<String> {
    meta["overlays"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| overlay_applies(e))
        .filter_map(|e| e["directory"].as_str().map(str::to_owned))
        // OverlayMetadataSection's DIR_VALIDATOR: one folder of the pack, `[-_a-zA-Z0-9.]+`.
        .filter(|d| {
            stays_inside(d)
                && !d.contains('/')
                && d.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
        .collect()
}

/// Portable pack metadata entry (mirrors `packs::Pack` without client Polytone configs).
#[derive(Clone, Debug)]
pub struct PackMeta {
    /// `file/<name>` for user packs, the bare name for built-in ones.
    pub id: String,
    pub source: Source,
    pub title: Text,
    pub description: Text,
    /// The description with its legacy codes, and its root style (the pack screen's).
    pub raw_description: Text,
    pub description_style: Style,
    pub compatible: bool,
    pub compatibility: Compat,
    /// Overlay directories active for this game version, in `pack.mcmeta` order.
    pub overlays: Vec<String>,
    /// `filter.block` entries (namespace and path patterns) hiding resources
    /// of the packs below this one.
    pub filter: Vec<Filter>,
    /// Presentation and selection supplied by an additional repository source.
    pub selection: Option<PackInfo>,
}

impl PackMeta {
    /// The pack's `pack.png` (`PackResources.getRootResource`), for its icon.
    pub fn icon_png(&self) -> Option<Vec<u8>> {
        self.source.open().ok()?.read("pack.png").ok()
    }
}

/// A portable pack whose compatibility is against `format`.
pub fn pack_meta_for(
    id: String,
    title: Text,
    source: Source,
    format: (i64, i64),
) -> Option<PackMeta> {
    let meta = read_meta(&source)?;
    Some(pack_meta_from(id, title, source, format, &meta))
}

pub fn pack_meta_from(
    id: String,
    title: Text,
    source: Source,
    format: (i64, i64),
    meta: &serde_json::Value,
) -> PackMeta {
    let (raw_description, description_style) = component_root(&meta["pack"]["description"]);
    PackMeta {
        id,
        title,
        description: raw_description.resolve_legacy(),
        raw_description,
        description_style,
        compatible: compatible(meta),
        compatibility: compatibility_for(meta, format),
        overlays: vanilla_overlays_of(meta),
        filter: filter_of(meta),
        source,
        selection: None,
    }
}

/// The packs (folders and zips) in `dir`, by name, as `FolderRepositorySource`
/// finds them; their compatibility is against `format`.
pub fn folder_meta(dir: &Path, format: (i64, i64)) -> Vec<PackMeta> {
    folder_meta_with(dir, format, &DirectoryValidator::game())
}

/// [`folder_meta`] under an explicit `allowed_symlinks.txt` validator.
pub fn folder_meta_with(
    dir: &Path,
    format: (i64, i64),
    validator: &DirectoryValidator,
) -> Vec<PackMeta> {
    let mut user: Vec<PackMeta> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = match detected_pack_path(&e.path(), validator) {
                Ok(Some(path)) => path,
                Ok(None) => return None,
                Err(err) => {
                    log::warn!(
                        "Failed to read properties of '{}', ignoring: {err}",
                        e.path().display()
                    );
                    return None;
                }
            };
            let source = if path.is_dir() {
                Source::Dir(path)
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            {
                Source::Zip(path)
            } else {
                return None;
            };
            pack_meta_for(format!("file/{name}"), Text::literal(name), source, format)
        })
        .collect();
    user.sort_by(|a, b| a.id.cmp(&b.id));
    user
}

/// Files vanilla loads from every pack in turn (`getResourceStack`) rather
/// than from the top pack only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Merge {
    /// `ClientLanguage.appendFrom`: later packs override keys.
    Lang,
    /// `SoundManager.Preparations.handleRegistration`: sounds add up unless
    /// an event says `replace`.
    Sounds,
    /// `FontManager.loadResourceStack`: the top pack's providers come first.
    Font,
    /// `SpriteSourceList.load`: sources from the bottom pack up.
    Atlas,
    /// `BlockStateModelLoader.loadBlockStateDefinitionStack`: every pack's
    /// definition binds on its own, and a higher one replaces only the states
    /// it binds. Kept as its layers ([`BLOCKSTATE_LAYERS`]) for the catalog.
    BlockStates,
}
/// The key of a stacked blockstate file: its definitions, bottom first, each
/// as the pack wrote it.
pub const BLOCKSTATE_LAYERS: &str = "sunrise:layers";
/// Whether the packs merger stacks this file's every layer (atlases, lang,
/// sounds, blockstates) instead of letting the topmost one win.
pub fn merge_stacks(name: &str) -> bool {
    merge_kind(name).is_some()
}

pub const VANILLA_LANG_DEPRECATIONS: &str = "assets/minecraft/lang/deprecated.json";

pub fn merge_kind(name: &str) -> Option<Merge> {
    let (_, rest) = name.strip_prefix("assets/")?.split_once('/')?;
    if name == VANILLA_LANG_DEPRECATIONS {
        return None;
    }
    if rest == "sounds.json" {
        return Some(Merge::Sounds);
    }
    let dir = rest.strip_suffix(".json")?.split_once('/')?.0;
    match dir {
        "lang" => Some(Merge::Lang),
        "font" => Some(Merge::Font),
        "atlases" => Some(Merge::Atlas),
        "blockstates" => Some(Merge::BlockStates),
        _ => None,
    }
}

/// `SpriteSourceList.load` on one document: it keeps its own fields, and
/// `sources` gains `added` after its own entries. None when `sources` is
/// not a list.
pub fn atlas_append(
    doc: &mut serde_json::Map<String, serde_json::Value>,
    added: Vec<serde_json::Value>,
) -> Option<()> {
    let sources = doc
        .entry("sources".to_owned())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    sources.as_array_mut()?.extend(added);
    Some(())
}
/// The atlas file `base` (vanilla's own) plus `added` sources make, as a
/// stack of them loads: the base document whole with the added sources
/// after its own. `what` names the file in errors.
pub fn append_atlas_sources(
    base: Option<&str>,
    added: &[serde_json::Value],
    what: &str,
) -> Result<Vec<u8>, String> {
    let mut doc: serde_json::Map<String, serde_json::Value> = match base {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("{what}: {e}"))?,
        None => serde_json::Map::new(),
    };
    atlas_append(&mut doc, added.to_vec()).ok_or_else(|| format!("{what}: no sources"))?;
    serde_json::to_vec(&doc).map_err(|e| format!("{what}: {e}"))
}

/// `Pack.Position`: where a pack goes when it is added to a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Top,
    Bottom,
}
/// A pack as the selection sees it (`PackSelectionConfig` and its
/// compatibility).
#[derive(Clone, Debug, PartialEq)]
pub struct Selectable {
    pub id: String,
    pub required: bool,
    pub position: Position,
    pub fixed: bool,
    pub compatible: bool,
}
impl Selectable {
    /// A pack from `resourcepacks/` or a built-in optional one: not
    /// required, added on top, free to move (`FolderRepositorySource`).
    pub fn optional(id: &str, compatible: bool) -> Self {
        Self {
            id: id.to_owned(),
            required: false,
            position: Position::Top,
            fixed: false,
            compatible,
        }
    }
    /// The vanilla pack: required, at the bottom (`BuiltInPackSource`).
    pub fn vanilla() -> Self {
        Self {
            id: "vanilla".into(),
            required: true,
            position: Position::Bottom,
            fixed: false,
            compatible: true,
        }
    }
    /// A server's downloaded pack (`DownloadedPackSource.DOWNLOADED_PACK_SELECTION`):
    /// required, fixed at the top, so the pack screen cannot remove or reorder it.
    pub fn downloaded(id: &str, compatible: bool) -> Self {
        Self {
            id: id.to_owned(),
            required: true,
            position: Position::Top,
            fixed: true,
            compatible,
        }
    }
}

/// Ids from `DownloadedPackSource` (`server/…`); they must not go into
/// `options.txt` `resourcePacks`.
pub fn is_downloaded_pack_id(id: &str) -> bool {
    id.starts_with("server/")
}

/// The player's own selection: drop every downloaded server/world pack id.
pub fn player_pack_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| !is_downloaded_pack_id(id))
        .collect()
}

/// `Pack.Position.insert`: below or above the packs fixed at that end.
fn insert(list: &mut Vec<Selectable>, pack: Selectable, reverse: bool) {
    let own = pack.position;
    let side = match (own, reverse) {
        (Position::Top, false) | (Position::Bottom, true) => Position::Top,
        _ => Position::Bottom,
    };
    let pinned = |p: &Selectable| p.fixed && p.position == own;
    if side == Position::Bottom {
        let i = list.iter().take_while(|p| pinned(p)).count();
        list.insert(i, pack);
    } else {
        let i = list.len() - list.iter().rev().take_while(|p| pinned(p)).count();
        list.insert(i, pack);
    }
}

/// `PackSelectionModel` over a `PackRepository`: the packs not selected
/// (in id order) and the selected ones, top first, as the pack screen's two
/// lists show them.
#[derive(Clone, Debug, Default)]
pub struct SelectionModel {
    available: Vec<Selectable>,
    pub unselected: Vec<Selectable>,
    pub selected: Vec<Selectable>,
}
impl SelectionModel {
    /// `available` in any order; `selected` as options.txt lists them,
    /// bottom first.
    pub fn new(mut available: Vec<Selectable>, selected: &[String]) -> Self {
        // PackRepository.discoverAvailable: every source's packs go into one
        // TreeMap by id (String.compareTo), so `file/` packs sort among the
        // built-in ones.
        available.sort_by(|a, b| a.id.encode_utf16().cmp(b.id.encode_utf16()));
        available.dedup_by(|a, b| a.id == b.id);
        let mut m = Self {
            available,
            ..Self::default()
        };
        let mut chosen = m.rebuild(selected);
        chosen.reverse();
        m.unselected = m
            .available
            .iter()
            .filter(|p| !chosen.iter().any(|c| c.id == p.id))
            .cloned()
            .collect();
        m.selected = chosen;
        m
    }
    /// `PackRepository.rebuildSelected`: the named packs that exist, and
    /// every required one.
    fn rebuild(&self, ids: &[String]) -> Vec<Selectable> {
        let mut out: Vec<Selectable> = ids
            .iter()
            .filter_map(|id| self.available.iter().find(|p| p.id == *id).cloned())
            .collect();
        for p in &self.available {
            if p.required && !out.iter().any(|o| o.id == p.id) {
                insert(&mut out, p.clone(), false);
            }
        }
        out
    }
    fn list(&self, selected: bool) -> &Vec<Selectable> {
        if selected {
            &self.selected
        } else {
            &self.unselected
        }
    }
    pub fn can_select(&self, selected: bool, _: usize) -> bool {
        !selected
    }
    pub fn can_unselect(&self, selected: bool, i: usize) -> bool {
        selected && !self.selected[i].required
    }
    pub fn can_move_up(&self, selected: bool, i: usize) -> bool {
        i > 0 && !self.list(selected)[i - 1].fixed
    }
    pub fn can_move_down(&self, selected: bool, i: usize) -> bool {
        let list = self.list(selected);
        i + 1 < list.len() && !list[i + 1].fixed
    }
    /// `EntryBase.toggleSelection`: to the other list, at its default
    /// position (from the other end).
    fn toggle(&mut self, selected: bool, i: usize) {
        let (from, to) = if selected {
            (&mut self.selected, &mut self.unselected)
        } else {
            (&mut self.unselected, &mut self.selected)
        };
        let p = from.remove(i);
        insert(to, p, true);
    }
    /// `PackEntry`'s activation (a click on the icon, or the select key):
    /// select, an incompatible pack only once its confirmation was
    /// accepted, or else unselect. True when the lists changed.
    pub fn activate(&mut self, selected: bool, i: usize, confirmed: bool) -> bool {
        if self.can_select(selected, i) {
            if self.unselected[i].compatible || confirmed {
                self.toggle(false, i);
                return true;
            }
            false
        } else if self.can_unselect(selected, i) {
            self.toggle(true, i);
            true
        } else {
            false
        }
    }
    /// `moveUp` / `moveDown` when allowed.
    pub fn move_by(&mut self, selected: bool, i: usize, up: bool) -> bool {
        let ok = if up {
            self.can_move_up(selected, i)
        } else {
            self.can_move_down(selected, i)
        };
        if ok {
            let list = if selected {
                &mut self.selected
            } else {
                &mut self.unselected
            };
            let p = list.remove(i);
            list.insert(if up { i - 1 } else { i + 1 }, p);
        }
        ok
    }
    /// `commit`: the repository's selection, bottom first, as options.txt
    /// stores it (downloaded `server/…` packs stay out of that file).
    pub fn commit(&self) -> Vec<String> {
        let ids: Vec<String> = self.selected.iter().rev().map(|p| p.id.clone()).collect();
        player_pack_ids(self.rebuild(&ids).into_iter().map(|p| p.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FolderRepositorySource.discoverPacks` + `PackDetector.detectPackResources`:
    /// an entry that is a link, or holds one, is not a pack the game lists.
    #[cfg(unix)]
    #[test]
    fn pack_discovery_refuses_symlinks() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("sunrise-pack-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        let pack = |dir: &Path| {
            fs::create_dir_all(dir.join("assets/deep")).unwrap();
            fs::write(dir.join("pack.mcmeta"), meta).unwrap();
        };
        // A linked pack's target, a file a pack may hold a link to, and a link
        // whose own target is a link.
        pack(&base.join("outside/pack"));
        fs::create_dir_all(base.join("outside")).unwrap();
        fs::write(base.join("outside/file.txt"), "f").unwrap();
        symlink(base.join("outside/pack"), base.join("outside/alias")).unwrap();
        let packs = base.join("packs");
        for name in ["clean", "nested", "broken"] {
            pack(&packs.join(name));
        }
        symlink(
            base.join("outside/file.txt"),
            packs.join("nested/assets/deep/link"),
        )
        .unwrap();
        symlink(base.join("missing"), packs.join("broken/link")).unwrap();
        symlink(base.join("outside/pack"), packs.join("linked")).unwrap();
        symlink(base.join("outside/alias"), packs.join("chained")).unwrap();
        let ids = |validator: &DirectoryValidator| {
            let mut ids: Vec<String> = folder_meta_with(&packs, RESOURCE_FORMAT, validator)
                .into_iter()
                .map(|p| p.id)
                .collect();
            ids.extend(
                crate::datapack::discover_folder_with(&packs, validator)
                    .into_iter()
                    .map(|p| p.id),
            );
            ids
        };
        // Vanilla's default validator, with no `allowed_symlinks.txt`: no link.
        assert_eq!(
            ids(&DirectoryValidator::default()),
            ["file/clean", "file/clean"]
        );
        // An entry naming the linked pack's target lets that link, and only it,
        // through: the pack holding a link to a file, the broken link and the
        // link to a link stay out. A `[glob]` entry Sunrise cannot compile
        // allows nothing.
        let allow = base.join(ALLOWED_SYMLINKS_CONFIG_NAME);
        fs::write(
            &allow,
            format!(
                "# packs shared between game folders\n[glob]**/outside/**\n[prefix]{}\n[prefix]{}\n",
                base.join("outside/pack").display(),
                base.join("outside/alias").display()
            ),
        )
        .unwrap();
        assert_eq!(
            ids(&DirectoryValidator::parse(&allow)),
            ["file/clean", "file/linked", "file/clean", "file/linked"]
        );
        // An unsupported entry type throws in `ConfigEntry.parse`, so the whole
        // file allows no link.
        fs::write(
            &allow,
            format!(
                "[bogus]{}\n[prefix]{}\n",
                base.join("outside/pack").display(),
                base.join("outside/pack").display()
            ),
        )
        .unwrap();
        assert_eq!(
            ids(&DirectoryValidator::parse(&allow)),
            ["file/clean", "file/clean"]
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// `DownloadedPackSource.DOWNLOADED_PACK_SELECTION`: the pack is selected,
    /// fixed at the top, cannot be removed, and stays out of options.txt.
    #[test]
    fn a_downloaded_server_pack_is_fixed_selected_and_not_committed() {
        let server = "server/00000000000000000000000000000001/hash.zip";
        let available = vec![
            Selectable::optional("file/Mine", true),
            Selectable::downloaded(server, true),
            Selectable::vanilla(),
        ];
        let model = SelectionModel::new(available, &["file/Mine".into()]);
        assert_eq!(
            model
                .selected
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            [server, "file/Mine", "vanilla"]
        );
        assert!(!model.can_unselect(true, 0));
        // A neighbour cannot move up past the fixed server pack.
        assert!(!model.can_move_up(true, 1));
        assert_eq!(
            model.commit(),
            vec!["vanilla".to_owned(), "file/Mine".to_owned()]
        );
        assert!(is_downloaded_pack_id(server));
        assert!(!is_downloaded_pack_id("file/Mine"));
    }

    #[test]
    fn a_comment_after_the_metadata_does_not_hide_a_pack() {
        let meta = br#"{"pack":{"description":"d","pack_format":97}}
// Signed by the pack's author"#;
        assert!(parse_meta(meta).is_some());
        assert!(parse_meta(b"not json").is_none());
    }

    #[test]
    fn mcmeta_with_utf8_bom_is_read() {
        let dir = std::env::temp_dir().join(format!("sunrise-bom-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut meta = b"\xEF\xBB\xBF".to_vec();
        meta.extend_from_slice(br#"{"pack":{"description":"d","pack_format":97}}"#);
        fs::write(dir.join("pack.mcmeta"), &meta).unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_some());
        // Without a BOM the same JSON still reads; other leading bytes still fail.
        fs::write(dir.join("pack.mcmeta"), &meta[3..]).unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_some());
        fs::write(
            dir.join("pack.mcmeta"),
            [b"\xEF\xBB".as_slice(), &meta[3..]].concat(),
        )
        .unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_none());
        fs::remove_dir_all(&dir).ok();
    }
    /// `PackFormat.IntermediaryFormat.validate` and its three helpers, with
    /// the resource pack's last pre-minor version, 64.
    #[test]
    fn pack_format_validation() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let check = |s: &str| validate_pack_range(&v(s), 64);
        // validate: both min_format and max_format, or neither.
        assert!(
            check(r#"{"min_format":97}"#)
                .unwrap_err()
                .contains("both min_format and max_format")
        );
        assert!(check(r#"{"max_format":97}"#).is_err());
        assert!(check("{}").unwrap_err().contains("missing format version"));
        // A pack_format alone: fine up to 64, not above.
        assert_eq!(check(r#"{"pack_format":34}"#), Ok(((34, 0), (34, 0))));
        assert!(
            check(r#"{"pack_format":97}"#)
                .unwrap_err()
                .contains("newer than 64")
        );
        // validateNewFormat: min above max, the deprecated key above 64.
        assert!(
            check(r#"{"min_format":98,"max_format":97}"#)
                .unwrap_err()
                .contains("greater than max_format")
        );
        assert!(check(r#"{"min_format":[97,2],"max_format":[97,1]}"#).is_err());
        assert!(
            check(r#"{"min_format":97,"max_format":98,"supported_formats":[97,98]}"#)
                .unwrap_err()
                .contains("deprecated")
        );
        // A decimal reads as its whole part, in a list or alone: `[97.1]` is format 97 (a pack
        // that meant `[97, 1]` still loads and counts as compatible).
        assert_eq!(
            check(r#"{"min_format":[97.1],"max_format":[97.1]}"#),
            Ok(((97, 0), (97, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":97.9,"max_format":98.5}"#),
            Ok(((97, 0), (98, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":97,"max_format":98}"#),
            Ok(((97, 0), (98, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":[97,0],"max_format":[97,1]}"#),
            Ok(((97, 0), (97, 1)))
        );
        // validatePackFormatForRange: a pack_format outside the range.
        assert!(
            check(r#"{"min_format":97,"max_format":98,"pack_format":99}"#)
                .unwrap_err()
                .contains("declared main format is 99")
        );
        assert!(check(r#"{"min_format":97,"max_format":98,"pack_format":98}"#).is_ok());
        // Below 65 the old fields are required and must agree.
        assert!(
            check(r#"{"min_format":34,"max_format":70}"#)
                .unwrap_err()
                .contains("require a supported_formats")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[35,64],"pack_format":35}"#
            )
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,60],"pack_format":35}"#
            )
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(
            check(r#"{"min_format":34,"max_format":70,"supported_formats":[34,64]}"#)
                .unwrap_err()
                .contains("require a pack_format")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,64],"pack_format":33}"#
            )
            .is_err()
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,64],"pack_format":34}"#
            )
            .is_ok()
        );
        // Multi-version packs below 15 cannot work.
        assert!(
            check(
                r#"{"min_format":10,"max_format":70,"supported_formats":[10,64],"pack_format":10}"#
            )
            .unwrap_err()
            .contains("less than 15")
        );
        // validateOldFormat: a range above 64, or no pack_format.
        assert!(
            check(r#"{"supported_formats":[34,70],"pack_format":34}"#)
                .unwrap_err()
                .contains("newer than 64")
        );
        assert!(
            check(r#"{"supported_formats":[34,60]}"#)
                .unwrap_err()
                .contains("require a pack_format")
        );
        assert_eq!(
            check(r#"{"supported_formats":[34,60],"pack_format":40}"#),
            Ok(((34, 0), (60, 0)))
        );
        assert!(
            check(
                r#"{"supported_formats":{"min_inclusive":34,"max_inclusive":60},"pack_format":40}"#
            )
            .is_ok()
        );
        assert!(check(r#"{"supported_formats":[60,34],"pack_format":40}"#).is_err());
        // A refused pack is of unknown format.
        assert_eq!(
            compatibility(&v(r#"{"pack":{"pack_format":97}}"#)),
            Compat::Unknown
        );
        assert_eq!(
            compatibility(&v(
                r#"{"pack":{"min_format":97,"max_format":98,"pack_format":99}}"#
            )),
            Compat::Unknown
        );
    }
    #[test]
    fn compatibility_ranges() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert!(compatible(&v(
            r#"{"pack":{"min_format":97,"max_format":98}}"#
        )));
        assert!(compatible(&v(
            r#"{"pack":{"min_format":[97,0],"max_format":[97,1]}}"#
        )));
        assert!(!compatible(&v(
            r#"{"pack":{"min_format":[97,2],"max_format":99}}"#
        )));
        assert!(!compatible(&v(r#"{"pack":{"pack_format":34}}"#)));
        assert_eq!(
            component(&v(r#"["a",{"text":"b","color":"red"}]"#)).plain(),
            "ab"
        );
    }
}
