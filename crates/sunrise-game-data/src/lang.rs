//! Languages (ClientLanguage / LanguageManager): `lang/<code>.json` from the
//! resource root and enabled packs layered over `en_us`, the language list
//! from `pack.mcmeta` `language` sections, and a process-wide current
//! language used by every screen.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        Arc, LazyLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageInfo {
    pub code: String,
    pub name: String,
    pub region: String,
    pub bidirectional: bool,
}
impl LanguageInfo {
    /// LanguageInfo.toComponent: "Name (Region)".
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.region)
    }
}
#[derive(Default, Clone)]
pub struct Language {
    pub code: Arc<str>,
    /// `LanguageInfo.bidirectional`: text without a strong character reads
    /// right to left (`ClientLanguage.defaultRightToLeft`).
    bidirectional: bool,
    /// The dedicated server never installs the client keybind resolver.
    pub(crate) literal_keybinds: bool,
    map: HashMap<String, String>,
    components: HashMap<String, serde_json::Value>,
    /// This language as vanilla's client loads it: the same sources and order without Sunrise's
    /// own (the engine's `sunrise` namespace and the crates'), built once with it. `None` when
    /// it has no strings of Sunrise's own to leave out.
    vanilla: Option<Arc<Language>>,
    /// What the mods' run-time entries ([`set_mod_entries`]) replaced: each key's value from the
    /// packs, `None` for a key the packs don't have. Put back before entries are applied again.
    shadowed: HashMap<String, Option<String>>,
}
pub const DEFAULT: &str = "en_us";

#[derive(Default)]
struct ModEntries {
    entries: BTreeMap<String, BTreeMap<String, HashMap<String, String>>>,
    disabled: BTreeSet<String>,
}

/// Entries mods add at run time from their own data (a quest book's language files under
/// `config/`), plus the currently disabled native mod ids: mod id, then language code, then key
/// and text. Applied over the packs' entries of every language [`set`] publishes, so they survive
/// a resource reload. Always accessed while holding [`CURRENT`]'s write lock.
static MOD_ENTRIES: RwLock<ModEntries> = RwLock::new(ModEntries {
    entries: BTreeMap::new(),
    disabled: BTreeSet::new(),
});

/// Puts back what earlier mod entries replaced in `language`, then applies each enabled mod's
/// entries in id order: `en_us` immediately before the selected language, so a later mod id wins
/// across locales while a missing key still falls back to that mod's `en_us`.
fn apply_mod_entries(language: &mut Language, mods: &ModEntries) {
    for (key, value) in std::mem::take(&mut language.shadowed) {
        match value {
            Some(value) => language.map.insert(key, value),
            None => language.map.remove(&key),
        };
    }
    let codes: &[&str] = if &*language.code == DEFAULT {
        &[DEFAULT]
    } else {
        &[DEFAULT, &language.code]
    };
    for (mod_id, by_code) in &mods.entries {
        if mods.disabled.contains(mod_id) {
            continue;
        }
        for code in codes {
            for (key, value) in by_code.get(*code).into_iter().flatten() {
                let before = language.map.insert(key.clone(), value.clone());
                language.shadowed.entry(key.clone()).or_insert(before);
            }
        }
    }
}

fn commit_language(slot: &mut Option<Arc<Language>>, mods: &ModEntries, mut language: Language) {
    apply_mod_entries(&mut language, mods);
    *slot = Some(Arc::new(language));
}

fn republish_locked(slot: &mut Option<Arc<Language>>, mods: &ModEntries) -> bool {
    let Some(current) = slot.as_deref() else {
        return false;
    };
    let language = current.clone();
    commit_language(slot, mods, language);
    true
}

fn update_publication<R>(
    f: impl FnOnce(&mut Option<Arc<Language>>, &mut ModEntries) -> (R, bool),
) -> R {
    #[cfg(any(test, feature = "test-language-publication"))]
    let f = {
        let mut f = Some(f);
        let isolated = PUBLICATION.with(|slot| {
            let mut slot = slot.borrow_mut();
            slot.as_mut().map(|isolated| {
                let f = f.take().expect("called once");
                f(&mut isolated.current, &mut isolated.mods)
            })
        });
        if let Some((result, published)) = isolated {
            if published {
                scope_changed();
            }
            return result;
        }
        f.expect("no isolated publication, so not called yet")
    };
    let mut current = CURRENT.write().unwrap_or_else(|e| e.into_inner());
    let mut mods = MOD_ENTRIES.write().unwrap_or_else(|e| e.into_inner());
    let (result, published) = f(&mut current, &mut mods);
    if published {
        if let Some(language) = current.as_deref() {
            BIDIRECTIONAL.store(language.bidirectional, Ordering::Relaxed);
            HAS_MOD_OVERLAY.store(!language.shadowed.is_empty(), Ordering::Release);
        }
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    result
}

/// Replaces mod `mod_id`'s run-time entries for language `code` with `entries` (an empty map drops
/// them) and publishes the current language again with every mod's entries, so text looks them
/// up at once. Returns how many entries `mod_id` now has for `code`.
pub fn set_mod_entries(mod_id: &str, code: &str, entries: HashMap<String, String>) -> usize {
    let entries: HashMap<String, String> = entries
        .into_iter()
        .map(|(key, mut value)| {
            normalize_numeric_placeholders(&mut value);
            (key, value)
        })
        .collect();
    let count = entries.len();
    update_publication(|current, mods| {
        let changed = if entries.is_empty() {
            if let Some(by_code) = mods.entries.get_mut(mod_id) {
                let removed = by_code.remove(code).is_some();
                if by_code.is_empty() {
                    mods.entries.remove(mod_id);
                }
                removed
            } else {
                false
            }
        } else {
            mods.entries
                .entry(mod_id.to_owned())
                .or_default()
                .insert(code.to_owned(), entries);
            true
        };
        let published =
            changed && !mods.disabled.contains(mod_id) && republish_locked(current, mods);
        (count, published)
    })
}

/// Drops all of mod `mod_id`'s run-time entries, in every language, and publishes again.
pub fn clear_mod_entries(mod_id: &str) {
    update_publication(|current, mods| {
        let removed = mods.entries.remove(mod_id).is_some();
        let published =
            removed && !mods.disabled.contains(mod_id) && republish_locked(current, mods);
        ((), published)
    });
}

/// Updates which native mods' run-time entries are disabled, keeping their stored entries so
/// re-enabling restores them without rerunning startup. Republishes once if any owner with
/// entries changed enabled state.
pub fn set_disabled_mod_entries(disabled: &BTreeSet<String>) {
    update_publication(|current, mods| {
        let changed = mods
            .entries
            .keys()
            .any(|id| mods.disabled.contains(id) != disabled.contains(id));
        mods.disabled.clone_from(disabled);
        let published = changed && republish_locked(current, mods);
        ((), published)
    });
}

static CURRENT: RwLock<Option<Arc<Language>>> = RwLock::new(None);
/// A language that only the calling thread sees, and the keys it was asked for and did not have
/// (`scoped`).
struct Scope {
    language: Arc<Language>,
    missed: Vec<String>,
}
thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}
/// The current language's `bidirectional`, read for every drawn string.
static BIDIRECTIONAL: AtomicBool = AtomicBool::new(false);
/// Whether the current published language carries any active run-time mod overrides.
static HAS_MOD_OVERLAY: AtomicBool = AtomicBool::new(false);
/// Counts the languages set (`set`): another count is another language, so text looked up
/// before it may read otherwise.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
thread_local! {
    /// Counts the scopes this thread entered and left (`scoped`), which no other thread sees.
    static SCOPE_GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
/// Which language text looks up in now, on this thread, as a count that changes whenever that
/// can: both counts only grow, so their sum changes when either does.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed) + SCOPE_GENERATION.get()
}
fn scope_changed() {
    SCOPE_GENERATION.set(SCOPE_GENERATION.get() + 1);
}

/// A source of resource files: the prepared root plus packs, highest priority last.
pub trait ResourceSource {
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Whether `path` is there, without reading it where the source can tell.
    fn exists(&self, path: &str) -> bool {
        self.read(path).is_some()
    }
    /// The namespaces under `assets/` (resource packs may add their own).
    fn namespaces(&self) -> Vec<String> {
        vec!["minecraft".into()]
    }
}
/// The bundled file a language reads, and whether it drops `ß`: a regional variant reads its
/// language's file (Swiss German writes "ss" for "ß", as vanilla's de_ch does).
fn bundled_code(code: &str) -> (&str, bool) {
    match code {
        "ko" => ("ko_kr", false),
        "hr" => ("hr_hr", false),
        "de_at" => ("de_de", false),
        "de_ch" => ("de_de", true),
        "fr_ca" | "fr_ch" => ("fr_fr", false),
        "es_ar" | "es_cl" | "es_ec" | "es_mx" | "es_uy" | "es_ve" => ("es_es", false),
        code => (code, false),
    }
}

/// Lang files crates above the engine ship (`sunrise_hooks::lang`), each a list of
/// (code, JSON) pairs. The game's entry installs them once, before any language loads.
static CRATE_FILES: std::sync::OnceLock<&'static [&'static [(&'static str, &'static str)]]> =
    std::sync::OnceLock::new();

/// Installs the crates' lang files (`sunrise_hooks::lang::engine::all()`). Only the first call
/// counts.
pub fn install_crate_files(files: &'static [&'static [(&'static str, &'static str)]]) {
    let _ = CRATE_FILES.set(files);
}

/// The crates' entries for language `code` (through [`bundled_code`]'s variants) that the
/// engine's own `sunrise` file for it doesn't have: on a key both have, the engine wins.
fn read_crate_files(files: &[&[(&str, &str)]], code: &str, map: &mut HashMap<String, String>) {
    if files.is_empty() {
        return;
    }
    let (file, swiss) = bundled_code(code);
    let mut own = HashMap::new();
    if let Some(bytes) = Bundled.read(&format!("assets/sunrise/lang/{code}.json")) {
        read_lang_file(&bytes, &mut own);
    }
    for (c, json) in files.iter().flat_map(|f| f.iter()) {
        if *c != file {
            continue;
        }
        let mut theirs = HashMap::new();
        if swiss {
            read_lang_file(json.replace('ß', "ss").as_bytes(), &mut theirs);
        } else {
            read_lang_file(json.as_bytes(), &mut theirs);
        }
        for (key, value) in theirs {
            if !own.contains_key(&key) {
                map.insert(key, value);
            }
        }
    }
}

/// Sunrise's own strings, compiled in so they need no prepared resources.
struct Bundled;
impl ResourceSource for Bundled {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let code = path
            .strip_prefix("assets/sunrise/lang/")?
            .strip_suffix(".json")?;
        let (file, swiss) = bundled_code(code);
        let path = format!("assets/sunrise/lang/{file}.json");
        let body = match path.as_str() {
            "assets/sunrise/lang/en_us.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/en_us.json")
            }
            "assets/sunrise/lang/zh_cn.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/zh_cn.json")
            }
            "assets/sunrise/lang/ru_ru.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ru_ru.json")
            }
            "assets/sunrise/lang/pl_pl.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/pl_pl.json")
            }
            "assets/sunrise/lang/uk_ua.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/uk_ua.json")
            }
            "assets/sunrise/lang/hi_in.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/hi_in.json")
            }
            "assets/sunrise/lang/id_id.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/id_id.json")
            }
            "assets/sunrise/lang/tr_tr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/tr_tr.json")
            }
            "assets/sunrise/lang/ko_kr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ko_kr.json")
            }
            "assets/sunrise/lang/hr_hr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/hr_hr.json")
            }
            "assets/sunrise/lang/pt_pt.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/pt_pt.json")
            }
            "assets/sunrise/lang/ur_pk.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ur_pk.json")
            }
            "assets/sunrise/lang/fa_ir.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fa_ir.json")
            }
            "assets/sunrise/lang/he_il.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/he_il.json")
            }
            "assets/sunrise/lang/hu_hu.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/hu_hu.json")
            }
            "assets/sunrise/lang/ro_ro.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ro_ro.json")
            }
            "assets/sunrise/lang/da_dk.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/da_dk.json")
            }
            "assets/sunrise/lang/fi_fi.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fi_fi.json")
            }
            "assets/sunrise/lang/no_no.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/no_no.json")
            }
            "assets/sunrise/lang/bg_bg.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/bg_bg.json")
            }
            "assets/sunrise/lang/sk_sk.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sk_sk.json")
            }
            "assets/sunrise/lang/lt_lt.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/lt_lt.json")
            }
            "assets/sunrise/lang/it_it.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/it_it.json")
            }
            "assets/sunrise/lang/nl_nl.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/nl_nl.json")
            }
            "assets/sunrise/lang/vi_vn.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/vi_vn.json")
            }
            "assets/sunrise/lang/th_th.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/th_th.json")
            }
            "assets/sunrise/lang/ja_jp.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ja_jp.json")
            }
            "assets/sunrise/lang/ar_sa.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ar_sa.json")
            }
            "assets/sunrise/lang/pt_br.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/pt_br.json")
            }
            "assets/sunrise/lang/cs_cz.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/cs_cz.json")
            }
            "assets/sunrise/lang/sv_se.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sv_se.json")
            }
            "assets/sunrise/lang/el_gr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/el_gr.json")
            }
            "assets/sunrise/lang/ca_es.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ca_es.json")
            }
            "assets/sunrise/lang/et_ee.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/et_ee.json")
            }
            "assets/sunrise/lang/lv_lv.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/lv_lv.json")
            }
            "assets/sunrise/lang/sl_si.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sl_si.json")
            }
            "assets/sunrise/lang/sr_cs.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sr_cs.json")
            }
            "assets/sunrise/lang/is_is.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/is_is.json")
            }
            "assets/sunrise/lang/af_za.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/af_za.json")
            }
            "assets/sunrise/lang/ms_my.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ms_my.json")
            }
            "assets/sunrise/lang/be_by.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/be_by.json")
            }
            "assets/sunrise/lang/sr_sp.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sr_sp.json")
            }
            "assets/sunrise/lang/az_az.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/az_az.json")
            }
            "assets/sunrise/lang/uz_uz.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/uz_uz.json")
            }
            "assets/sunrise/lang/kk_kz.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/kk_kz.json")
            }
            "assets/sunrise/lang/ka_ge.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ka_ge.json")
            }
            "assets/sunrise/lang/hy_am.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/hy_am.json")
            }
            "assets/sunrise/lang/bs_ba.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/bs_ba.json")
            }
            "assets/sunrise/lang/mk_mk.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/mk_mk.json")
            }
            "assets/sunrise/lang/sq_al.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sq_al.json")
            }
            "assets/sunrise/lang/cy_gb.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/cy_gb.json")
            }
            "assets/sunrise/lang/ga_ie.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ga_ie.json")
            }
            "assets/sunrise/lang/eu_es.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/eu_es.json")
            }
            "assets/sunrise/lang/gl_es.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/gl_es.json")
            }
            "assets/sunrise/lang/fil_ph.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fil_ph.json")
            }
            "assets/sunrise/lang/ta_in.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ta_in.json")
            }
            "assets/sunrise/lang/mn_mn.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/mn_mn.json")
            }
            "assets/sunrise/lang/lo_la.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/lo_la.json")
            }
            "assets/sunrise/lang/so_so.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/so_so.json")
            }
            "assets/sunrise/lang/fy_nl.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fy_nl.json")
            }
            "assets/sunrise/lang/gd_gb.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/gd_gb.json")
            }
            "assets/sunrise/lang/lb_lu.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/lb_lu.json")
            }
            "assets/sunrise/lang/ky_kg.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ky_kg.json")
            }
            "assets/sunrise/lang/kn_in.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/kn_in.json")
            }
            "assets/sunrise/lang/mt_mt.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/mt_mt.json")
            }
            "assets/sunrise/lang/eo_uy.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/eo_uy.json")
            }
            "assets/sunrise/lang/haw_us.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/haw_us.json")
            }
            "assets/sunrise/lang/ig_ng.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ig_ng.json")
            }
            "assets/sunrise/lang/la_la.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/la_la.json")
            }
            "assets/sunrise/lang/tt_ru.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/tt_ru.json")
            }
            "assets/sunrise/lang/yi_de.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/yi_de.json")
            }
            "assets/sunrise/lang/yo_ng.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/yo_ng.json")
            }
            "assets/sunrise/lang/ba_ru.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/ba_ru.json")
            }
            "assets/sunrise/lang/br_fr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/br_fr.json")
            }
            "assets/sunrise/lang/cv_cu.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/cv_cu.json")
            }
            "assets/sunrise/lang/fo_fo.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fo_fo.json")
            }
            "assets/sunrise/lang/fur_it.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fur_it.json")
            }
            "assets/sunrise/lang/li_li.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/li_li.json")
            }
            "assets/sunrise/lang/lmo.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/lmo.json")
            }
            "assets/sunrise/lang/oc_fr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/oc_fr.json")
            }
            "assets/sunrise/lang/sah_sah.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/sah_sah.json")
            }
            "assets/sunrise/lang/se_no.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/se_no.json")
            }
            "assets/sunrise/lang/szl.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/szl.json")
            }
            "assets/sunrise/lang/vec_it.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/vec_it.json")
            }
            "assets/sunrise/lang/de_de.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/de_de.json")
            }
            "assets/sunrise/lang/fr_fr.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/fr_fr.json")
            }
            "assets/sunrise/lang/es_es.json" => {
                include_str!("../../sunrise-client/assets/sunrise/lang/es_es.json")
            }
            _ => return None,
        };
        if swiss {
            return Some(body.replace('ß', "ss").into_bytes());
        }
        Some(body.as_bytes().to_vec())
    }
    fn namespaces(&self) -> Vec<String> {
        vec!["sunrise".into()]
    }
}
impl ResourceSource for sunrise_assets::resource_source::ResourceSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.read(path).ok()
    }
    fn exists(&self, path: &str) -> bool {
        self.contains(path)
    }
    fn namespaces(&self) -> Vec<String> {
        self.namespaces("assets")
    }
}
impl ResourceSource for std::path::PathBuf {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.join(path)).ok()
    }
    fn exists(&self, path: &str) -> bool {
        self.join(path).is_file()
    }
    fn namespaces(&self) -> Vec<String> {
        std::fs::read_dir(self.join("assets"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .collect()
    }
}

/// DeprecatedTranslationsInfo.applyToMap: drop removed keys, then move each
/// renamed key's text to its new key (a missing old key removes the new one).
pub fn apply_deprecated<T>(map: &mut HashMap<String, T>, json: &[u8]) {
    let Ok(info) = serde_json::from_slice::<serde_json::Value>(json) else {
        return;
    };
    for key in info["removed"].as_array().into_iter().flatten() {
        if let Some(k) = key.as_str() {
            map.remove(k);
        }
    }
    for (from, to) in info["renamed"].as_object().into_iter().flatten() {
        let Some(to) = to.as_str() else { continue };
        match map.remove(from) {
            Some(value) => {
                map.insert(to.to_owned(), value);
            }
            None => {
                map.remove(to);
            }
        }
    }
}

/// `Language.loadFromJson` for one `lang/<code>.json`: its entries go into `map` in file order.
/// A file that is plain JSON reads as before; any other goes through Gson's lenient reader
/// (`#`, `//` and `/* */` comments, unquoted or single-quoted names and values, raw control
/// characters in strings, `=` or `=>` and `;`), which vanilla reads lang files with (expected
/// from `Language.loadFromJson`, Gson). A file that is not JSON even so adds nothing; a value
/// that is not a string, number or boolean (`GsonHelper.convertToString`) ends the file there.
pub fn read_lang_file(bytes: &[u8], map: &mut HashMap<String, String>) {
    if let Ok(entries) = serde_json::from_slice::<HashMap<String, String>>(bytes) {
        map.extend(entries.into_iter().map(|(key, mut value)| {
            normalize_numeric_placeholders(&mut value);
            (key, value)
        }));
        return;
    }
    if let Some(entries) = lang_entries(bytes) {
        for (key, value) in entries {
            let Some(value) = lang_primitive(value) else {
                break;
            };
            map.insert(String::from_utf16_lossy(&key), value);
        }
    }
}

fn lang_entries(bytes: &[u8]) -> Option<Vec<(Vec<u16>, sunrise_world::datafix::json::Json)>> {
    use sunrise_world::datafix::json::{Json, parse_lenient};
    let text = String::from_utf8_lossy(bytes);
    // Gson's JsonReader skips a leading byte order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match parse_lenient(&text.encode_utf16().collect::<Vec<_>>())? {
        Json::Obj(entries) => Some(entries),
        _ => None,
    }
}

fn lang_primitive(value: sunrise_world::datafix::json::Json) -> Option<String> {
    use sunrise_world::datafix::json::Json;
    let mut text = match value {
        Json::Str(s) => String::from_utf16_lossy(&s),
        Json::Num(n) => n,
        Json::Bool(b) => b.to_string(),
        _ => return None,
    };
    normalize_numeric_placeholders(&mut text);
    Some(text)
}

/// `Language.loadFromJson` treats numeric placeholders as strings:
/// `%[n$][digits or dots][df]` becomes `%[n$]s`.
fn normalize_numeric_placeholders(template: &mut String) {
    let mut out = String::new();
    let mut start = 0;
    for (at, _) in template.match_indices('%') {
        let tail = &template[at + 1..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let index = if digits != 0 && tail.as_bytes().get(digits) == Some(&b'$') {
            digits + 1
        } else {
            0
        };
        let rest = tail[index..].trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        if rest.starts_with(['d', 'f']) {
            out.push_str(&template[start..at + 1 + index]);
            out.push('s');
            start = template.len() - rest.len() + 1;
        }
    }
    if start != 0 {
        out.push_str(&template[start..]);
        *template = out;
    }
}

impl Language {
    /// A language holding only `entries` (key, template), as `code`: the part of the current
    /// language a text needs, carried where no language is loaded.
    pub fn of(code: &str, entries: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            code: code.into(),
            map: entries.into_iter().collect(),
            ..Self::default()
        }
    }

    /// `Language.loadDefault`: built-in English from the unlayered vanilla root
    /// and the platform's bundles, with vanilla's deprecated keys applied.
    pub fn load_default(source: &dyn ResourceSource) -> Self {
        Self::load_server(source, std::iter::empty(), |_| None)
    }

    /// The dedicated server's English at startup. Namespaces load in lexical
    /// order; within a namespace later packs win, until the first invalid file.
    /// The caller supplies the shared component codec for object and array values.
    pub fn load_server<'a>(
        vanilla: &dyn ResourceSource,
        packs: impl IntoIterator<Item = &'a dyn ResourceSource>,
        decode: impl Fn(&serde_json::Value) -> Option<serde_json::Value>,
    ) -> Self {
        let mut language = Self {
            code: DEFAULT.into(),
            literal_keybinds: true,
            ..Self::default()
        };
        if let Some(bytes) = vanilla.read("assets/minecraft/lang/en_us.json") {
            read_lang_file(&bytes, &mut language.map);
        }
        if let Some(bytes) = Bundled.read("assets/sunrise/lang/en_us.json") {
            read_lang_file(&bytes, &mut language.map);
        }
        read_crate_files(
            CRATE_FILES.get().copied().unwrap_or_default(),
            DEFAULT,
            &mut language.map,
        );
        let packs: Vec<_> = packs.into_iter().collect();
        let namespaces: std::collections::BTreeSet<_> =
            packs.iter().flat_map(|pack| pack.namespaces()).collect();
        for namespace in namespaces {
            let path = format!("assets/{namespace}/lang/en_us.json");
            for bytes in packs.iter().filter_map(|pack| pack.read(&path)) {
                if !language.overlay(&bytes, &decode) {
                    log::warn!(
                        "Skipped language file {path} and remaining layers in its namespace"
                    );
                    break;
                }
            }
        }
        if let Some(bytes) = vanilla.read("assets/minecraft/lang/deprecated.json") {
            language.deprecate(&bytes);
        }
        language
    }

    fn overlay(
        &mut self,
        bytes: &[u8],
        decode: &impl Fn(&serde_json::Value) -> Option<serde_json::Value>,
    ) -> bool {
        use sunrise_world::datafix::json::{Json, to_stable_string};
        let Some(entries) = lang_entries(bytes) else {
            return false;
        };
        for (key, value) in entries {
            let key = String::from_utf16_lossy(&key);
            if matches!(value, Json::Obj(_) | Json::Arr(_)) {
                let json = String::from_utf16_lossy(&to_stable_string(&value));
                let Some(component) = serde_json::from_str(&json)
                    .ok()
                    .and_then(|value| decode(&value))
                else {
                    return false;
                };
                self.map.remove(&key);
                self.components.insert(key, component);
            } else if let Some(template) = lang_primitive(value) {
                self.components.remove(&key);
                self.map.insert(key, template);
            } else {
                return false;
            }
        }
        true
    }

    fn deprecate(&mut self, bytes: &[u8]) {
        apply_deprecated(&mut self.map, bytes);
        apply_deprecated(&mut self.components, bytes);
    }

    pub(crate) fn component(&self, key: &str) -> Option<&serde_json::Value> {
        self.components.get(key)
    }

    /// ClientLanguage.loadFrom: en_us first, then the selected language, each
    /// from every namespace and every source in priority order (later sources
    /// override).
    pub fn load(sources: &[&dyn ResourceSource], code: &str) -> Self {
        let crate_files = CRATE_FILES.get().copied().unwrap_or_default();
        Self::load_with(sources, code, crate_files)
    }

    /// [`Language::load`] with these crates' lang files.
    fn load_with(
        sources: &[&dyn ResourceSource],
        code: &str,
        crate_files: &[&[(&str, &str)]],
    ) -> Self {
        let mut map = HashMap::new();
        // What vanilla's client would load from the same sources: everything but the engine's.
        let mut vanilla = HashMap::new();
        let mut engine_keys = false;
        let mut codes = vec![DEFAULT];
        if code != DEFAULT {
            codes.push(code);
        }
        // Sunrise's own strings first, so a pack or the prepared root can still
        // override them. Vanilla has no key for "Controller vibration".
        let bundled = Bundled;
        let mut all: Vec<&dyn ResourceSource> = vec![&bundled];
        all.extend_from_slice(sources);
        let sources = all.as_slice();
        let namespaces: std::collections::BTreeSet<String> =
            sources.iter().flat_map(|s| s.namespaces()).collect();
        for c in codes {
            for namespace in &namespaces {
                for (i, source) in sources.iter().enumerate() {
                    let engine = i == 0 && namespace == "sunrise";
                    if let Some(bytes) = source.read(&format!("assets/{namespace}/lang/{c}.json")) {
                        if engine {
                            engine_keys = true;
                        } else {
                            read_lang_file(&bytes, &mut vanilla);
                        }
                        read_lang_file(&bytes, &mut map);
                    }
                    // The crates' text right after the engine's own (`Bundled`, the first source).
                    if engine {
                        let before = map.len();
                        read_crate_files(crate_files, c, &mut map);
                        engine_keys |= map.len() != before;
                    }
                }
            }
        }
        // DeprecatedTranslationsInfo.loadFromDefaultResource().applyToMap: the
        // game's own deprecated.json, not a resource pack's.
        if let Some(bytes) = sources
            .iter()
            .find_map(|s| s.read("assets/minecraft/lang/deprecated.json"))
        {
            apply_deprecated(&mut map, &bytes);
            apply_deprecated(&mut vanilla, &bytes);
        }
        let bidirectional = available(sources)
            .iter()
            .any(|info| info.code == code && info.bidirectional);
        let code: Arc<str> = code.into();
        let vanilla = engine_keys.then(|| {
            Arc::new(Self {
                code: Arc::clone(&code),
                bidirectional,
                map: vanilla,
                ..Self::default()
            })
        });
        Self {
            code,
            bidirectional,
            map,
            vanilla,
            ..Self::default()
        }
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
    /// The resolved map, for `sunrise_protocol::text::plain`: a server's
    /// components carry keys, and the network layer resolves them with this.
    pub fn map(&self) -> &HashMap<String, String> {
        &self.map
    }
    /// The resolved map without any run-time mod overrides: what a network session captures so
    /// clearing or disabling a mod later falls back to the packs' values, not a stale override.
    pub fn pack_map(&self) -> HashMap<String, String> {
        let mut map = self.map.clone();
        for (key, value) in &self.shadowed {
            match value {
                Some(value) => {
                    map.insert(key.clone(), value.clone());
                }
                None => {
                    map.remove(key);
                }
            }
        }
        map
    }
    /// The active run-time mod override for `key`, if an enabled mod supplied one on top of the
    /// packs' entries.
    pub fn mod_entry(&self, key: &str) -> Option<&str> {
        if self.shadowed.contains_key(key) {
            self.map.get(key).map(String::as_str)
        } else {
            None
        }
    }
    pub fn len(&self) -> usize {
        self.map.len() + self.components.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty() && self.components.is_empty()
    }
}

/// Languages declared by `pack.mcmeta` files (the vanilla asset root first).
pub fn available(sources: &[&dyn ResourceSource]) -> Vec<LanguageInfo> {
    let mut found: BTreeMap<String, LanguageInfo> = BTreeMap::new();
    found.insert(
        DEFAULT.into(),
        LanguageInfo {
            code: DEFAULT.into(),
            name: "English".into(),
            region: "US".into(),
            bidirectional: false,
        },
    );
    for source in sources {
        for path in ["assets/pack.mcmeta", "pack.mcmeta"] {
            let Some(bytes) = source.read(path) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            for (code, info) in value["language"].as_object().into_iter().flatten() {
                // Only languages whose file exists somewhere are selectable.
                let file = format!("assets/minecraft/lang/{code}.json");
                if code != DEFAULT && !sources.iter().any(|s| s.exists(&file)) {
                    continue;
                }
                found.insert(
                    code.clone(),
                    LanguageInfo {
                        code: code.clone(),
                        name: info["name"].as_str().unwrap_or(code).to_owned(),
                        region: info["region"].as_str().unwrap_or("").to_owned(),
                        bidirectional: info["bidirectional"].as_bool().unwrap_or(false),
                    },
                );
            }
        }
    }
    found.into_values().collect()
}

#[cfg(any(test, feature = "test-language-publication"))]
struct IsolatedPublication {
    current: Option<Arc<Language>>,
    mods: ModEntries,
}

#[cfg(any(test, feature = "test-language-publication"))]
thread_local! {
    static PUBLICATION: RefCell<Option<IsolatedPublication>> = const { RefCell::new(None) };
}

/// Isolate language reload publication on this fixture thread, including nested scopes.
/// Unlike `scoped`, subsequent `set` calls replace the visible language.
#[cfg(any(test, feature = "test-language-publication"))]
pub fn isolate_publication(language: Language) -> impl Drop {
    struct Restore(
        Option<IsolatedPublication>,
        std::marker::PhantomData<std::rc::Rc<()>>,
    );
    impl Drop for Restore {
        fn drop(&mut self) {
            PUBLICATION.with(|slot| *slot.borrow_mut() = self.0.take());
            scope_changed();
        }
    }
    let previous = PUBLICATION.with(|slot| {
        slot.replace(Some(IsolatedPublication {
            current: Some(Arc::new(language)),
            mods: ModEntries::default(),
        }))
    });
    scope_changed();
    Restore(previous, std::marker::PhantomData)
}

#[cfg(test)]
#[test]
fn isolated_reload_publications_are_mutable_nested_and_thread_local() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        for value in ["first", "second"] {
            let barrier = barrier.clone();
            scope.spawn(move || {
                let _outer = isolate_publication(Language::of("en_us", []));
                set(Language::of(
                    "en_us",
                    [("fixture.key".into(), value.into())],
                ));
                barrier.wait();
                assert_eq!(get("fixture.key"), value);
                {
                    let _inner = isolate_publication(Language::of("en_us", []));
                    set(Language::of(
                        "en_us",
                        [("fixture.key".into(), "nested".into())],
                    ));
                    assert_eq!(get("fixture.key"), "nested");
                }
                assert_eq!(get("fixture.key"), value);
            });
        }
    });
}

pub fn set(language: Language) {
    update_publication(|current, mods| {
        commit_language(current, mods, language);
        ((), true)
    });
}
#[cfg(feature = "test-language-publication")]
pub fn set_shared(language: Arc<Language>) {
    update_publication(|current, mods| {
        if mods.entries.is_empty() && language.shadowed.is_empty() {
            *current = Some(language);
        } else {
            commit_language(current, mods, Language::clone(&language));
        }
        ((), true)
    });
}
pub fn current() -> Option<Arc<Language>> {
    if let Some(language) = SCOPE.with(|s| s.borrow().as_ref().map(|s| s.language.clone())) {
        return Some(language);
    }
    #[cfg(any(test, feature = "test-language-publication"))]
    if let Some(language) = PUBLICATION.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|isolated| isolated.current.clone())
    }) {
        return Some(language);
    }
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone()
}
/// Runs `f` with `language` as the current language of this thread only, and returns what it
/// returned with the keys `get` and `get_or` looked up and `language` did not have, in the order
/// asked and without repeats. Nothing is set process-wide, so a test can draw a screen in another
/// language while the tests beside it read theirs. Scopes nest: the inner one wins, and the outer
/// one's misses do not include the inner one's. For tests: `bidirectional` stays the process-wide
/// language's.
pub fn scoped<R>(language: Language, f: impl FnOnce() -> R) -> (R, Vec<String>) {
    scoped_shared(Arc::new(language), f)
}
/// [`scoped`] with a language already shared.
fn scoped_shared<R>(language: Arc<Language>, f: impl FnOnce() -> R) -> (R, Vec<String>) {
    /// Puts the outer scope back, also when `f` panics.
    struct Restore(Option<Scope>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPE.with(|s| *s.borrow_mut() = self.0.take());
            scope_changed();
        }
    }
    let outer = SCOPE.with(|s| {
        s.borrow_mut().replace(Scope {
            language,
            missed: Vec::new(),
        })
    });
    let _restore = Restore(outer);
    scope_changed();
    let result = f();
    let missed = SCOPE.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map(|s| std::mem::take(&mut s.missed))
    });
    (result, missed.unwrap_or_default())
}
/// Text the client sends back to a server, resolved as vanilla's client resolves it (an anvil's
/// name box, a sign opened for editing: `Component.getString`): a key only Sunrise's own files
/// give comes back as the key, so a server can't tell this client by the keys it knows.
pub fn returned_to_server<R>(f: impl FnOnce() -> R) -> R {
    match current().and_then(|language| language.vanilla.clone()) {
        Some(vanilla) => scoped_shared(vanilla, f).0,
        None => f(),
    }
}
/// `key` was asked for in a `scoped` language that has no text for it.
pub(crate) fn missed(key: &str) {
    SCOPE.with(|s| {
        if let Some(scope) = s.borrow_mut().as_mut()
            && !scope.missed.iter().any(|m| m == key)
        {
            scope.missed.push(key.to_owned());
        }
    });
}
/// `ClientLanguage.defaultRightToLeft` of the current language.
pub fn bidirectional() -> bool {
    #[cfg(any(test, feature = "test-language-publication"))]
    if let Some(language) = PUBLICATION.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|isolated| isolated.current.clone())
    }) {
        return language.bidirectional;
    }
    BIDIRECTIONAL.load(Ordering::Relaxed)
}
/// Whether the current published language carries any active run-time mod overrides.
pub fn has_mod_entries() -> bool {
    #[cfg(any(test, feature = "test-language-publication"))]
    if let Some(has) = PUBLICATION.with(|slot| {
        slot.borrow().as_ref().map(|isolated| {
            isolated
                .current
                .as_deref()
                .is_some_and(|language| !language.shadowed.is_empty())
        })
    }) {
        return has;
    }
    HAS_MOD_OVERLAY.load(Ordering::Acquire)
}
pub fn code() -> String {
    current().map_or_else(|| DEFAULT.to_owned(), |language| language.code.to_string())
}

/// Snapshot the current language code without copying its text or keeping a publication guard.
pub fn code_snapshot() -> Arc<str> {
    static DEFAULT_CODE: LazyLock<Arc<str>> = LazyLock::new(|| DEFAULT.into());
    current().map_or_else(
        || Arc::clone(&DEFAULT_CODE),
        |language| Arc::clone(&language.code),
    )
}
/// Language.getOrDefault: the translation, or the key itself.
pub fn get(key: &str) -> String {
    get_or(key, key)
}
/// The translation, or `fallback` when the key is missing.
pub fn get_or(key: &str, fallback: &str) -> String {
    current()
        .and_then(|l| {
            l.get(key).map(str::to_owned).or_else(|| {
                l.component(key)
                    .and_then(|component| {
                        crate::text::resolve_lang_component(
                            &l,
                            component,
                            crate::text::Origin::Local,
                        )
                    })
                    .map(|text| text.plain())
            })
        })
        .unwrap_or_else(|| {
            missed(key);
            fallback.to_owned()
        })
}
pub fn has(key: &str) -> bool {
    current().is_some_and(|l| l.get(key).is_some() || l.component(key).is_some())
}
/// I18n.get with plain string arguments.
pub fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> String {
    crate::text::Text::tr(key, args).plain()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mem(HashMap<String, Vec<u8>>);
    impl ResourceSource for Mem {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
    }
    #[test]
    fn dedicated_components_use_default_english_and_literal_keybind_ids() {
        let source = Mem(HashMap::from([
            (
                "assets/minecraft/lang/en_us.json".into(),
                br#"{"test.old":"Value: %2$s / %1$s","key.keyboard.space":"Space","test.removed":"Hidden"}"#.to_vec(),
            ),
            (
                "assets/minecraft/lang/deprecated.json".into(),
                br#"{"removed":["test.removed"],"renamed":{"test.old":"test.value"}}"#.to_vec(),
            ),
            (
                "assets/example/lang/en_us.json".into(),
                br#"{"test.mod":"Mod translation"}"#.to_vec(),
            ),
        ]));
        scoped(Language::load_default(&source), || {
            let text = crate::text::Text::from(serde_json::json!({
                "translate": "test.value",
                "with": ["name", {"keybind": "key.keyboard.space"}],
                "extra": [
                    " ",
                    {"translate": "test.mod", "fallback": "fallback"},
                    " ",
                    {"translate": "test.removed"}
                ]
            }));
            assert_eq!(
                text.plain(),
                "Value: key.keyboard.space / name fallback test.removed"
            );
        });
    }

    #[test]
    fn client_lang_stops_at_non_primitive_values() {
        let client = Mem(HashMap::from([(
            "assets/minecraft/lang/en_us.json".into(),
            br#"{"test.good":"%2$03d / %.2f","test.component":{"text":"not a string"},"test.after":"unread"}"#.to_vec(),
        )]));
        let lang = Language::load(&[&client], "en_us");
        assert_eq!(lang.get("test.good"), Some("%2$s / %s"));
        assert_eq!(lang.get("test.component"), None);
        assert_eq!(lang.get("test.after"), None);
    }

    #[test]
    fn server_starting_layers_pack_languages_and_resolves_component_values() {
        use sunrise_assets::resource_source::{Resource, ResourceSource as Assets};

        let base = Mem(HashMap::from([
            (
                "assets/minecraft/lang/en_us.json".into(),
                br#"{"test.decimal":"%2$03d","test.removed":"old","test.text":"Base text"}"#
                    .to_vec(),
            ),
            (
                "assets/minecraft/lang/deprecated.json".into(),
                br#"{"removed":["test.removed"]}"#.to_vec(),
            ),
        ]));
        let assets = |files: &[(&str, &str)]| {
            Assets::indexed(
                files
                    .iter()
                    .map(|(path, file)| {
                        (
                            (*path).to_owned(),
                            Resource::bytes(file.as_bytes().to_vec()),
                        )
                    })
                    .collect(),
            )
        };
        let mods = assets(&[
            (
                "assets/a/lang/en_us.json",
                r#"{"test.text":"Mod text","test.order":"a","test.priority":"Mod priority"}"#,
            ),
            ("assets/z/lang/en_us.json", r#"{"test.order":"z"}"#),
        ]);
        let pack = assets(&[
            (
                "assets/a/lang/en_us.json",
                r#"{
                "test.text": [{"translate":"test.priority"},{"text":"!"}],
                "test.priority":"Pack priority",
                "test.removed":"Do not revive",
                "test.loop":{"translate":"test.loop","fallback":"Do not use"}
            }"#,
            ),
            (
                "assets/a/lang/fr_fr.json",
                r#"{"test.order":"not English"}"#,
            ),
        ]);
        let read = |key| {
            crate::text::Text::from(serde_json::json!({
                "translate":key,"fallback":"fallback","with":["first","second"]
            }))
            .plain()
        };

        scoped(Language::load_default(&base), || {
            assert_eq!(read("test.text"), "Base text");
            assert_eq!(read("test.priority"), "fallback");
        });
        scoped(
            Language::load_server(&base, [&mods as &dyn ResourceSource, &pack], |value| {
                Some(value.clone())
            }),
            || {
                assert_eq!(read("test.order"), "z");
                assert_eq!(read("test.text"), "Pack priority!");
                assert_eq!(read("test.decimal"), "second");
                assert_eq!(read("test.removed"), "fallback");
                assert_eq!(read("test.loop"), "");
            },
        );
    }

    #[test]
    fn server_language_only_elides_active_cycles() {
        let mut entries = serde_json::Map::new();
        for n in 0..1024 {
            entries.insert(
                format!("test.chain.{n}"),
                serde_json::json!({"translate": format!("test.chain.{}", n + 1)}),
            );
        }
        let literal = "z".repeat(70 * 1024);
        entries.insert(
            "test.chain.1024".into(),
            serde_json::json!({"text":literal}),
        );
        entries.insert(
            "test.repeat".into(),
            serde_json::Value::Array(vec![serde_json::json!({"translate":"test.shared"}); 160]),
        );
        entries.insert("test.shared".into(), serde_json::json!({"text":"."}));
        entries.insert(
            "test.cycle.a".into(),
            serde_json::json!({"translate":"test.cycle.b","extra":["?"]}),
        );
        entries.insert(
            "test.cycle.b".into(),
            serde_json::json!({"translate":"test.cycle.a","fallback":"unused","extra":["!"]}),
        );
        entries.insert(
            "test.cycle.siblings".into(),
            serde_json::json!([{"translate":"test.cycle.a"},{"translate":"test.cycle.b"}]),
        );
        let mut language = Language::default();
        assert!(
            language.overlay(&serde_json::to_vec(&entries).unwrap(), &|value| Some(
                value.clone()
            ))
        );
        scoped(language, || {
            assert_eq!(get("test.chain.0"), literal);
            assert_eq!(get("test.repeat"), ".".repeat(160));
            assert_eq!(get("test.cycle.a"), "!?");
            assert_eq!(get("test.cycle.siblings"), "!??!");
        });
    }

    #[test]
    fn language_shares_a_diamond_chain_and_refuses_oversized_fanout() {
        let mut entries = serde_json::Map::new();
        for n in 0..48 {
            let next = format!("test.diamond.{}", n + 1);
            entries.insert(
                format!("test.diamond.{n}"),
                serde_json::json!([{"translate":next},{"translate":next}]),
            );
        }
        entries.insert("test.diamond.48".into(), serde_json::json!({"text":""}));
        entries.insert(
            "test.large".into(),
            serde_json::json!({"text":"x".repeat(64 * 1024)}),
        );
        entries.insert(
            "test.fanout".into(),
            serde_json::Value::Array(vec![serde_json::json!({"translate":"test.large"}); 128]),
        );
        let mut language = Language::default();
        assert!(
            language.overlay(&serde_json::to_vec(&entries).unwrap(), &|value| Some(
                value.clone()
            ))
        );
        scoped(language, || {
            assert_eq!(get_or("test.diamond.0", "fallback"), "");
            assert_eq!(get_or("test.fanout", "fallback"), "fallback");
        });
    }

    #[test]
    fn empty_cycle_dependent_expansions_are_work_bounded() {
        let mut entries = serde_json::Map::new();
        entries.insert(
            "test.root".into(),
            serde_json::json!([{"translate":"test.left.0"},{"translate":"test.right.0"}]),
        );
        for n in 0..24 {
            let children = serde_json::json!([
                {"translate":format!("test.left.{}", n + 1)},
                {"translate":format!("test.right.{}", n + 1)}
            ]);
            entries.insert(format!("test.left.{n}"), children.clone());
            entries.insert(format!("test.right.{n}"), children);
        }
        entries.insert(
            "test.left.24".into(),
            serde_json::json!({"translate":"test.root"}),
        );
        entries.insert(
            "test.right.24".into(),
            serde_json::json!({"translate":"test.root"}),
        );
        let mut language = Language::default();
        assert!(
            language.overlay(&serde_json::to_vec(&entries).unwrap(), &|value| Some(
                value.clone()
            ))
        );
        scoped(language, || {
            assert_eq!(get_or("test.root", "over budget"), "over budget")
        });
    }

    #[test]
    fn language_budgets_inline_fallbacks_and_inherited_insertion() {
        let payload = "x".repeat(64 * 1024);
        let entries = serde_json::json!({
            "test.object": {"object":"atlas","sprite":"minecraft:test","fallback":{"text":payload}},
            "test.inline_fanout": vec![serde_json::json!({"translate":"test.object"}); 128],
            "test.spans": vec![serde_json::json!({"text":"."}); 128],
            "test.insertion": {"translate":"test.spans","insertion":payload},
        });
        let mut language = Language::default();
        assert!(
            language.overlay(&serde_json::to_vec(&entries).unwrap(), &|value| Some(
                value.clone()
            ))
        );
        scoped(language, || {
            assert_eq!(get("test.object"), payload);
            assert_eq!(get_or("test.inline_fanout", "over budget"), "over budget");
            assert_eq!(get_or("test.insertion", "over budget"), "over budget");
        });
    }

    #[test]
    fn bundled_sunrise_strings_follow_the_language() {
        // No prepared resources at all: the namespace is compiled in.
        let empty = Mem(HashMap::new());
        let en = Language::load(&[&empty], "en_us");
        assert_eq!(en.get("sunrise.options.controller"), Some("Controller"));
        let ru = Language::load(&[&empty], "ru_ru");
        assert_eq!(ru.get("sunrise.options.controller"), Some("Геймпад"));
        // Every bundled string carries both languages: a key added to one file
        // and forgotten in the other shows as a key name on screen.
        for key in en.map().keys() {
            assert!(ru.get(key).is_some(), "{key} has no ru_ru value");
        }
        assert_eq!(en.get("sunrise.pages.mods"), Some("Mods"));
        assert_eq!(ru.get("sunrise.pages.mods"), Some("Моды"));
        let zh = Language::load(&[&empty], "zh_cn");
        assert_eq!(zh.get("sunrise.options.controller"), Some("手柄"));
        assert_eq!(zh.get("sunrise.pages.mods"), Some("模组"));
        assert_eq!(zh.get("Preset"), Some("预设"));
        assert_eq!(zh.get("%s chunks"), Some("%s 个区块"));
        // Not every bundle is complete: the en_us layer is in every map, so a
        // missing key falls back to English (no bundle exists for xx_xx).
        assert_eq!(zh.get("LOD Distance"), Some("LOD 距离"));
        let xx = Language::load(&[&empty], "xx_xx");
        assert_eq!(xx.get("sunrise.mods.not_built"), Some(" (Not built)"));
        let pl = Language::load(&[&empty], "pl_pl");
        assert_eq!(pl.get("sunrise.options.controller"), Some("Kontroler"));
        assert_eq!(pl.get("sunrise.options.vibration"), Some("Wibracje"));
        let uk = Language::load(&[&empty], "uk_ua");
        assert_eq!(
            uk.get("sunrise.options.on_screen_keyboard"),
            Some("Екранна клавіатура")
        );
        let tr = Language::load(&[&empty], "tr_tr");
        assert_eq!(tr.get("sunrise.options.controller"), Some("Kontrolcü"));
        let es = Language::load(&[&empty], "es_es");
        assert_eq!(es.get("sunrise.options.controller"), Some("Mando"));
        // German and French, and their regional variants on the same file
        // (Swiss German without "ß").
        let de = Language::load(&[&empty], "de_de");
        assert_eq!(de.get("sunrise.osk.close"), Some("Schließen"));
        let fr = Language::load(&[&empty], "fr_fr");
        assert_eq!(fr.get("sunrise.osk.close"), Some("Fermer"));
        for (code, base) in [("de_at", &de), ("fr_ca", &fr), ("fr_ch", &fr)] {
            let variant = Language::load(&[&empty], code);
            assert_eq!(variant.map(), base.map(), "{code}");
        }
        let de_ch = Language::load(&[&empty], "de_ch");
        assert_eq!(de_ch.get("sunrise.osk.close"), Some("Schliessen"));
        assert!(de_ch.map().values().all(|v| !v.contains('ß')));
        assert_eq!(de_ch.len(), de.len());
        let es_ve = Language::load(&[&empty], "es_ve");
        assert_eq!(
            es_ve.get("sunrise.options.on_screen_keyboard"),
            Some("Teclado en pantalla")
        );
    }

    #[test]
    fn a_crates_text_loads_after_the_engines_and_before_packs() {
        static CRATE: &[(&str, &str)] = &[
            (
                "en_us",
                r#"{"test.crate.hello": "Hello", "sunrise.osk.close": "Shut", "test.crate.packed": "Crate"}"#,
            ),
            (
                "de_de",
                r#"{"test.crate.hello": "Grüß dich", "sunrise.osk.close": "Zu"}"#,
            ),
            ("fr_fr", r#"{"test.crate.hello": "Bonjour"}"#),
        ];
        let pack = Mem(HashMap::from([(
            "assets/sunrise/lang/en_us.json".to_owned(),
            br#"{"test.crate.packed": "Pack"}"#.to_vec(),
        )]));
        let load = |code| Language::load_with(&[&pack], code, &[CRATE]);
        let en = load("en_us");
        assert_eq!(en.get("test.crate.hello"), Some("Hello"));
        // The engine's own file wins on a key both have; a pack wins over both.
        assert_eq!(en.get("sunrise.osk.close"), Some("Close"));
        assert_eq!(en.get("test.crate.packed"), Some("Pack"));
        let de = load("de_de");
        assert_eq!(de.get("test.crate.hello"), Some("Grüß dich"));
        assert_eq!(de.get("sunrise.osk.close"), Some("Schließen"));
        // Regional variants read their language's file, as the engine's do.
        assert_eq!(load("de_ch").get("test.crate.hello"), Some("Grüss dich"));
        assert_eq!(load("fr_ca").get("test.crate.hello"), Some("Bonjour"));
        // A language the crate lacks falls back to its English.
        assert_eq!(load("pl_pl").get("test.crate.hello"), Some("Hello"));
        // Without crate files, nothing changes.
        let plain = Language::load_with(&[&pack], "de_de", &[]);
        assert_eq!(plain.get("test.crate.hello"), None);
        assert_eq!(plain.len() + 1, de.len());
    }

    /// Text the client sends back resolves as vanilla's client would load it from the same
    /// sources: a key only Sunrise's own files or the crates give stays the key, and a pack's
    /// English stays the fallback for a key whose selected-language text is only Sunrise's own,
    /// while the screens keep Sunrise's own text.
    #[test]
    fn text_returned_to_a_server_knows_no_key_only_sunrise_gives() {
        static CRATE: &[(&str, &str)] = &[
            (
                "en_us",
                r#"{"test.crate.hello": "Hello", "test.crate.packed": "Crate"}"#,
            ),
            ("de_de", r#"{"test.crate.hello": "Hallo"}"#),
        ];
        let pack = Mem(HashMap::from([
            (
                "assets/minecraft/lang/en_us.json".to_owned(),
                br#"{"block.minecraft.stone": "Stone"}"#.to_vec(),
            ),
            (
                "assets/minecraft/lang/de_de.json".to_owned(),
                br#"{"block.minecraft.stone": "Stein"}"#.to_vec(),
            ),
            (
                "assets/sunrise/lang/en_us.json".to_owned(),
                br#"{"sunrise.osk.close": "Packed close", "test.crate.packed": "Pack"}"#.to_vec(),
            ),
        ]));
        let load = || Language::load_with(&[&pack], "de_de", &[CRATE]);
        let german = load();
        assert_eq!(german.get("sunrise.osk.close"), Some("Schließen"));
        assert_eq!(german.get("test.crate.hello"), Some("Hallo"));
        let back = |key: &str| scoped(load(), || returned_to_server(|| get(key))).0;
        assert_eq!(back("sunrise.pages.mods"), "sunrise.pages.mods");
        assert_eq!(back("test.crate.hello"), "test.crate.hello");
        assert_eq!(back("sunrise.osk.close"), "Packed close");
        assert_eq!(back("test.crate.packed"), "Pack");
        assert_eq!(back("block.minecraft.stone"), "Stein");
        // Outside the call, the same language keeps Sunrise's own text.
        assert_ne!(
            scoped(load(), || get("sunrise.pages.mods")).0,
            "sunrise.pages.mods"
        );
    }

    #[test]
    fn a_scoped_language_is_the_calling_threads_and_reports_what_it_lacks() {
        let empty = Mem(HashMap::new());
        let outside = code();
        let ((german, english_only, fallback, elsewhere), missed) =
            scoped(Language::load(&[&empty], "de_de"), || {
                (
                    get("sunrise.osk.close"),
                    get("sunrise.no_such_key"),
                    get_or("sunrise.no_fallback", "English"),
                    // Another thread keeps the process-wide language.
                    std::thread::spawn(code).join().unwrap(),
                )
            });
        assert_eq!(german, "Schließen");
        assert_eq!(english_only, "sunrise.no_such_key");
        assert_eq!(fallback, "English");
        assert_eq!(elsewhere, outside);
        assert_eq!(missed, ["sunrise.no_such_key", "sunrise.no_fallback"]);
        // The scope ends with its closure.
        assert_eq!(code(), outside);
    }

    #[test]
    fn scopes_nest_and_end_when_their_closure_panics() {
        let empty = Mem(HashMap::new());
        let outside = code();
        let (inner, missed) = scoped(Language::load(&[&empty], "fr_fr"), || {
            let (inner, inner_missed) = scoped(Language::load(&[&empty], "pl_pl"), || {
                (code(), get("sunrise.inner_miss"))
            });
            assert_eq!(inner_missed, ["sunrise.inner_miss"]);
            assert_eq!(code(), "fr_fr");
            inner.0
        });
        assert_eq!(inner, "pl_pl");
        assert!(missed.is_empty());
        let panicked = std::panic::catch_unwind(|| {
            scoped(Language::load(&[&Mem(HashMap::new())], "de_de"), || {
                panic!("a test failed inside a scope")
            })
        });
        assert!(panicked.is_err());
        assert_eq!(code(), outside);
    }

    /// `crates/sunrise-client/assets/sunrise/lang`, where Sunrise's own strings live.
    fn bundled_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sunrise-client/assets/sunrise/lang")
    }
    /// The crates whose own lang files the game loads (`crates/<name>/assets/lang`,
    /// registered through `sunrise_hooks::lang`), with that folder.
    fn crate_lang_dirs() -> Vec<(String, std::path::PathBuf)> {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut dirs: Vec<_> = std::fs::read_dir(&crates)
            .unwrap()
            .flatten()
            .map(|e| (e.file_name().into_string().unwrap(), e.path()))
            .map(|(name, path)| (name, path.join("assets/lang")))
            .filter(|(_, dir)| dir.join("en_us.json").is_file())
            .collect();
        dirs.sort();
        dirs
    }
    /// The keys of a flat bundled file in file order, duplicates included (a map
    /// would keep only the last of two equal keys). One `"key": "value"` per line.
    fn file_keys(body: &str) -> Vec<String> {
        body.lines()
            .filter(|line| line.starts_with("  \""))
            .map(|line| {
                serde_json::Deserializer::from_str(line.trim_start())
                    .into_iter::<String>()
                    .next()
                    .and_then(Result::ok)
                    .unwrap_or_else(|| panic!("no key on line {line}"))
            })
            .collect()
    }
    /// The fields the code fills in (`%s`, `%d`, `%1$s`, `%.2f`, `%%`), sorted. A
    /// percent sign that starts none of them ("100% is") is plain text.
    fn placeholders(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find('%') {
            rest = &rest[at + 1..];
            let spec = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '$' || c == '.'))
                .unwrap_or(rest.len());
            let after = rest[spec..].chars().next();
            if after.is_some_and(|c| matches!(c, 's' | 'd' | 'f' | '%')) {
                let end = spec + 1;
                found.push(format!("%{}", &rest[..end]));
                rest = &rest[end..];
            }
        }
        found.sort();
        found
    }

    #[test]
    fn bundled_files_are_served_and_have_no_stale_or_repeated_keys() {
        let engine = check_lang_dir(&bundled_dir(), true);
        assert!(
            engine.1 >= 7,
            "found {} bundled lang files in {}",
            engine.1,
            bundled_dir().display()
        );
        // A crate's files pass the same checks. Its keys are its own: on a key the engine's
        // files also have, the engine's text would win and hide the crate's.
        for (name, dir) in crate_lang_dirs() {
            let (en, _) = check_lang_dir(&dir, false);
            for key in en.keys() {
                assert!(
                    !engine.0.contains_key(key),
                    "{name}: {key} is also the engine's"
                );
            }
        }
    }

    /// The checks every lang folder passes: flat maps, no repeated key, no key en_us lacks, the
    /// same placeholders as English, no empty text. Its en_us and how many files it has.
    fn check_lang_dir(dir: &std::path::Path, bundled: bool) -> (HashMap<String, String>, usize) {
        let en_body = std::fs::read_to_string(dir.join("en_us.json")).unwrap();
        let en: HashMap<String, String> = serde_json::from_str(&en_body).unwrap();
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().into_string().unwrap();
            let Some(code) = name.strip_suffix(".json") else {
                continue;
            };
            seen += 1;
            // A file that Bundled::read does not name is never loaded.
            let path = format!("assets/sunrise/lang/{name}");
            assert!(
                !bundled || Bundled.read(&path).is_some(),
                "{name} is not served by Bundled"
            );
            let body = std::fs::read_to_string(entry.path()).unwrap();
            let map: HashMap<String, String> = serde_json::from_str(&body)
                .unwrap_or_else(|e| panic!("{name} is not a flat string map: {e}"));
            let keys = file_keys(&body);
            assert_eq!(
                keys.len(),
                map.len(),
                "{name} repeats a key or has a multi-line entry"
            );
            for (key, value) in &map {
                // Stale: the code no longer asks for it, or it was renamed in en_us only.
                let english = en
                    .get(key)
                    .unwrap_or_else(|| panic!("{code}: {key} is not in en_us.json"));
                // A translation keeps the fields the code fills in.
                assert_eq!(
                    placeholders(value),
                    placeholders(english),
                    "{code}: {key} changes the placeholders of the English text"
                );
                assert!(!value.trim().is_empty(), "{code}: {key} is empty");
            }
        }
        (en, seen)
    }

    /// The crates whose `sunrise.` keys must be in en_us.json.
    const CLIENT_CRATES: [&str; 4] = [
        "sunrise-client",
        "sunrise-client-render",
        "sunrise-game-data",
        "sunrise-client-input",
    ];

    /// The Rust sources of `krates`, or of every crate under `crates/` when it is empty.
    fn crate_sources(krates: &[&str]) -> Vec<std::path::PathBuf> {
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        if krates.is_empty() {
            for entry in std::fs::read_dir(&crates).unwrap().flatten() {
                rust_files(&entry.path().join("src"), &mut files);
            }
        }
        for krate in krates {
            rust_files(&crates.join(krate).join("src"), &mut files);
        }
        assert!(files.len() > 100, "found only {} sources", files.len());
        files
    }

    /// A sentence key (the English text itself, not a `sunrise.` id) that no
    /// source names as a string literal is text the game no longer shows: its
    /// rewording left it behind in every language (#13802).
    #[test]
    #[ignore = "daily"]
    fn every_sentence_key_in_en_us_is_one_the_code_names() {
        let en: HashMap<String, String> = serde_json::from_str(
            &std::fs::read_to_string(bundled_dir().join("en_us.json")).unwrap(),
        )
        .unwrap();
        // Every crate: a feature moved out of the client crates keeps its strings.
        let code: String = crate_sources(&[])
            .iter()
            .map(|file| std::fs::read_to_string(file).unwrap())
            .collect();
        let sentences: Vec<&String> = en.keys().filter(|k| !k.starts_with("sunrise.")).collect();
        assert!(
            sentences.len() > 100,
            "found only {} sentence keys",
            sentences.len()
        );
        // Every quote in the sources opens a candidate literal that runs to the next
        // unescaped quote, quotes included. A needle is a quote, escaped text and a
        // quote, so wherever it occurs it is exactly such a slice, and one pass
        // over the sources replaces a `contains` scan per key.
        let bytes = code.as_bytes();
        let mut literals: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (start, _) in bytes.iter().enumerate().filter(|(_, b)| **b == b'"') {
            let mut at = start + 1;
            while at < bytes.len() && bytes[at] != b'"' {
                at += if bytes[at] == b'\\' { 2 } else { 1 };
            }
            if at < bytes.len() {
                literals.insert(&code[start..=at]);
            }
        }
        // A JSON string literal escapes `"` and `\` as Rust's does.
        let mut dead: Vec<&str> = sentences
            .into_iter()
            .filter(|key| !literals.contains(serde_json::to_string(key).unwrap().as_str()))
            .map(String::as_str)
            .collect();
        dead.sort_unstable();
        assert!(
            dead.is_empty(),
            "en_us.json keys no code asks for: {dead:#?}"
        );
    }

    #[test]
    fn every_sunrise_key_the_code_names_is_in_en_us() {
        let english = |dir: &std::path::Path| -> HashMap<String, String> {
            serde_json::from_str(&std::fs::read_to_string(dir.join("en_us.json")).unwrap()).unwrap()
        };
        let mut en = english(&bundled_dir());
        let mut krates = CLIENT_CRATES.to_vec();
        // A crate that ships lang files names its keys in its own sources.
        let lang_dirs = crate_lang_dirs();
        for (name, dir) in &lang_dirs {
            en.extend(english(dir));
            krates.push(name);
        }
        let mut found = 0;
        for file in crate_sources(&krates) {
            let text = std::fs::read_to_string(&file).unwrap();
            for (at, _) in text.match_indices("\"sunrise.") {
                let rest = &text[at + 1..];
                let Some(end) = rest.find('"') else { continue };
                let key = &rest[..end];
                // A key is `sunrise.<group>.<name>`: lower-case letters, digits and `_`
                // between the dots. A key built at run time has a brace; a file name
                // has one dot; the server addresses that look like keys are listed.
                const NOT_KEYS: &[&str] = &["sunrise.bunchy7s.dev", "sunrise.hdr.tonemap"];
                let key_like = key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.".contains(&b));
                if key.matches('.').count() < 2 || !key_like || NOT_KEYS.contains(&key) {
                    continue;
                }
                found += 1;
                assert!(
                    en.contains_key(key),
                    "{} names {key}, which is not in en_us.json",
                    file.display()
                );
            }
        }
        assert!(found > 50, "found only {found} key names");
    }

    #[test]
    fn layers_selected_language_over_english() {
        let base = Mem(HashMap::from([
            ("assets/minecraft/lang/en_us.json".into(), br#"{"a":"Apple","b":"Banana"}"#.to_vec()),
            ("assets/minecraft/lang/fr_fr.json".into(), br#"{"a":"Pomme"}"#.to_vec()),
            ("assets/pack.mcmeta".into(), r#"{"language":{"fr_fr":{"name":"Français","region":"France"},"de_de":{"name":"Deutsch","region":"Deutschland"}}}"#.as_bytes().to_vec()),
        ]));
        let pack = Mem(HashMap::from([(
            "assets/minecraft/lang/fr_fr.json".into(),
            br#"{"b":"Banane"}"#.to_vec(),
        )]));
        let sources: [&dyn ResourceSource; 2] = [&base, &pack];
        let fr = Language::load(&sources, "fr_fr");
        assert_eq!(fr.get("a"), Some("Pomme"));
        assert_eq!(fr.get("b"), Some("Banane"));
        let langs = available(&sources);
        // de_de has no file, so only English and French are offered.
        assert_eq!(
            langs.iter().map(|l| l.code.as_str()).collect::<Vec<_>>(),
            ["en_us", "fr_fr"]
        );
        assert_eq!(langs[1].label(), "Français (France)");
    }

    #[test]
    fn languages_are_listed_without_reading_their_files() {
        struct NoLangReads;
        impl ResourceSource for NoLangReads {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                assert!(!path.contains("/lang/"), "read {path}");
                (path == "assets/pack.mcmeta").then(|| {
                    br#"{"language":{"fr_fr":{"name":"French"},"de_de":{"name":"German"}}}"#
                        .to_vec()
                })
            }
            fn exists(&self, path: &str) -> bool {
                path == "assets/minecraft/lang/fr_fr.json"
            }
        }
        let langs = available(&[&NoLangReads]);
        assert_eq!(
            langs.iter().map(|l| l.code.as_str()).collect::<Vec<_>>(),
            ["en_us", "fr_fr"]
        );
    }

    fn read(body: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        read_lang_file(body.as_bytes(), &mut map);
        map
    }
    fn entries(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_plain_json_lang_file_reads_as_before() {
        let body = "{\"a\": \"Apple \\\"quoted\\\" \\u00e9\", \"b\": \"\", \"a\": \"Again\"}";
        let strict: HashMap<String, String> = serde_json::from_str(body).unwrap();
        assert_eq!(read(body), strict);
        assert_eq!(read(body), entries(&[("a", "Again"), ("b", "")]));
    }

    #[test]
    fn lang_files_read_with_comments() {
        let want = entries(&[("a", "Apple"), ("b", "Banana # not a comment // either")]);
        for body in [
            "# header\n{\n# a comment\n\"a\": \"Apple\",\n\"b\": \"Banana # not a comment // either\"\n}\n",
            "// header\n{\"a\": \"Apple\", // after\n\"b\": \"Banana # not a comment // either\"}",
            "/* header\n over lines */{\"a\": /* inline */ \"Apple\", \"b\": \"Banana # not a comment // either\"}",
        ] {
            assert_eq!(read(body), want, "{body}");
        }
    }

    #[test]
    fn lang_files_read_unquoted_and_single_quoted_names_and_values() {
        assert_eq!(
            read("{a: 'Apple', 'b': \"Banana\", c: Cherry}"),
            entries(&[("a", "Apple"), ("b", "Banana"), ("c", "Cherry")])
        );
        // A single-quoted string may hold a double quote, and the reverse.
        assert_eq!(
            read("{'a': 'say \"hi\"', \"b\": \"it's\"}"),
            entries(&[("a", "say \"hi\""), ("b", "it's")])
        );
    }

    #[test]
    fn lang_files_read_raw_control_characters_in_strings() {
        assert_eq!(
            read("{\"a\": \"tab\there\", \"b\": \"two\nlines\"}"),
            entries(&[("a", "tab\there"), ("b", "two\nlines")])
        );
    }

    #[test]
    fn lang_files_read_gson_separators_and_a_byte_order_mark() {
        assert_eq!(
            read("\u{feff}{\"a\" = \"Apple\"; \"b\" => \"Banana\"}"),
            entries(&[("a", "Apple"), ("b", "Banana")])
        );
        // A trailing comma before `}` is not JSON even to Gson's lenient reader.
        assert_eq!(read("{# c\n\"a\": \"Apple\",}"), HashMap::new());
    }

    #[test]
    fn numbers_and_booleans_read_as_their_text() {
        // GsonHelper.convertToString: a primitive reads as its text.
        assert_eq!(
            read("{\"a\": 1.50, \"b\": true, \"c\": -2e3}"),
            entries(&[("a", "1.50"), ("b", "true"), ("c", "-2e3")])
        );
    }

    #[test]
    fn a_value_that_is_no_text_ends_the_file_there() {
        assert_eq!(
            read("# c\n{\"a\": \"Apple\", \"b\": [\"x\"], \"c\": \"Cherry\"}"),
            entries(&[("a", "Apple")])
        );
        assert_eq!(read("{\"a\": null, \"b\": \"Banana\"}"), HashMap::new());
    }

    #[test]
    fn a_lang_file_that_is_not_json_adds_nothing() {
        for body in [
            "# c\n{\"a\": \"Apple\"",
            "[\"a\"]",
            "\"a\"",
            "",
            "{\"a\": \"Apple\"} {",
        ] {
            assert_eq!(read(body), HashMap::new(), "{body:?}");
        }
    }

    #[test]
    fn a_pack_lang_file_with_comments_layers_over_english() {
        let base = Mem(HashMap::from([(
            "assets/minecraft/lang/en_us.json".into(),
            br#"{"a":"Apple","b":"Banana"}"#.to_vec(),
        )]));
        let pack = Mem(HashMap::from([(
            "assets/minecraft/lang/fr_fr.json".into(),
            b"{\n# Fruits\n\"a\": \"Pomme\"\n}\n".to_vec(),
        )]));
        let sources: [&dyn ResourceSource; 2] = [&base, &pack];
        let fr = Language::load(&sources, "fr_fr");
        assert_eq!(fr.get("a"), Some("Pomme"));
        assert_eq!(fr.get("b"), Some("Banana"));
    }

    /// Every lang file of a downloaded pack, e.g. BACAP Language Pack:
    /// `SUNRISE_LANG_PACK_DIR=<pack>/assets/minecraft/lang cargo test -p sunrise-game-data
    /// lang::tests::every_lang_file_of_a_pack_reads -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a downloaded pack in SUNRISE_LANG_PACK_DIR"]
    fn every_lang_file_of_a_pack_reads() {
        let Ok(dir) = std::env::var("SUNRISE_LANG_PACK_DIR") else {
            return;
        };
        let mut failed = Vec::new();
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            let bytes = std::fs::read(&path).unwrap();
            let strict = serde_json::from_slice::<HashMap<String, String>>(&bytes).is_ok();
            let mut map = HashMap::new();
            read_lang_file(&bytes, &mut map);
            // An independent count: the distinct `"name":` that start a line.
            let text = String::from_utf8_lossy(&bytes);
            let names: std::collections::HashSet<&str> = text
                .lines()
                .filter_map(|line| {
                    let rest = line.trim_start().strip_prefix('"')?;
                    let mut escaped = false;
                    let end = rest.char_indices().find_map(|(i, c)| {
                        let close = c == '"' && !escaped;
                        escaped = c == '\\' && !escaped;
                        close.then_some(i)
                    })?;
                    rest[end + 1..]
                        .trim_start()
                        .starts_with(':')
                        .then(|| &rest[..end])
                })
                .collect();
            println!(
                "{}: strict {strict}, {} keys, {} names",
                path.display(),
                map.len(),
                names.len()
            );
            if map.is_empty() || map.len() != names.len() {
                failed.push(path);
            }
        }
        assert!(failed.is_empty(), "{failed:?}");
    }
}

#[cfg(test)]
mod shared_source_tests {
    use super::*;

    fn pairs(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// Issue #5474: a mod's run-time entries lie over the packs', normalize numeric placeholders,
    /// survive a fresh pack reload, support replacement and empty-map removal, and come off cleanly.
    #[test]
    fn mod_entries_lie_over_the_packs_and_come_off_again() {
        let _isolated = isolate_publication(Language::of(
            "de_de",
            [
                ("t5474.a".into(), "pack a".into()),
                ("t5474.b".into(), "pack b".into()),
            ],
        ));
        assert!(!has_mod_entries());
        assert_eq!(
            set_mod_entries(
                "lang_test_mod",
                "en_us",
                pairs(&[
                    ("t5474.a", "en a"),
                    ("t5474.c", "en c"),
                    ("t5474.count", "Count %2$03d"),
                ]),
            ),
            3
        );
        assert!(has_mod_entries());
        assert_eq!(
            set_mod_entries("lang_test_mod", "de_de", pairs(&[("t5474.a", "de a")])),
            1
        );
        assert_eq!(get("t5474.a"), "de a", "the selected language wins");
        assert_eq!(get("t5474.b"), "pack b", "untouched keys stay");
        assert_eq!(get("t5474.c"), "en c", "falls back to the mod's en_us");
        let count = crate::text::Text::translate(
            "t5474.count",
            &[
                crate::text::Text::literal("first"),
                crate::text::Text::literal("42"),
            ],
        );
        assert_eq!(count.plain(), "Count 42");

        // Replacing entries for a locale updates the published lookup immediately.
        assert_eq!(
            set_mod_entries("lang_test_mod", "de_de", pairs(&[("t5474.a", "de a2")])),
            1
        );
        assert_eq!(get("t5474.a"), "de a2");

        // Publishing a fresh pack table keeps the mod overlay on top of the new pack entries.
        set(Language::of(
            "de_de",
            [
                ("t5474.a".into(), "reloaded a".into()),
                ("t5474.b".into(), "reloaded b".into()),
            ],
        ));
        assert_eq!(get("t5474.a"), "de a2");
        assert_eq!(get("t5474.b"), "reloaded b");
        assert_eq!(get("t5474.c"), "en c");

        // Passing an empty map drops only that locale's entries.
        assert_eq!(set_mod_entries("lang_test_mod", "de_de", HashMap::new()), 0);
        assert_eq!(get("t5474.a"), "en a");
        assert_eq!(get("t5474.c"), "en c");

        clear_mod_entries("lang_test_mod");
        assert!(!has_mod_entries());
        assert_eq!(get("t5474.a"), "reloaded a", "the fresh pack text is back");
        assert!(!has("t5474.c"), "a key only the mod had is gone");
        assert_eq!(get("t5474.c"), "t5474.c");
        assert!(!has("t5474.count"));
    }

    #[test]
    fn later_mod_id_wins_across_locales_and_clearing_reveals_earlier_mod() {
        let _isolated = isolate_publication(Language::of(
            "de_de",
            [("t5474.shared".into(), "pack shared".into())],
        ));
        set_mod_entries("alpha", "de_de", pairs(&[("t5474.shared", "alpha de")]));
        set_mod_entries("zeta", "en_us", pairs(&[("t5474.shared", "zeta en")]));
        assert_eq!(get("t5474.shared"), "zeta en");

        clear_mod_entries("zeta");
        assert_eq!(get("t5474.shared"), "alpha de");

        clear_mod_entries("alpha");
        assert_eq!(get("t5474.shared"), "pack shared");
    }

    #[test]
    fn disabled_mod_entries_stay_hidden_across_reloads_and_return_on_reenable() {
        let _isolated = isolate_publication(Language::of(
            "en_us",
            [("t5474.k".into(), "pack v1".into())],
        ));
        set_mod_entries(
            "mod_a",
            "en_us",
            pairs(&[("t5474.k", "mod a"), ("t5474.only", "only a")]),
        );
        assert_eq!(get("t5474.k"), "mod a");
        assert_eq!(get("t5474.only"), "only a");

        // Toggling a mod with no entries does not republish.
        let before = generation();
        set_disabled_mod_entries(&BTreeSet::from(["other_mod".to_owned()]));
        assert_eq!(generation(), before);
        assert_eq!(get("t5474.k"), "mod a");

        let before = generation();
        set_disabled_mod_entries(&BTreeSet::from(["mod_a".to_owned()]));
        assert_eq!(generation(), before + 1);
        assert!(!has_mod_entries());
        assert_eq!(get("t5474.k"), "pack v1");
        assert!(!has("t5474.only"));

        set(Language::of(
            "en_us",
            [("t5474.k".into(), "pack v2".into())],
        ));
        assert!(!has_mod_entries());
        assert_eq!(get("t5474.k"), "pack v2");
        assert!(!has("t5474.only"));

        let before = generation();
        set_disabled_mod_entries(&BTreeSet::new());
        assert_eq!(generation(), before + 1);
        assert!(has_mod_entries());
        assert_eq!(get("t5474.k"), "mod a");
        assert_eq!(get("t5474.only"), "only a");

        clear_mod_entries("mod_a");
        assert_eq!(get("t5474.k"), "pack v2");
    }
    #[test]
    fn selected_language_and_english_fallback_read_resolved_resources() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/reader/lang/en_us.json".into(),
                Resource::bytes(
                    br#"{"reader.hello":"Hello","reader.only":"English fallback"}"#.to_vec(),
                ),
            ),
            (
                "assets/reader/lang/fr_fr.json".into(),
                Resource::bytes(br#"{"reader.hello":"Bonjour"}"#.to_vec()),
            ),
            (
                "assets/mod_fixture/lang/fr_fr.json".into(),
                Resource::bytes(br#"{"mod_fixture.menu":"Mod layer"}"#.to_vec()),
            ),
        ]));
        let language = Language::load(&[&source], "fr_fr");
        assert_eq!(language.map["reader.hello"], "Bonjour");
        assert_eq!(language.map["reader.only"], "English fallback");
        assert_eq!(language.map["mod_fixture.menu"], "Mod layer");
    }
}
