//! The host traits: the area files under `sunrise-mod-api/src/host/` are the hand-written
//! declarations of what the game implements for mods. This reads them and writes the unions
//! that name them (`Host`, `ServerHost`, `ClientHost` and the hidden backends), the handles'
//! forwarding methods and the mob tick's goal view.
use super::*;
use crate::gameplay::{Borrow, GameplayAudit, GameplayPackage, HostRoute, RouteKind, Routing};
use heck::ToUpperCamelCase;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use syn::visit::Visit;

/// Reads implemented by the scoped goal context over the active mob and its
/// disjoint live mob loans. Players are read from the game's backend; other
/// entity reads not supported by the scoped context remain typed refusals.
pub(super) const GOAL_VIEW_READS: &[&str] = &[
    "entity_position",
    "entity_block_position",
    "entity_velocity",
    "entity_rotation",
    "entity_head_yaw",
    "entity_body_yaw",
    "entity_bounding_box",
    "entity_eye_height",
    "entity_eye_position",
    "entity_distance_to_sqr",
    "entity_on_ground",
    "entity_kind",
    "living_health",
    "living_max_health",
    "living_alive",
    "merchant_offers",
    "merchant_trading_player",
    "merchant_profession",
    "merchant_level",
    "merchant_villager_type",
    "merchant_xp",
    "entity_dismount_location",
    "entity_root_vehicle",
    "entity_is_passenger_of_same_vehicle",
    "entity_visit_passengers",
    "entity_is_vehicle",
    "entity_visit_indirect_passengers",
    "entity_controlling_passenger",
    "entity_on_block",
    "entity_entities_team_damage_allowed",
    "living_can_use_portal",
    "entity_tags",
    "entity_name",
    "entity_custom_name",
    "entity_added_to_level",
    "entity_portal_cooldown",
    "entity_invulnerable_to_damage",
    "living_is_climbing",
    "living_last_attacker",
    "living_kill_credit",
    "entity_direction",
    "entity_pose",
    "entity_is_crouching",
    "entity_view_vector",
    "entity_up_vector",
    "entity_level",
    "entity_vehicle",
    "entity_is_passenger",
    "entity_sound_source",
    "entity_leash_holder",
    "entity_pick",
    "living_attribute_value",
    "living_attribute_base",
    "living_visit_attributes",
    "living_visit_attribute_modifiers",
    "living_used_item_hand",
    "mob_can_use_portal",
    "mob_target",
    "mob_raid",
    "entity_custom_name_visible",
    "entity_classification",
    "entity_permanently_invulnerable",
    "entity_invulnerable_time",
    "entity_damage_cooldown_time",
    "entity_invulnerable_to_piercing_weapon",
    "entity_no_physics",
    "entity_spectator",
    "entity_is_shift_key_down",
    "entity_is_sprinting",
    "entity_is_in_water",
    "entity_is_in_lava",
    "entity_fire_immune",
    "entity_fire_ticks",
    "entity_air_supply",
    "entity_max_air_supply",
    "entity_is_silent",
    "entity_glowing",
    "entity_no_gravity",
    "entity_ticks_frozen",
    "entity_tick_count",
    "entity_is_invisible",
    "entity_is_multipart",
    "entity_visit_parts",
    "entity_dimensions",
    "living_effect",
    "living_visit_effects",
    "living_potion_affected",
    "living_gliding_ticks",
    "living_is_gliding",
    "living_is_sleeping",
    "living_sleeping_pos",
    "living_is_jumping",
    "living_no_jump_delay",
    "living_baby",
    "living_equipment",
    "living_hand_item",
    "living_is_tamed",
    "living_owner",
    "living_dimensions_for_pose",
    "living_can_freeze",
    "living_can_breathe_underwater",
    "living_has_line_of_sight",
    "living_in_melee_range",
    "living_bed_orientation",
    "mob_goals",
    "mob_target_goals",
    "mob_is_done",
    "mob_navigation_speed",
    "mob_pathfinding_malus",
    "mob_is_aggressive",
    "mob_is_in_sitting_pose",
    "mob_persistence_required",
    "mob_requires_custom_persistence",
    "mob_no_ai",
    "mob_restrict_center",
    "mob_restrict_radius",
    "mob_has_restriction",
    "mob_is_within_restriction",
    "mob_left_handed",
    "mob_can_pick_up_loot",
    "mob_no_action_time",
];

/// The handle types whose backend reads are entity data (#1077): a read taking
/// one first answers the tick's own view of that entity — the snapshot, the
/// player the tick sees — and is refused where that carries it not. Every
/// other backend read (a level, a chunk, a block entity, a registry key)
/// names data the tick leaves in place, so the view forwards it to the real
/// game backend instead of refusing it.
const GOAL_VIEW_ENTITY_ARGS: &[&str] = &[
    "EntityRef",
    "LivingRef",
    "MobRef",
    "PlayerRef",
    "ItemEntityRef",
    "ExperienceOrbRef",
    "MerchantRef",
];

/// The `Host` methods that find entities (#1077): the goal view answers these
/// by its own rules (the ticking mob, the player the tick sees, the snapshot,
/// else `None`) instead of forwarding them, so nothing asks the game's taken
/// mob list. Everything else the view forwards to the game host it carries.
pub(super) const GOAL_VIEW_LOOKUPS: &[&str] = &[
    "entity",
    "living",
    "mob",
    "player",
    "item_entity",
    "block_entity",
];

pub(super) const DIRECTORY: &str = "crates/sunrise-mod-vocab/src/host";
/// The generated file `sunrise-mod-api/src/host.rs` includes: the area modules and the prelude.
pub(super) const AREAS_OUTPUT: &str = "mod_api_host_areas.rs";

/// The traits an area file may hold, each a part of one of the unions.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Tier {
    Host,
    ServerHost,
    ClientHost,
    Backend,
    ClientBackend,
    BackendMut,
    ClientBackendMut,
}

impl Tier {
    const ALL: [Tier; 7] = [
        Tier::Host,
        Tier::ServerHost,
        Tier::ClientHost,
        Tier::Backend,
        Tier::ClientBackend,
        Tier::BackendMut,
        Tier::ClientBackendMut,
    ];
    fn suffix(self) -> &'static str {
        match self {
            Tier::Host => "Host",
            Tier::ServerHost => "ServerHost",
            Tier::ClientHost => "ClientHost",
            Tier::Backend => "Backend",
            Tier::ClientBackend => "ClientBackend",
            Tier::BackendMut => "BackendMut",
            Tier::ClientBackendMut => "ClientBackendMut",
        }
    }
    /// The union trait this one is a part of.
    fn union(self) -> &'static str {
        match self {
            Tier::ClientBackend => "Backend",
            tier => tier.suffix(),
        }
    }
    fn mutable(self) -> bool {
        matches!(self, Tier::BackendMut | Tier::ClientBackendMut)
    }
    fn client(self) -> bool {
        matches!(
            self,
            Tier::ClientHost | Tier::ClientBackend | Tier::ClientBackendMut
        )
    }
    fn backend(self) -> bool {
        matches!(
            self,
            Tier::Backend | Tier::ClientBackend | Tier::BackendMut | Tier::ClientBackendMut
        )
    }
}

/// A method of an area trait, as the file declares it.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) sig: syn::Signature,
    pub(crate) attrs: Vec<syn::Attribute>,
}

impl Entry {
    /// `#[goal_view]`: a mob's goal tick answers this read from its own view for every holder,
    /// players included, and resolves the entity the answer names there too, so a mob lent to
    /// the tick stays visible. Unmarked reads in [`GOAL_VIEW_READS`] send players to the game.
    fn goal_view(&self) -> bool {
        self.attrs.iter().any(|a| a.path().is_ident("goal_view"))
    }

    /// `#[server_only]`: a `Host` or `Backend` method that only the logical server implements;
    /// `ClientGame` answers with its typed refusal instead of calling an inherent method.
    fn server_only(&self) -> bool {
        self.attrs.iter().any(|a| a.path().is_ident("server_only"))
    }

    /// The `Side: server.` / `Side: client.` / `Side: both.` marker in the method's doc comments.
    fn doc_side(&self) -> Option<Side> {
        for attr in &self.attrs {
            if !attr.path().is_ident("doc") {
                continue;
            }
            if let syn::Meta::NameValue(nv) = &attr.meta
                && let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
            {
                let line = s.value();
                if line.contains("Side: server.") {
                    return Some(Side::Server);
                }
                if line.contains("Side: client.") {
                    return Some(Side::Client);
                }
                if line.contains("Side: both.") {
                    return Some(Side::Both);
                }
            }
        }
        None
    }
}

/// One area file: its traits by tier.
pub(crate) struct Area {
    pub(crate) stem: String,
    pub(crate) camel: String,
    pub(crate) tiers: BTreeMap<Tier, Vec<Entry>>,
    /// The backend tiers the game answers from a borrowed view of its read-only tables, not
    /// from `Game`, declared by `#[table_backend("path")]` on the trait: the view carries the
    /// reads and implements the trait once, and `Game` forwards to it.
    tables: BTreeMap<Tier, String>,
    /// Tiers carrying `#[server_only]` on the trait itself.
    server_only_tiers: std::collections::BTreeSet<Tier>,
}

impl Area {
    fn trait_name(&self, tier: Tier) -> String {
        format!("{}{}", self.camel, tier.suffix())
    }
    fn trait_path(&self, tier: Tier) -> String {
        format!("crate::host::{}::{}", self.stem, self.trait_name(tier))
    }
}

/// The traits of one area file by tier. Only `<Camel><Tier>` and the explicitly listed draw
/// contexts and services are accepted: an unknown trait cannot be reached through the Host.
/// Imports and file/item attributes are allowed, but modules, macros and other items are refused
/// rather than hiding declarations from this scan.
fn area_tiers(camel: &str, file: syn::File) -> Result<BTreeMap<Tier, Vec<Entry>>, String> {
    let mut tiers = BTreeMap::new();
    for item in file.items {
        let item = match item {
            Item::Use(_) => continue,
            Item::Trait(item) => item,
            other => {
                let kind = match &other {
                    Item::Mod(_) => "module",
                    Item::Macro(_) => "macro",
                    _ => "item",
                };
                return Err(format!(
                    "unsupported {kind} in host area `{camel}`: only imports and declared traits are allowed, found `{}`",
                    other.to_token_stream()
                ));
            }
        };
        let name = item.ident.to_string();
        let Some(tier) = Tier::ALL
            .into_iter()
            .find(|tier| name == format!("{camel}{}", tier.suffix()))
        else {
            if matches!(
                (camel, name.as_str()),
                (
                    "Draw",
                    "DrawContext"
                        | "BlockEntityDrawContext"
                        | "WorldDrawContext"
                        | "CameraDrawContext"
                        | "WorldPassDrawContext"
                        | "WorldOitPassDrawContext"
                        | "RenderLevelStageContext"
                        | "PrepareRenderBuffersContext"
                        | "FrameGraphContext"
                        | "OffscreenDrawContext"
                ) | (
                    "Services",
                    "ClientSession"
                        | "Commands"
                        | "Levels"
                        | "Messages"
                        | "Players"
                        | "Profiles"
                        | "RuntimeLevels"
                        | "RenderAssets"
                        | "InstanceGroups"
                )
            ) {
                continue;
            }
            return Err(format!(
                "unrecognised trait `{name}`: expected `{camel}<Host|ServerHost|ClientHost|Backend|ClientBackend|BackendMut|ClientBackendMut>` or an explicitly listed draw context or service"
            ));
        };
        let entries = item
            .items
            .into_iter()
            .filter_map(|member| match member {
                syn::TraitItem::Fn(method) => Some(Entry {
                    name: method.sig.ident.to_string(),
                    sig: method.sig,
                    attrs: method.attrs,
                }),
                _ => None,
            })
            .collect();
        tiers.insert(tier, entries);
    }
    Ok(tiers)
}

/// The `#[table_backend("path")]` declarations of an area file: only a `Backend` trait may carry
/// one, since the game answers only its reads from a view.
fn table_backends(camel: &str, file: &syn::File) -> Result<BTreeMap<Tier, String>, String> {
    let mut tables = BTreeMap::new();
    for item in &file.items {
        let Item::Trait(item) = item else { continue };
        for attr in &item.attrs {
            if !attr.path().is_ident("table_backend") {
                continue;
            }
            if item.ident != format!("{camel}Backend") {
                return Err(format!(
                    "`#[table_backend]` belongs on `{camel}Backend`, not `{}`",
                    item.ident
                ));
            }
            let path = attr
                .parse_args::<syn::LitStr>()
                .map_err(|e| format!("`#[table_backend]` takes a path string: {e}"))?;
            tables.insert(Tier::Backend, path.value());
        }
    }
    Ok(tables)
}

/// Tiers carrying `#[server_only]` on the trait declaration itself.
fn server_only_tiers(camel: &str, file: &syn::File) -> std::collections::BTreeSet<Tier> {
    let mut set = std::collections::BTreeSet::new();
    for item in &file.items {
        let Item::Trait(item) = item else { continue };
        if !item.attrs.iter().any(|a| a.path().is_ident("server_only")) {
            continue;
        }
        let name = item.ident.to_string();
        if let Some(tier) = Tier::ALL
            .into_iter()
            .find(|tier| name == format!("{camel}{}", tier.suffix()))
        {
            set.insert(tier);
        }
    }
    set
}

/// The area files, `base` first and then by name, as the catalog's are.
/// The area files, sorted by name, as the catalog's are.
pub(crate) fn load(root: &Path) -> Result<Vec<Area>, String> {
    let dir = root.join(DIRECTORY);
    let mut areas = Vec::new();
    for stem in sunrise_game_discovery::module_files(&dir)? {
        let path = dir.join(format!("{stem}.rs"));
        crate::rerun_if_changed(&path.display());
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        let camel = stem.to_upper_camel_case();
        let tables =
            table_backends(&camel, &file).map_err(|e| format!("{}: {e}", path.display()))?;
        let server_only_tiers = server_only_tiers(&camel, &file);
        let tiers = area_tiers(&camel, file).map_err(|e| format!("{}: {e}", path.display()))?;
        areas.push(Area {
            stem,
            camel,
            tiers,
            tables,
            server_only_tiers,
        });
    }
    crate::rerun_if_changed(&dir.display());
    Ok(areas)
}

/// The area modules, the prelude that names every trait of them, and one macro for each union of
/// them (`impl_host_areas!`, `impl_server_host_areas!`...) that implements the area traits of that
/// union with their defaults, except the areas a double overrides: what a test double or a startup
/// host writes instead of an `impl` for each area.
pub(super) fn areas_source(root: &Path) -> Result<String, String> {
    let areas = load(root)?;
    let dir = root.join(DIRECTORY);
    let mut modules = String::new();
    let mut prelude = String::new();
    for area in &areas {
        let path =
            fs::canonicalize(dir.join(format!("{}.rs", area.stem))).map_err(|e| e.to_string())?;
        modules.push_str(&format!("#[path = {path:?}] pub mod {};\n", area.stem));
        prelude.push_str(&format!("pub use super::{}::*;\n", area.stem));
    }
    // One helper per area: it expands its item unless the area is in the skip list.
    // Rustc rejects public re-exports of non-exported macro_rules macros. Export
    // hidden __mod_api_* implementations and re-export their caller-facing names
    // unqualified: absolute paths to include!-expanded macro_export definitions
    // otherwise trigger macro_expanded_macro_exports_accessed_by_absolute_paths.
    let mut macros = String::new();
    for area in &areas {
        macros.push_str(&format!(
            "#[doc(hidden)]\n#[macro_export]\nmacro_rules! __mod_api_skip_{stem} {{ ([] $body:item) => {{ $body }}; ([{stem} $($rest:ident)*] $body:item) => {{}}; ([$other:ident $($rest:ident)*] $body:item) => {{ $crate::__skip_{stem}!([$($rest)*] $body); }}; }}\n#[doc(hidden)]\npub use __mod_api_skip_{stem} as __skip_{stem};\n",
            stem = area.stem
        ));
    }
    // A partial read backend advertises the backend but can only answer some of it.
    // Forward the named methods, and let the API's standard no-backend value handle
    // the rest.
    let mut choices = String::new();
    let mut partial_impls = String::new();
    let mut read_names = std::collections::BTreeSet::new();
    for area in &areas {
        for tier in [Tier::Backend, Tier::ClientBackend] {
            let Some(entries) = area.tiers.get(&tier) else {
                continue;
            };
            let trait_name = area.trait_name(tier);
            writeln!(
                partial_impls,
                "#[deny(unconditional_recursion)] impl $crate::prelude::{trait_name} for $host {{"
            )
            .unwrap();
            for entry in entries {
                let name = entry.name.as_str();
                if read_names.insert(name) {
                    writeln!(choices, "(@known {name}) => {{}};\n([{name} $($tail:ident)*] {name} $forward:item {{$($refused:item)*}}) => {{ $forward }};").unwrap();
                }
                let sig = entry.sig.to_token_stream();
                let args = signature_args(&entry.sig, name)?;
                let call_args = if args.is_empty() {
                    "self".to_owned()
                } else {
                    format!("self, {args}")
                };
                let value = refusal(entry)?;
                let refused = if args.is_empty() {
                    format!("{sig} {{ {value} }}")
                } else {
                    format!("{sig} {{ let _ = ({args},); {value} }}")
                };
                writeln!(partial_impls, "$crate::__partial_read!([$($provided)*] {name} {sig} {{ Self::{name}({call_args}) }} {{ {refused} }});").unwrap();
            }
            partial_impls.push_str("}\n");
        }
    }
    writeln!(macros, r#"#[doc(hidden)] #[macro_export] macro_rules! __mod_api_partial_read {{
        {choices}
        (@known $name:ident) => {{ compile_error!(concat!("unknown backend read: ", stringify!($name))); }};
        ([$other:ident $($tail:ident)*] $name:ident $forward:item {{$($refused:item)*}}) => {{
            $crate::__partial_read!([$($tail)*] $name $forward {{$($refused)*}});
        }};
        ([] $name:ident $forward:item {{$($refused:item)*}}) => {{ $($refused)* }};
    }}
    #[doc(hidden)] pub use __mod_api_partial_read as __partial_read;"#).unwrap();
    let partial = format!(
        "($host:ty, partial($($provided:ident),* $(,)?)) => {{
        const _: () = {{
            use $crate::*;
            use ::std::sync::Arc;
            $($crate::__partial_read!(@known $provided);)*
            {partial_impls}
        }};
    }};"
    );
    for (name, tiers, union) in [
        ("impl_host_areas", &[Tier::Host][..], "Host"),
        (
            "impl_server_host_areas",
            &[Tier::ServerHost][..],
            "ServerHost",
        ),
        (
            "impl_client_host_areas",
            &[Tier::ClientHost][..],
            "ClientHost",
        ),
        (
            "impl_backend_areas",
            &[Tier::Backend, Tier::ClientBackend][..],
            "Backend",
        ),
        (
            "impl_backend_mut_areas",
            &[Tier::BackendMut][..],
            "BackendMut",
        ),
        (
            "impl_client_backend_mut_areas",
            &[Tier::ClientBackendMut][..],
            "ClientBackendMut",
        ),
    ] {
        let impls: String = areas
            .iter()
            .flat_map(|area| tiers.iter().map(move |tier| (area, *tier)))
            .filter(|(area, tier)| area.tiers.contains_key(tier))
            .map(|(area, tier)| {
                format!(
                    "$crate::__skip_{}!([$($($skip)*)?] impl $crate::prelude::{} for $host {{}});\n",
                    area.stem,
                    area.trait_name(tier)
                )
            })
            .collect();
        let (partial, partial_doc) = if name == "impl_backend_areas" {
            (
                partial.as_str(),
                "/// `partial(method, ...)` forwards the listed reads to inherent methods and answers\n/// the ordinary no-backend value for other reads that can refuse.\n",
            )
        } else {
            ("", "")
        };
        macros.push_str(&format!(
            "/// Implements the area traits of `{union}` for `$host` with their defaults, except the areas listed in `except(...)`, which the caller implements itself. The union trait is left to the caller.\n{partial_doc}#[doc(hidden)]\n#[macro_export]\nmacro_rules! __mod_api_{name} {{ ($host:ty $(, except($($skip:ident),*))?) => {{ {impls} }}; {partial} }}\n#[doc(hidden)]\npub use __mod_api_{name} as {name};\n"
        ));
    }
    Ok(format!(
        "{modules}/// Every host trait of every area: bring it into scope to implement the game's side or a test double.\npub mod prelude {{\n{prelude}}}\n{macros}"
    ))
}

#[derive(Clone, Copy)]
enum ForwarderTarget<'a> {
    Table(&'a str),
    Server,
    Client,
}

impl<'a> ForwarderTarget<'a> {
    fn type_name(self) -> &'a str {
        match self {
            Self::Table(path) => path,
            Self::Server => "ServerGame",
            Self::Client => "ClientGame",
        }
    }
}

struct ForwarderImpl<'a> {
    trait_name: String,
    target: ForwarderTarget<'a>,
    methods: Vec<(&'a Entry, ForwarderCall<'a>)>,
}

enum ForwarderCall<'a> {
    Published(syn::Path),
    Refusal,
    Service,
    Table(&'a str),
    Routed(&'a HostRoute),
    Inherent {
        owner: &'a str,
        receiver: Option<ForwarderBorrow>,
    },
}

#[derive(Clone, Copy)]
enum ForwarderBorrow {
    Shared,
    Mutable,
}

fn game_method(entry: &Entry) -> Result<Cow<'_, str>, String> {
    match entry
        .attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("game_method"))
    {
        Some(attribute) => attribute
            .parse_args::<syn::Ident>()
            .map(|method| Cow::Owned(method.to_string()))
            .map_err(|error| format!("{}: {error}", entry.name)),
        None => Ok(Cow::Borrowed(&entry.name)),
    }
}

fn forwarder_plan(areas: &[Area]) -> Result<Vec<ForwarderImpl<'_>>, String> {
    let mut plan = Vec::new();
    for area in areas {
        for (tier, entries) in &area.tiers {
            let table = area.tables.get(tier).map(String::as_str);
            let tier_server_only = area.server_only_tiers.contains(tier);
            let targets: Vec<ForwarderTarget<'_>> = match (table, tier) {
                (Some(tbl), _) => vec![
                    ForwarderTarget::Table(tbl),
                    ForwarderTarget::Server,
                    ForwarderTarget::Client,
                ],
                (None, Tier::ServerHost | Tier::BackendMut) => vec![ForwarderTarget::Server],
                (None, Tier::ClientHost | Tier::ClientBackendMut) => vec![ForwarderTarget::Client],
                (None, Tier::Host | Tier::Backend | Tier::ClientBackend) => {
                    vec![ForwarderTarget::Server, ForwarderTarget::Client]
                }
            };
            for target_kind in targets {
                let methods = entries
                    .iter()
                    .map(|entry| {
                        if let Some(published) =
                            entry.attrs.iter().find(|a| a.path().is_ident("published"))
                        {
                            let path = published
                                .parse_args::<syn::Path>()
                                .map_err(|e| format!("{}: {e}", entry.name))?;
                            return Ok((entry, ForwarderCall::Published(path)));
                        }
                        let refuse_on_this_side = match (tier, target_kind) {
                            (Tier::ClientBackend, ForwarderTarget::Server) => true,
                            (Tier::Host | Tier::Backend, ForwarderTarget::Client) => {
                                tier_server_only
                                    || entry.server_only()
                                    || entry.doc_side() == Some(Side::Server)
                            }
                            (Tier::Host | Tier::Backend, ForwarderTarget::Server) => {
                                entry.doc_side() == Some(Side::Client)
                            }
                            _ => false,
                        };
                        let call = if refuse_on_this_side {
                            ForwarderCall::Refusal
                        } else if entry.attrs.iter().any(|a| a.path().is_ident("service")) {
                            ForwarderCall::Service
                        } else if let Some(table) =
                            table.filter(|_| !matches!(target_kind, ForwarderTarget::Table(_)))
                        {
                            ForwarderCall::Table(table)
                        } else {
                            let receiver = (!entry
                                .attrs
                                .iter()
                                .any(|a| a.path().is_ident("static_call")))
                            .then(|| match entry.sig.receiver().and_then(|r| r.mutability) {
                                Some(_) => ForwarderBorrow::Mutable,
                                None => ForwarderBorrow::Shared,
                            });
                            ForwarderCall::Inherent {
                                owner: target_kind.type_name(),
                                receiver,
                            }
                        };
                        Ok((entry, call))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                plan.push(ForwarderImpl {
                    trait_name: area.trait_name(*tier),
                    target: target_kind,
                    methods,
                });
            }
        }
    }
    Ok(plan)
}

/// Generate every area impl explicitly. Full-path calls plus a hard recursion lint
/// refuse a missing inherent implementation instead of inheriting a trait default.
pub(super) fn game_forwarders(root: &Path) -> Result<String, String> {
    let areas = load(root)?;
    render_forwarders(&forwarder_plan(&areas)?, None)
}

fn render_forwarders(
    plan: &[ForwarderImpl<'_>],
    split: Option<(&Routing, GameplayPackage)>,
) -> Result<String, String> {
    let mut code = String::from(
        "#[cfg(feature = \"client\")]\nuse crate::game::ClientGame;\nuse crate::game::ServerGame;\nuse crate::game::api_values::{call_stack_write, call_state_write, call_write, TryIntoApi, TryIntoEngine};\nuse mod_api::*;\n",
    );
    if split.is_some() {
        code.clear();
    }
    for ForwarderImpl {
        trait_name,
        target,
        methods,
    } in plan
    {
        if split.is_none() && matches!(target, ForwarderTarget::Client) {
            code.push_str("#[cfg(feature = \"client\")]\n");
        }
        let target = match (split, target) {
            (Some(_), ForwarderTarget::Server | ForwarderTarget::Client) => {
                Cow::Owned(format!("crate::game::{}", target.type_name()))
            }
            _ => Cow::Borrowed(target.type_name()),
        };
        writeln!(code, "#[deny(unconditional_recursion)]\nimpl mod_api::prelude::{trait_name} for {target}<'_> {{").unwrap();
        for (entry, route) in methods {
            let name = &entry.name;
            let sig = entry.sig.to_token_stream();
            match route {
                ForwarderCall::Published(read) => {
                    let args = signature_args(&entry.sig, name)?;
                    writeln!(
                        code,
                        "{sig} {{ mod_api::__private::{}({args}) }}",
                        read.to_token_stream()
                    )
                    .unwrap();
                    continue;
                }
                ForwarderCall::Refusal => {
                    let args = signature_args(&entry.sig, name)?;
                    let value = side_refusal(entry)?;
                    if args.is_empty() {
                        writeln!(code, "{sig} {{ {value} }}").unwrap();
                    } else {
                        writeln!(code, "{sig} {{ let _ = ({args},); {value} }}").unwrap();
                    }
                    continue;
                }
                _ => {}
            }
            let keeps_api_inputs = entry
                .attrs
                .iter()
                .any(|a| a.path().is_ident("forward_api_inputs"));
            let (setup, args, checked_write) = if keeps_api_inputs {
                (String::new(), signature_args(&entry.sig, name)?, false)
            } else {
                let read_stack = entry.attrs.iter().any(|a| a.path().is_ident("read_stack"));
                let serialize_text = entry
                    .attrs
                    .iter()
                    .any(|a| a.path().is_ident("serialize_text"));
                let items = if let Some((routing, package)) = split
                    && package != GameplayPackage::Common
                {
                    routing
                        .items
                        .get(&package)
                        .map(|route| route.call(package, "items", ""))
                        .transpose()?
                        .map(Cow::Owned)
                } else {
                    Some(Cow::Borrowed("Self::items(self)"))
                };
                let common = split.map_or("crate", |(_, package)| package.common());
                boundary_arguments_with(
                    &entry.sig,
                    name,
                    read_stack,
                    serialize_text.then_some(if split.is_some() {
                        "sunrise_sim::api_text::to_serialized_engine"
                    } else {
                        "sunrise_client_lib::api_text::to_serialized_engine"
                    }),
                    common,
                    items.as_deref(),
                )?
            };
            let method = game_method(entry)?;
            let call = match route {
                ForwarderCall::Service => "self".into(),
                ForwarderCall::Table(table) => {
                    if let Some((_, package)) = split {
                        let table = split_table(package, table)?;
                        let args = if args.is_empty() {
                            String::new()
                        } else {
                            format!(", {args}")
                        };
                        format!("{table}::{name}(&{table}::of(&self.common){args})")
                    } else {
                        format!("{table}::of(self).{name}({args})")
                    }
                }
                ForwarderCall::Routed(route) => {
                    route.call(split.expect("routed layout").1, &method, &args)?
                }
                ForwarderCall::Inherent { owner, receiver } => {
                    let owner = if split.is_some() && !owner.contains("::") {
                        Cow::Owned(format!("crate::game::{owner}"))
                    } else {
                        Cow::Borrowed(*owner)
                    };
                    let args = if receiver.is_none() {
                        args
                    } else if args.is_empty() {
                        "self".into()
                    } else {
                        format!("self, {args}")
                    };
                    let method = if matches!(
                        owner.as_ref(),
                        "ServerGame"
                            | "ClientGame"
                            | "crate::game::ServerGame"
                            | "crate::game::ClientGame"
                    ) {
                        &*method
                    } else {
                        name
                    };
                    format!("{owner}::{method}({args})")
                }
                ForwarderCall::Published(_) | ForwarderCall::Refusal => unreachable!(),
            };
            let keeps_api_output = entry
                .attrs
                .iter()
                .any(|a| a.path().is_ident("forward_api_output"));
            let converted_output = match &entry.sig.output {
                syn::ReturnType::Type(_, ty) if !keeps_api_output => Some(ty.as_ref()),
                _ => None,
            };
            let crosses_output = converted_output.is_some_and(boundary_value);
            let text_output = converted_output.is_some_and(text_value);
            let text_result = result_error(&entry.sig.output)
                .is_some_and(|error| outer_type(error, "TextFormError"));
            let call = if text_output {
                if text_result {
                    format!("{call}.try_into_api()")
                } else {
                    format!("{call}.try_into_api().unwrap_or_else(|error| Err(error.into()))")
                }
            } else if text_result {
                // Writes with converted Text arguments call an infallible engine
                // body. Read backends return Result so converting API-owned
                // aggregate fields can refuse without losing a boundary error.
                let text_input =
                    !keeps_api_inputs
                        && entry.sig.inputs.iter().any(
                            |arg| matches!(arg, syn::FnArg::Typed(arg) if text_value(&arg.ty)),
                        );
                let read = entry
                    .sig
                    .receiver()
                    .is_some_and(|receiver| receiver.mutability.is_none());
                if text_input && !read {
                    if matches!(result_types(&entry.sig.output), Some((Type::Tuple(unit), _)) if unit.elems.is_empty())
                    {
                        format!("{{ {call}; Ok(()) }}")
                    } else {
                        format!("Ok({call})")
                    }
                } else {
                    call
                }
            } else if checked_write {
                if converted_output.is_some_and(|ty| contains_value(ty, &["BlockState"]))
                    && result_error(&entry.sig.output)
                        .is_some_and(|error| outer_type(error, "LevelWriteError"))
                {
                    format!("call_state_write(|| {call})")
                } else if crosses_output {
                    format!("call_stack_write(|| {call})")
                } else {
                    format!("call_write(|| {call})")
                }
            } else if crosses_output {
                match result_error(&entry.sig.output) {
                    Some(error) if outer_type(error, "LevelReadError") => format!(
                        "{call}.try_into_api().unwrap_or(Err(mod_api::LevelReadError::Unavailable))"
                    ),
                    // The placement-state read refuses an unrepresentable state as
                    // other level reads do.
                    Some(error) if outer_type(error, "PlacementStateError") => format!(
                        "{call}.try_into_api().unwrap_or(Err(mod_api::PlacementStateError::Unavailable))"
                    ),
                    Some(error) if outer_type(error, "GameRuleReadError") => format!(
                        "{call}.try_into_api().expect(\"game rule values convert infallibly\")"
                    ),
                    Some(_) if split.is_some() => format!("call_stack_write(|| {call})"),
                    Some(_) => format!(
                        "{call}.try_into_api().unwrap_or_else(|error| Err(error.into_write_error().into()))"
                    ),
                    None => format!("{call}.try_into_api().unwrap_or_default()"),
                }
            } else {
                call
            };
            let call = match name.as_str() {
                "schedule_task" | "cancel_task" => call,
                _ if matches!(route, ForwarderCall::Service)
                    || borrows_caller_callback(&entry.sig) =>
                {
                    call
                }
                _ => format!(
                    "mod_api::__private::game::CallbackOwner::call_host(move || {{ {call} }})"
                ),
            };
            code.push_str(&format!("#[inline]\n{sig} {{ {setup} {call} }}\n"));
        }
        code.push_str("}\n");
    }
    let parsed = syn::parse_file(&code).map_err(|e| e.to_string())?;
    let mut imports = standard_imports(&parsed);
    if let Some((_, package)) = split {
        let names = [
            "call_stack_write",
            "call_state_write",
            "call_write",
            "TryIntoApi",
            "TryIntoEngine",
        ];
        let mut tokens = std::collections::BTreeSet::new();
        fn identifiers(
            stream: proc_macro2::TokenStream,
            tokens: &mut std::collections::BTreeSet<String>,
        ) {
            for token in stream {
                match token {
                    proc_macro2::TokenTree::Group(group) => identifiers(group.stream(), tokens),
                    proc_macro2::TokenTree::Ident(ident) => {
                        tokens.insert(ident.to_string());
                    }
                    _ => {}
                }
            }
        }
        identifiers(parsed.to_token_stream(), &mut tokens);
        let imports_used = names
            .into_iter()
            .filter(|name| {
                let expression = match *name {
                    "TryIntoApi" => "try_into_api",
                    "TryIntoEngine" => "try_into_engine",
                    name => name,
                };
                tokens.contains(expression)
            })
            .collect::<Vec<_>>();
        if !imports_used.is_empty() {
            writeln!(
                imports,
                "use {}::game::api_values::{{{}}};",
                package.common(),
                imports_used.join(", ")
            )
            .unwrap();
        }
        if uses_vocab(&parsed) {
            imports.push_str("use mod_api::*;\n");
        }
    }
    let parsed = syn::parse_file(&(imports + &code)).map_err(|e| e.to_string())?;
    Ok(prettyplease::unparse(&parsed))
}

fn uses_vocab(file: &syn::File) -> bool {
    struct Types(bool);
    impl<'ast> Visit<'ast> for Types {
        fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
            if let Some(ident) = node.path.get_ident() {
                self.0 |= !matches!(
                    ident.to_string().as_str(),
                    "Self"
                        | "bool"
                        | "char"
                        | "str"
                        | "String"
                        | "Vec"
                        | "Option"
                        | "Result"
                        | "Arc"
                        | "PathBuf"
                ) && !crate::PRIMITIVE_SCALARS.contains(&ident.to_string().as_str());
            }
            syn::visit::visit_type_path(self, node);
        }

        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            self.0 |= node
                .path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "NoBackend");
        }
    }
    let mut types = Types(false);
    types.visit_file(file);
    types.0
}

fn split_table(package: GameplayPackage, table: &str) -> Result<String, String> {
    let relative = table
        .strip_prefix("crate::")
        .ok_or_else(|| format!("table path is not relative to common gameplay: {table}"))?;
    Ok(format!("{}::{relative}", package.common()))
}

pub(crate) fn gameplay_forwarders(
    areas: &[Area],
    package: GameplayPackage,
    routing: &Routing,
    audit: &mut GameplayAudit,
) -> Result<String, String> {
    render_forwarders(
        &gameplay_forwarder_plan(areas, package, routing, audit)?,
        Some((routing, package)),
    )
}

fn gameplay_forwarder_plan<'a>(
    areas: &'a [Area],
    package: GameplayPackage,
    routing: &'a Routing,
    audit: &mut GameplayAudit,
) -> Result<Vec<ForwarderImpl<'a>>, String> {
    let mut plan = forwarder_plan(areas)?;
    plan.retain(|implementation| match implementation.target {
        ForwarderTarget::Table(_) => package == GameplayPackage::Common,
        ForwarderTarget::Server => package == GameplayPackage::Server,
        ForwarderTarget::Client => package == GameplayPackage::Client,
    });
    for implementation in &mut plan {
        let trait_name = &implementation.trait_name;
        let target = implementation.target;
        let mut methods = Vec::new();
        for (entry, call) in std::mem::take(&mut implementation.methods) {
            let key = format!("{trait_name}::{}", entry.name);
            let selected = (|| {
                let (call, kind) = match call {
                    ForwarderCall::Published(_) => (call, "published"),
                    ForwarderCall::Refusal => (call, "refusal"),
                    ForwarderCall::Service => (call, "service"),
                    ForwarderCall::Table(table) => {
                        split_table(package, table)?;
                        (call, "table")
                    }
                    ForwarderCall::Inherent { receiver, .. } => {
                        if let ForwarderTarget::Table(table) = target {
                            split_table(package, table)?;
                            (call, "table")
                        } else if let Some(route) = routing.host_route(package, &key) {
                            let borrow = match receiver {
                                None => Borrow::None,
                                Some(ForwarderBorrow::Shared) => Borrow::Shared,
                                Some(ForwarderBorrow::Mutable) => Borrow::Mutable,
                            };
                            if route.borrow != borrow {
                                return Err(
                                    "declared callee borrows differently from the host signature"
                                        .into(),
                                );
                            }
                            route.call(package, &game_method(entry)?, "")?;
                            let kind = match route.kind {
                                RouteKind::Local => "local",
                                RouteKind::Common => "common_inherent",
                                RouteKind::SharedTrait => "shared_trait",
                                RouteKind::SideOperation => "side_operation",
                                RouteKind::Static => "static",
                            };
                            (ForwarderCall::Routed(route), kind)
                        } else if receiver.is_some() {
                            (call, "local_default")
                        } else {
                            return Err("static call requires an explicit owner".into());
                        }
                    }
                    ForwarderCall::Routed(_) => unreachable!(),
                };
                if !matches!(call, ForwarderCall::Published(_) | ForwarderCall::Refusal)
                    && !entry
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("forward_api_inputs"))
                {
                    let context = routing
                        .items
                        .get(&package)
                        .map(|route| route.call(package, "items", ""))
                        .transpose()?;
                    boundary_arguments_with(
                        &entry.sig,
                        &entry.name,
                        entry
                            .attrs
                            .iter()
                            .any(|attr| attr.path().is_ident("read_stack")),
                        entry
                            .attrs
                            .iter()
                            .any(|attr| attr.path().is_ident("serialize_text"))
                            .then_some("sunrise_sim::api_text::to_serialized_engine"),
                        package.common(),
                        if package == GameplayPackage::Common {
                            Some("Self::items(self)")
                        } else {
                            context.as_deref()
                        },
                    )?;
                }
                Ok((call, kind))
            })();
            match selected {
                Ok((call, kind)) => {
                    audit.count(package, &key, kind);
                    methods.push((entry, call));
                }
                Err(reason) => audit.issue(package, &key, "host", reason),
            }
        }
        implementation.methods = methods;
    }
    Ok(plan)
}

pub(crate) struct HostCallee {
    pub label: String,
    pub owner: String,
    pub method: String,
    pub kind: RouteKind,
    pub borrow: Borrow,
    pub arity: usize,
}

pub(crate) fn host_callees(
    areas: &[Area],
    package: GameplayPackage,
    routing: &Routing,
) -> Result<Vec<HostCallee>, String> {
    let mut audit = GameplayAudit::default();
    let plan = gameplay_forwarder_plan(areas, package, routing, &mut audit)?;
    if !audit.issues.is_empty() {
        return Err(audit
            .issues
            .iter()
            .map(|issue| format!("{}: {}", issue.entry, issue.reason))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let mut used = BTreeSet::new();
    let mut checks = Vec::new();
    let mut items_used = false;
    for implementation in plan {
        for (entry, call) in implementation.methods {
            let method = game_method(entry)?;
            let label = format!("{}::{}", implementation.trait_name, entry.name);
            let arity = entry
                .sig
                .inputs
                .iter()
                .filter(|arg| matches!(arg, FnArg::Typed(_)))
                .count();
            let mut note = |owner: String, method: &str, kind, borrow, arity| {
                checks.push(HostCallee {
                    label: label.clone(),
                    owner,
                    method: method.into(),
                    kind,
                    borrow,
                    arity,
                });
            };
            match call {
                ForwarderCall::Routed(route) => {
                    used.insert(label.clone());
                    note(
                        route.owner.clone(),
                        &method,
                        route.kind,
                        route.borrow,
                        arity,
                    );
                }
                ForwarderCall::Inherent { owner, receiver } => {
                    let owner = if owner.contains("::") {
                        owner.to_owned()
                    } else {
                        format!("crate::game::{owner}")
                    };
                    let borrow = match receiver {
                        None => Borrow::None,
                        Some(ForwarderBorrow::Shared) => Borrow::Shared,
                        Some(ForwarderBorrow::Mutable) => Borrow::Mutable,
                    };
                    note(owner, &method, RouteKind::Local, borrow, arity);
                }
                ForwarderCall::Table(owner) => {
                    let owner = split_table(package, owner)?;
                    note(owner.clone(), "of", RouteKind::Static, Borrow::None, 1);
                    note(owner, &entry.name, RouteKind::Local, Borrow::Shared, arity);
                }
                ForwarderCall::Service | ForwarderCall::Published(_) | ForwarderCall::Refusal => {
                    continue;
                }
            }
            if !entry
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("forward_api_inputs"))
            {
                let (setup, _, _) = boundary_arguments_with(
                    &entry.sig,
                    &entry.name,
                    entry
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("read_stack")),
                    entry
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("serialize_text"))
                        .then_some("sunrise_sim::api_text::to_serialized_engine"),
                    package.common(),
                    Some("__ITEMS_ROUTE_WITNESS__"),
                )?;
                items_used |= setup.contains("__ITEMS_ROUTE_WITNESS__");
                if package == GameplayPackage::Common
                    && let ForwarderTarget::Table(owner) = implementation.target
                    && setup.contains("__ITEMS_ROUTE_WITNESS__")
                {
                    note(owner.into(), "items", RouteKind::Local, Borrow::Shared, 0);
                }
            }
        }
    }
    if let Some(rows) = routing.host.get(&package) {
        for label in rows.keys() {
            if !used.contains(label) {
                return Err(format!("{label}: unused host route"));
            }
        }
    }
    if let Some(items) = routing.items.get(&package) {
        if !items_used || package == GameplayPackage::Common {
            return Err("items: unused conversion receiver route".into());
        }
        checks.push(HostCallee {
            label: "items".into(),
            method: "items".into(),
            owner: items.owner.clone(),
            kind: items.kind,
            borrow: items.borrow,
            arity: 0,
        });
    }
    Ok(checks)
}

/// API records whose game representation is owned by another crate. Conversions are
/// implemented once by the game; this rule only selects the boundary, never its behaviour.
/// Whether `sig` takes a borrowed callable the engine invokes synchronously on the caller's
/// behalf. Those methods stay inside the caller's retained panic scope so a panic in the
/// caller's closure still blames that caller.
fn borrows_caller_callback(sig: &syn::Signature) -> bool {
    sig.inputs.iter().any(|input| {
        let FnArg::Typed(arg) = input else {
            return false;
        };
        is_borrowed_callable(&arg.ty)
    })
}

fn is_borrowed_callable(mut ty: &Type) -> bool {
    while let Type::Paren(paren) = ty {
        ty = &paren.elem;
    }
    let Type::Reference(reference) = ty else {
        return false;
    };
    let mut elem = &*reference.elem;
    while let Type::Paren(paren) = elem {
        elem = &paren.elem;
    }
    matches!(
        elem,
        Type::TraitObject(object)
            if object.bounds.iter().any(|bound| {
                matches!(
                    bound,
                    syn::TypeParamBound::Trait(trait_bound)
                        if trait_bound.path.segments.last().is_some_and(|segment| {
                            matches!(
                                segment.ident.to_string().as_str(),
                                "Fn" | "FnMut" | "FnOnce"
                            )
                        })
                )
            })
    )
}

/// API records whose game representation is owned by another crate. Conversions are
/// implemented once by the game; this rule only selects the boundary, never its behaviour.
fn text_value(ty: &Type) -> bool {
    contains_value(ty, &["Text"])
}

fn boundary_value(ty: &Type) -> bool {
    contains_value(
        ty,
        &[
            "ItemStack",
            "BlockState",
            "GameRuleValue",
            "ItemResource",
            "ItemCost",
            "MerchantOffer",
            "Ingredient",
            "Recipe",
            "RecipeRule",
            "RecipeTemplate",
            "RecipeTrimPattern",
            "FireworkShapeIngredient",
            "ParticleOptions",
        ],
    )
}

fn contains_value(ty: &Type, names: &[&str]) -> bool {
    struct Values<'a>(bool, &'a [&'a str]);
    impl<'ast> Visit<'ast> for Values<'_> {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            self.0 |= path
                .path
                .segments
                .last()
                .is_some_and(|segment| self.1.contains(&segment.ident.to_string().as_str()));
            syn::visit::visit_type_path(self, path);
        }

        fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
            if matches!(&segment.arguments, syn::PathArguments::Parenthesized(_))
                && (segment.ident == "Fn" || segment.ident == "FnMut" || segment.ident == "FnOnce")
            {
                // The callee converts values before invoking the unchanged callback.
                return;
            }
            syn::visit::visit_path_segment(self, segment);
        }
    }
    let mut values = Values(false, names);
    values.visit_type(ty);
    values.0
}

fn result_error(output: &syn::ReturnType) -> Option<&Type> {
    result_types(output).map(|(_, error)| error)
}

fn result_types(output: &syn::ReturnType) -> Option<(&Type, &Type)> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    let Type::Path(path) = &**ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let (Some(syn::GenericArgument::Type(value)), Some(syn::GenericArgument::Type(error))) =
        (args.args.first(), args.args.last())
    else {
        return None;
    };
    Some((value, error))
}

fn outer_type(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == expected))
}

/// The conversion code for a host method's boundary arguments, the arguments to pass on, and
/// whether it is a checked write (`&mut self`, a `Result`, a converted value). A checked write
/// refuses an invalid value with its error; a read or comparison converts a borrowed stack
/// without write limits, and any other refused value makes the method return its default.
#[cfg(test)]
fn boundary_arguments(
    sig: &syn::Signature,
    method: &str,
    read_stack: bool,
    serialize_text: bool,
) -> Result<(String, String, bool), String> {
    boundary_arguments_with(
        sig,
        method,
        read_stack,
        serialize_text.then_some("sunrise_client_lib::api_text::to_serialized_engine"),
        "crate",
        Some("Self::items(self)"),
    )
}

fn boundary_arguments_with(
    sig: &syn::Signature,
    method: &str,
    read_stack: bool,
    text_serializer: Option<&str>,
    common: &str,
    items: Option<&str>,
) -> Result<(String, String, bool), String> {
    let mutable = sig
        .receiver()
        .is_some_and(|receiver| receiver.mutability.is_some());
    let typed = sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(arg) => Some(arg),
            _ => None,
        })
        .collect::<Vec<_>>();
    let checked_write = mutable
        && result_error(&sig.output).is_some()
        && typed
            .iter()
            .any(|arg| boundary_value(&arg.ty) && !contains_value(&arg.ty, &["ParticleOptions"]));
    let mut setup = String::new();
    let mut args = Vec::new();
    for arg in typed {
        let Pat::Ident(pat) = &*arg.pat else {
            return Err(format!("{method}: unsupported argument"));
        };
        let name = pat.ident.to_string();
        let text = text_value(&arg.ty);
        if !boundary_value(&arg.ty) && !text {
            args.push(name);
            continue;
        }
        let is_borrowed_stack = |ty: &Type| matches!(ty, Type::Reference(reference) if outer_type(&reference.elem, "ItemStack"));
        let optional_read_stack = read_stack
            && matches!(&*arg.ty, Type::Path(path)
            if outer_type(&arg.ty, "Option") && path.path.segments.last().is_some_and(|segment|
                matches!(&segment.arguments, syn::PathArguments::AngleBracketed(args)
                    if matches!(args.args.first(), Some(syn::GenericArgument::Type(ty)) if is_borrowed_stack(ty)))));
        let item_context = || {
            items.ok_or_else(|| {
                format!("{method}: boundary conversion needs an explicit items receiver route")
            })
        };
        let conversion = if let Some(serializer) = text_serializer.filter(|_| text) {
            let reference = if matches!(&*arg.ty, Type::Reference(_)) {
                ""
            } else {
                "&"
            };
            format!("{serializer}({reference}{name})")
        } else if text {
            format!("{name}.try_into_engine(&())")
        } else if optional_read_stack {
            format!("{name}.map({common}::game::api_values::stack_value).transpose()")
        } else if (!checked_write || read_stack) && is_borrowed_stack(&arg.ty) {
            // A detached stack used as input need not fit a resident slot.
            format!("{common}::game::api_values::stack_value({name})")
        } else if read_stack
            && matches!(&*arg.ty, Type::Reference(reference) if outer_type(&reference.elem, "ItemResource"))
        {
            format!("{common}::game::api_values::stack_value({name}.as_stack())")
        } else {
            format!("{name}.try_into_engine({})", item_context()?)
        };
        let value = if contains_value(&arg.ty, &["ParticleOptions"]) {
            format!(
                "match {name}.try_into_engine({}) {{ Ok(value) => value, Err(_) => return Default::default() }}",
                item_context()?
            )
        } else if text || checked_write {
            format!("{conversion}?")
        } else if result_error(&sig.output).is_some() {
            format!(
                "match {conversion} {{ Ok(value) => value, Err(_) => return Err(mod_api::NoBackend::no_backend()) }}"
            )
        } else {
            format!(
                "match {conversion} {{ Ok(value) => value, Err(_) => return Default::default() }}"
            )
        };
        setup.push_str(&format!("let {name} = {value};\n"));
        args.push(match &*arg.ty {
            Type::Reference(_) => format!("&{name}"),
            Type::Path(path) if outer_type(&arg.ty, "Option") => {
                let segment = path.path.segments.last().unwrap();
                let borrowed = matches!(&segment.arguments, syn::PathArguments::AngleBracketed(args)
                    if args.args.iter().any(|arg| matches!(arg, syn::GenericArgument::Type(Type::Reference(_)))));
                if borrowed { format!("{name}.as_ref()") } else { name }
            }
            _ => name,
        });
    }
    Ok((setup, args.join(", "), checked_write))
}

fn standard_imports(file: &syn::File) -> String {
    struct StdTypes {
        path_buf: bool,
        arc: bool,
    }
    impl<'ast> Visit<'ast> for StdTypes {
        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            if ty.path.segments.len() == 1 {
                self.path_buf |= ty.path.segments[0].ident == "PathBuf";
                self.arc |= ty.path.segments[0].ident == "Arc";
            }
            syn::visit::visit_type_path(self, ty);
        }
    }
    let mut std_types = StdTypes {
        path_buf: false,
        arc: false,
    };
    std_types.visit_file(file);
    let mut imports = String::new();
    if std_types.path_buf {
        imports.push_str("use std::path::PathBuf;\n");
    }
    if std_types.arc {
        imports.push_str("use std::sync::Arc;\n");
    }
    imports
}

fn signature_args(sig: &syn::Signature, name: &str) -> Result<String, String> {
    Ok(sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| match arg {
            FnArg::Typed(arg) => match &*arg.pat {
                Pat::Ident(pat) => Ok(pat.ident.to_string()),
                _ => Err(format!("{name}: unsupported argument")),
            },
            FnArg::Receiver(_) => Err(format!("{name}: unexpected receiver")),
        })
        .collect::<Result<Vec<_>, String>>()?
        .join(", "))
}

/// A backend read's typed answer where there is no backend to ask.
fn refusal(entry: &Entry) -> Result<&'static str, String> {
    refusal_with(entry, "Err(NoBackend::no_backend())")
}

fn side_refusal(entry: &Entry) -> Result<&'static str, String> {
    refusal_with(entry, "Err(NoBackend::wrong_side())")
}

fn refusal_with(entry: &Entry, result_err: &'static str) -> Result<&'static str, String> {
    let name = &entry.name;
    let syn::ReturnType::Type(_, ty) = &entry.sig.output else {
        if entry
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("decline_unavailable"))
        {
            return Ok("");
        }
        return Err(format!("{name}: a backend read returns a value"));
    };
    let syn::Type::Path(path) = &**ty else {
        return Err(format!("{name}: cannot refuse its backend return type"));
    };
    let last = path.path.segments.last().map(|seg| seg.ident.to_string());
    match last.as_deref() {
        Some("Option") => Ok("None"),
        Some("bool") => Ok("false"),
        Some("EntityResidency") => Ok("EntityResidency::Gone"),
        Some("Result") => Ok(result_err),
        Some("u64" | "u32" | "i32" | "u16" | "u8" | "usize") => Ok("0"),
        _ => Err(format!("{name}: cannot refuse its backend return type")),
    }
}

/// `entity_position(entity)` on a backend is also `EntityRef::position(self, host)`: the handle
/// a method is of, when its first argument is one and its name starts with the handle's.
/// `#[manual_handle]` opts out only from this inherent wrapper, not backend routing.
fn handle_method(
    entry: &Entry,
    tier: Tier,
    handles: &BTreeMap<String, String>,
) -> Result<Option<(String, String)>, String> {
    if entry
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("manual_handle"))
    {
        return Ok(None);
    }
    let name = entry.name.as_str();
    let mut inputs = entry.sig.inputs.iter();
    let mutable = matches!(inputs.next(), Some(FnArg::Receiver(r)) if r.mutability.is_some());
    let Some(FnArg::Typed(first)) = inputs.next() else {
        return Ok(None);
    };
    let Type::Path(first) = &*first.ty else {
        return Ok(None);
    };
    let Some(handle) = first.path.get_ident().map(|ident| ident.to_string()) else {
        return Ok(None);
    };
    let Some((receiver, _)) = handles.iter().find(|(_, ty)| **ty == handle) else {
        return Ok(None);
    };
    let Some(short) = name
        .strip_prefix(receiver.as_str())
        .and_then(|rest| rest.strip_prefix('_'))
    else {
        return Ok(None);
    };
    if mutable != tier.mutable() {
        return Err(format!("{name}: its receiver and its trait disagree"));
    }
    let mut sig = entry.sig.clone();
    sig.ident =
        syn::parse_str(short).map_err(|_| format!("{name}: {short} is not a method name"))?;
    let host: FnArg = match (mutable, tier.client()) {
        (true, true) => syn::parse_quote!(host: &mut dyn ClientHost),
        (true, false) => syn::parse_quote!(host: &mut dyn ServerHost),
        (false, true) => syn::parse_quote!(host: &dyn ClientHost),
        (false, false) => syn::parse_quote!(host: &dyn Host),
    };
    let mut args = String::new();
    let mut rest = vec![syn::parse_quote!(self), host];
    let mut transaction_arg = None;
    for arg in entry.sig.inputs.iter().skip(2) {
        let mut arg = arg.clone();
        let FnArg::Typed(typed) = &mut arg else {
            return Err(format!("{name}: unexpected receiver"));
        };
        let Pat::Ident(pat) = &mut *typed.pat else {
            return Err(format!("{name}: unsupported argument"));
        };
        pat.mutability = None;
        args.push_str(&format!(", {}", pat.ident));
        if matches!(&*typed.ty, Type::Reference(reference)
            if matches!(&*reference.elem, Type::Path(path) if path.path.is_ident("TransactionContext")))
        {
            if !mutable || tier.client() {
                return Err(format!(
                    "{name}: transaction loans require a mutable server host"
                ));
            }
            transaction_arg = Some(pat.ident.clone());
        }
        rest.push(arg);
    }
    sig.inputs = rest.into_iter().collect();
    let mut docs = doc_comments(&entry.attrs, name, true)?;
    // The reads a mob's goal tick answers from its view carry the snapshot rule where a mod reads
    // them (#1077).
    if entry.goal_view() {
        docs.push_str("///\n/// In a mob's goal tick ([`GoalContext::host`](crate::GoalContext::host)) this reads\n/// the tick's own view for every holder, and the entity it names resolves there too.\n");
    } else if GOAL_VIEW_READS.contains(&name) {
        docs.push_str("///\n/// In a mob's goal tick ([`GoalContext::host`](crate::GoalContext::host)) this reads\n/// the active mob or a disjoint live mob when available; player reads use the\n/// game host. Other unsupported reads retain their typed refusal.\n");
    }
    let backend = match (mutable, tier.client()) {
        (true, true) => "client_backend_mut_of",
        (true, false) => "backend_mut_of",
        _ => "backend_of",
    };
    let returns = |ident: &str| {
        matches!(&sig.output, syn::ReturnType::Type(_, ty) if matches!(&**ty, syn::Type::Path(path)
            if path.path.segments.last().is_some_and(|seg| seg.ident == ident)))
    };
    let check_transaction = if let Some(tx) = transaction_arg {
        let Some(error) = result_error(&sig.output) else {
            return Err(format!(
                "{name}: a transaction write needs a Result refusal"
            ));
        };
        format!(
            "if !host.transaction_matches(Some({tx})) {{ return Err(<{}>::InvalidContext); }} ",
            error.to_token_stream()
        )
    } else {
        String::new()
    };
    let call = format!("{name}(self{args})");
    let body = if returns("Option") {
        format!("{backend}(host)?.{call}")
    } else if returns("bool") {
        format!("{backend}(host).map(|backend| backend.{call}).unwrap_or(false)")
    } else if returns("CommandResult") || returns("EntityResidency") {
        // Without a backend, a command fails with no feedback and an entity is gone.
        format!("{backend}(host).map(|backend| backend.{call}).unwrap_or_default()")
    } else if returns("Result") {
        format!(
            "{backend}(host).map_or_else(|| Err(crate::NoBackend::no_backend()), |backend| backend.{call})"
        )
    } else {
        return Err(format!(
            "{name}: a {receiver} helper returns Option, bool, CommandResult, EntityResidency or Result, the value a host without a backend refuses"
        ));
    };
    Ok(Some((
        handle,
        format!(
            "{docs}pub {} {{ {check_transaction}{body} }}\n",
            sig.to_token_stream()
        ),
    )))
}

/// The unions, the handles' methods and the hidden backends' side of the host.
pub(super) fn generate(root: &Path) -> Result<String, String> {
    let areas = load(root)?;
    let api_source = fs::read_to_string(root.join("crates/sunrise-mod-vocab/src/lib.rs"))
        .map_err(|e| e.to_string())?;
    let api_source = super::exports::expand(root, &api_source)?;
    let mut types = api_types(&api_source)?;
    // Parse domain files separately: their inner docs and attributes belong to
    // that file, not to the middle of a concatenated API root.
    for entry in
        fs::read_dir(root.join("crates/sunrise-mod-vocab/src")).map_err(|e| e.to_string())?
    {
        let path = entry.map_err(|e| e.to_string())?.path().join("types.rs");
        if path.is_file() {
            let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let domain = api_types(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            for (receiver, name) in domain.handles {
                if types.handles.insert(receiver.clone(), name).is_some() {
                    return Err(format!("duplicate handle receiver {receiver}"));
                }
            }
        }
    }
    let supertraits = |union: &str| -> String {
        areas
            .iter()
            .flat_map(|area| {
                Tier::ALL
                    .into_iter()
                    .filter(|tier| tier.union() == union && area.tiers.contains_key(tier))
                    .map(|tier| area.trait_path(tier))
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let mut methods = String::new();
    let mut by_handle = BTreeMap::<String, String>::new();
    for area in &areas {
        for (tier, entries) in &area.tiers {
            for entry in entries {
                if tier.backend()
                    && let Some((handle, code)) = handle_method(entry, *tier, &types.handles)?
                {
                    by_handle.entry(handle).or_default().push_str(&code);
                }
            }
        }
    }
    for (handle, code) in &by_handle {
        methods.push_str(&format!("impl {handle} {{\n{code}}}\n"));
    }
    let imports = standard_imports(&syn::parse_file(&methods).map_err(|e| e.to_string())?);
    let side_enum = |name: &str, server: &str, client: &str| {
        format!(
            "/// The logical side of a hook's host.\n// exhaustive: a logical simulation runs on exactly one of these two sides.\n// no wire: borrows the active host; passed as call context.\npub enum {name}<'a> {{\n/// The hook fired on the server.\nServer({server}),\n/// The hook fired on the client.\nClient({client}) }}\n"
        )
    };
    let sides = format!(
        "{}{}{}",
        side_enum(
            "HostSide",
            "&'a mut dyn ServerHost",
            "&'a mut dyn ClientHost"
        ),
        side_enum("LevelSide", "&'a dyn LevelHost", "&'a dyn LevelHost"),
        side_enum(
            "LevelRandomSide",
            "&'a mut dyn LevelRandomHost",
            "&'a dyn LevelHost"
        )
    );
    let (host_supers, server_supers, client_supers) = (
        supertraits("Host"),
        supertraits("ServerHost"),
        supertraits("ClientHost"),
    );
    let (backend_supers, backend_mut_supers, client_backend_mut_supers) = (
        supertraits("Backend"),
        supertraits("BackendMut"),
        supertraits("ClientBackendMut"),
    );
    let host = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\n{imports}/// Services supplied by the game to trait mods.\n/// Each method's default panics with its name, so a test double implements only what its test calls; the game implements every one.\n/// The backend accessors are the exception: they default to None, so a double without a backend answers every handle's typed refusal.\n/// Its methods are declared by area in `host/<area>.rs`; `use mod_api::prelude::*` names every area trait.\npub trait Host: {host_supers} {{\n#[doc(hidden)] fn backend(&self) -> Option<BackendRef<'_>> {{ None }}\n#[doc(hidden)] fn capability_providers(&self) -> &'static dyn CapabilityProviders {{ &InstalledProviders }}\n}}\n/// What only the server's hooks are given: writes to the world.\npub trait ServerHost: Host + {server_supers} {{\n#[doc(hidden)] fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {{ None }}\n/// Runs `callback` inside one scoped transaction on this host, committing on `true` and aborting on `false` or unwind.\n/// Returns `Ok(true)` on commit and `Ok(false)` on callback abort. `Err` distinguishes a scope that could not open from a refused commit.\n/// Most callers use [`HostTransaction::transaction`], which wraps this in a typed `Result<T, TransactionError<E>>`.\nfn with_transaction_scope(&mut self, _callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext) -> bool) -> Result<bool, TransactionScopeError> {{ Err(TransactionScopeError::Unavailable) }}\n/// Runs `callback` inside one scoped transaction with a lent single-slot scratch [`ItemPlace`] initialised from `stack`, committing on `true` and aborting on `false` or unwind.\n/// Returns `Ok(Some(final_stack))` on commit, `Ok(None)` on abort, or a typed `Err` when opening or committing is refused or `stack` fails write validation.\n/// Most callers use [`HostTransaction::scratch_stack`], which wraps this in a typed `Result<(T, Option<ItemStack>), ScratchError<E>>`.\nfn with_scratch_stack_scope(&mut self, _stack: ItemStack, _callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext, &'loan ItemPlace) -> bool) -> Result<Option<Option<ItemStack>>, ScratchScopeError> {{ Err(ScratchScopeError::Transaction(TransactionScopeError::Unavailable)) }}\n/// Runs `callback` inside one scoped transaction with a lent two-slot scratch [`ItemPlace`] initialised with `stack` in its first slot, committing on `true` and aborting on `false` or unwind.\n/// Returns `Ok(Some((first, second)))` on commit, `Ok(None)` on abort, or a typed `Err` when opening or committing is refused or `stack` fails write validation.\n/// Most callers use [`HostTransaction::scratch_slots`], which wraps this in a typed `Result<(T, (Option<ItemStack>, Option<ItemStack>)), ScratchError<E>>`.\nfn with_scratch_slots_scope(&mut self, _stack: ItemStack, _callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext, &'loan ItemPlace) -> bool) -> Result<Option<(Option<ItemStack>, Option<ItemStack>)>, ScratchScopeError> {{ Err(ScratchScopeError::Transaction(TransactionScopeError::Unavailable)) }}\n/// Borrows the already-open innermost transaction without closing it or opening a new frame.\n/// Returns `Err(TransactionUnavailable)` if there is no transaction to borrow.\nfn with_current_transaction_scope(&mut self, _callback: &mut dyn for<'loan> FnMut(&'loan mut dyn ServerHost, &'loan TransactionContext)) -> Result<(), TransactionUnavailable> {{ Err(TransactionUnavailable) }}\n/// Whether `tx` names the innermost open transaction on this host (`None` matching when no transaction is open).\nfn transaction_matches(&self, tx: Option<&TransactionContext>) -> bool {{ tx.is_none() }}\n}}\n/// What only the client's hooks are given: the client's own services.\npub trait ClientHost: Host + {client_supers} {{\n#[doc(hidden)] fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {{ None }}\n}}\n{sides}#[doc(hidden)] pub trait Backend: {backend_supers} {{}}\n#[doc(hidden)] pub trait BackendMut: {backend_mut_supers} {{}}\n#[doc(hidden)] pub trait ClientBackendMut: {client_backend_mut_supers} {{}}\n{methods}"
    );
    syn::parse_file(&host).map_err(|e| format!("generated Host: {e}"))?;
    Ok(host)
}

pub(crate) fn generate_gameplay_goal_view(areas: &[Area]) -> Result<String, String> {
    goal_view_for(areas, true)
}

fn goal_view_for(areas: &[Area], split: bool) -> Result<String, String> {
    let mut reads = Vec::new();
    let mut host_methods = Vec::new();
    for area in areas {
        for (tier, entries) in &area.tiers {
            if matches!(tier, Tier::Backend | Tier::ClientBackend) {
                reads.extend(entries.iter().map(|entry| (area, *tier, entry)));
            } else if *tier == Tier::Host && area.stem != "services" {
                host_methods.extend(entries.iter().map(|entry| (area, entry)));
            }
        }
    }
    goal_view(areas, &reads, &host_methods, split)
}

/// The goal view's generated impls (#1077): the backend ones forward the snapshot-backed
/// reads to the engine context's `r_*` methods, refuse their typed refusal for entity data the
/// tick's view does not carry, and forward every other backend read — the level's own: blocks,
/// loaded chunks, time, height, weather, block entities — to the real backend of the game host
/// the view carries; the host ones answer the entity lookups by the tick's own rules, refuse the
/// mutable methods, and forward everything else to the game host the view carries. The engine
/// includes them beside its `GoalCx`.
fn goal_view(
    areas: &[Area],
    reads: &[(&Area, Tier, &Entry)],
    host_methods: &[(&Area, &Entry)],
    split: bool,
) -> Result<String, String> {
    let mut impls = Impls::default();
    let mut code = String::from(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse mod_api::{prelude::*, *};\n/// The mob tick's read view (#1077), as the game generates it: the snapshot-backed\n/// reads forward to the engine context's own methods, entity data the view does\n/// not carry answers its typed refusal, and every other backend read forwards\n/// to the real backend of the game host the view carries.\nimpl Backend for super::GoalCx<'_, '_> {}\n",
    );
    for (area, tier, entry) in reads {
        let (name, args) = (
            entry.name.as_str(),
            signature_args(&entry.sig, &entry.name)?,
        );
        let refusal = refusal(entry)?;
        let mut sig = entry.sig.clone();
        sig.ident = syn::parse_str(name).map_err(|e| format!("{name}: {e}"))?;
        // Entity data answers the tick's view; residency and non-entity reads forward to the real
        // backend the view carries (a lent mob is `EntityResidency::Busy`, not `Gone`).
        let entity_data = name != "entity_residency"
            && matches!(
                entry.sig.inputs.iter().skip(1).find_map(|arg| match arg {
                    FnArg::Typed(arg) => Some(&*arg.ty),
                    FnArg::Receiver(_) => None,
                }),
                Some(syn::Type::Path(ty))
                    if ty.path.segments.len() == 1
                        && GOAL_VIEW_ENTITY_ARGS
                            .contains(&ty.path.segments[0].ident.to_string().as_str())
            );
        // How a forwarded read answers a host without a backend: its typed refusal.
        let forwarded = match refusal {
            "None" => ".flatten()".to_string(),
            "" => ".unwrap_or_default()".to_string(),
            // A `Result` read's no-backend answer is its error type's own `NoBackend` value,
            // wrapped back in the `Err`.
            refusal if refusal.starts_with("Err(") => {
                ".unwrap_or_else(|| Err(mod_api::NoBackend::no_backend()))".to_string()
            }
            refusal => format!(".unwrap_or({refusal})"),
        };
        let body = if entry.goal_view() || GOAL_VIEW_READS.contains(&name) {
            format!("self.r_{name}({args})")
        } else if entity_data {
            if args.is_empty() {
                refusal.to_string()
            } else if args.contains(',') {
                format!("let _ = ({args}); {refusal}")
            } else {
                format!("let _ = {args}; {refusal}")
            }
        } else {
            // The read names data the tick leaves in place, so the real backend answers it:
            // `backend_read` holds the `None` a host without one answers.
            format!(
                "mod_api::__private::backend_read(self.host, |backend| backend.{name}({args})){forwarded}"
            )
        };
        // Entities that remain in Game can use its ordinary backend. For a
        // multi-entity query all handle arguments must be present there: the
        // Game backend cannot see a mob lent to the current AI tick.
        let game_entities: Vec<_> = entry
            .sig
            .inputs
            .iter()
            .skip(1)
            .filter_map(|arg| {
                let FnArg::Typed(arg) = arg else { return None };
                let Pat::Ident(pat) = &*arg.pat else {
                    return None;
                };
                let Type::Path(ty) = &*arg.ty else {
                    return None;
                };
                let ty = ty.path.get_ident()?.to_string();
                let name = pat.ident.to_string();
                match ty.as_str() {
                    "EntityRef" => Some(name),
                    "LivingRef" | "PlayerRef" | "MobRef" | "ItemEntityRef" | "ExperienceOrbRef"
                    | "MerchantRef" => Some(format!("{name}.entity()")),
                    _ => None,
                }
            })
            .collect();
        let body = if entity_data && !game_entities.is_empty() && !entry.goal_view() {
            let all_available = game_entities
                .iter()
                .map(|entity| format!("self.is_game_entity({entity})"))
                .collect::<Vec<_>>()
                .join(" && ");
            format!(
                "if {all_available} {{ return mod_api::__private::backend_read(self.host, |backend| backend.{name}({args})){forwarded}; }} {body}"
            )
        } else {
            body
        };
        impls.add(
            area.trait_name(*tier),
            format!("#[inline]\n{} {{ {body} }}\n", sig.to_token_stream()),
        );
    }
    code.push_str(&impls.take());
    code.push_str(
        "/// The same view as a host: the entity lookups answer the tick's own rules\n/// (the ticking mob, the player it sees, the snapshot, else None), the mutable\n/// methods stay out of reach of the shared host the view holds, and everything\n/// else - the level, its blocks, tags, recipes and settings - forwards to the\n/// game host the view carries.\nimpl Host for super::GoalCx<'_, '_> {\n#[doc(hidden)]\n#[inline]\nfn backend(&self) -> Option<BackendRef<'_>> { Some(mod_api::__private::backend_ref(self)) }\n}\n",
    );
    // An area whose host methods all write has nothing to forward, but the view still implements it.
    for area in areas
        .iter()
        .filter(|area| area.tiers.contains_key(&Tier::Host))
    {
        impls.add(area.trait_name(Tier::Host), String::new());
    }
    for (area, entry) in host_methods {
        let name = entry.name.as_str();
        let mut sig = entry.sig.clone();
        sig.ident = syn::parse_str(name).map_err(|e| format!("{name}: {e}"))?;
        // A `&mut self` Host method cannot be called on the `&dyn Host` the context hands out,
        // so the view leaves it to the trait's panicking default: a goal cannot reach it (that
        // is the point).
        if matches!(sig.inputs.first(), Some(FnArg::Receiver(r)) if r.mutability.is_some()) {
            continue;
        }
        let args = signature_args(&sig, name)?;
        let body = if GOAL_VIEW_LOOKUPS.contains(&name) {
            format!("self.lookup_{name}({args})")
        } else {
            format!("self.host.{name}({args})")
        };
        impls.add(
            area.trait_name(Tier::Host),
            format!("#[inline]\n{} {{ {body} }}\n", sig.to_token_stream()),
        );
    }
    code.push_str(&impls.take());
    let standard = if split {
        standard_imports(&syn::parse_file(&code).map_err(|e| e.to_string())?)
    } else {
        "use std::path::PathBuf;\nuse std::sync::Arc;\n".into()
    };
    let index = code
        .find("/// The mob")
        .expect("generated goal documentation");
    code.insert_str(index, &standard);
    Ok(code)
}

/// The methods of each trait the goal view implements, in the order they were added.
#[derive(Default)]
struct Impls(Vec<(String, String)>);

impl Impls {
    fn add(&mut self, name: String, method: String) {
        match self.0.iter_mut().find(|(other, _)| *other == name) {
            Some((_, methods)) => methods.push_str(&method),
            None => self.0.push((name, method)),
        }
    }
    fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
            .into_iter()
            .map(|(name, methods)| {
                format!("impl {name} for super::GoalCx<'_, '_> {{\n{methods}}}\n")
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "host/state_write_tests.rs"]
mod state_write_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backend_can_forward_to_a_distinct_game_method() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let areas = load(&root).unwrap();
        let routing = Routing::load(&root).unwrap();
        for package in [GameplayPackage::Server, GameplayPackage::Client] {
            let callees = host_callees(&areas, package, &routing).unwrap();
            assert!(callees.iter().any(|callee| {
                callee.label == "LevelBlocksBackend::level_can_survive"
                    && callee.method == "native_level_can_survive"
                    && callee.kind == RouteKind::SharedTrait
                    && callee
                        .owner
                        .ends_with("::native_survive::NativeSurviveGameExt<'_>")
            }));
            let generated = crate::gameplay::host_source(&root, package).unwrap();
            let generated: String = generated.chars().filter(|ch| !ch.is_whitespace()).collect();
            assert!(generated.contains("::native_level_can_survive(self,level,pos,state)"));
        }
    }

    fn entry(source: &str) -> Entry {
        let method: syn::TraitItemFn = syn::parse_str(source).unwrap();
        Entry {
            name: method.sig.ident.to_string(),
            sig: method.sig,
            attrs: method.attrs,
        }
    }

    fn handles(source: &str) -> BTreeMap<String, String> {
        api_types(source).unwrap().handles
    }

    #[test]
    fn immutable_result_arguments_refuse_conversion_with_a_typed_error() {
        let method = entry(
            "fn read(&self, ingredient: &Ingredient) -> Result<Arc<[ItemKey]>, IngredientKeysError>;",
        );
        let (setup, _, checked_write) =
            boundary_arguments(&method.sig, "read", false, false).unwrap();
        assert!(!checked_write);
        assert!(setup.contains("return Err(mod_api::NoBackend::no_backend())"));
        assert!(!setup.contains("Default::default()"));
    }

    #[test]
    fn native_read_callbacks_forward_without_boundary_conversion() {
        let read = entry(
            "fn visit(&self, at: BlockSide, visitor: &mut dyn FnMut(&ItemResource, u64) -> std::ops::ControlFlow<()>) -> Result<(), StorageReadError>;",
        );
        let (setup, args, checked_write) =
            boundary_arguments(&read.sig, "visit", false, false).unwrap();
        assert!(setup.is_empty());
        assert_eq!(args, "at, visitor");
        assert!(!checked_write);
    }

    #[test]
    fn borrowed_stacks_are_read_without_write_validation_outside_checked_writes() {
        let read = entry("fn item_resource_damageable(&self, stack: &ItemStack) -> bool;");
        let (setup, args, checked_write) =
            boundary_arguments(&read.sig, "read", false, false).unwrap();
        assert!(!checked_write);
        assert!(setup.contains("crate::game::api_values::stack_value(stack)"));
        assert!(!setup.contains("try_into_engine"));
        assert_eq!(args, "&stack");
        let write = entry(
            "fn living_apply_consume_effect(&mut self, stack: &ItemStack) -> Result<bool, StackWriteError>;",
        );
        let (setup, _, checked_write) =
            boundary_arguments(&write.sig, "write", false, false).unwrap();
        assert!(checked_write);
        assert!(setup.contains("stack.try_into_engine(Self::items(self))?"));
        let owned = entry("fn drop_at(&mut self, stack: ItemStack) -> bool;");
        let (setup, _, _) = boundary_arguments(&owned.sig, "owned", false, false).unwrap();
        assert!(setup.contains("stack.try_into_engine(Self::items(self))"));
    }

    #[test]
    fn a_method_is_a_handles_when_it_takes_one_first_and_is_named_for_it() {
        let handles = handles("pub struct EntityRef; pub struct LevelRef; pub struct BlockKey;");
        let read = entry(
            "/// Where it is.\nfn entity_position(&self, entity: EntityRef, scale: f64) -> Option<[f64; 3]>;",
        );
        let (handle, code) = handle_method(&read, Tier::Backend, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "EntityRef");
        assert!(code.starts_with("/// Where it is.\n"));
        assert!(code.contains("pub fn position (self , host : & dyn Host , scale : f64)"));
        assert!(code.contains("backend_of(host)?.entity_position(self, scale)"));
        // A client read borrows the client host, a write the mutable one of its side.
        let (_, client) = handle_method(&read, Tier::ClientBackend, &handles)
            .unwrap()
            .unwrap();
        assert!(client.contains("host : & dyn ClientHost"));
        let write = entry("fn level_set(&mut self, level: LevelRef) -> bool;");
        let (_, code) = handle_method(&write, Tier::BackendMut, &handles)
            .unwrap()
            .unwrap();
        assert!(code.contains("host : & mut dyn ServerHost"));
        assert!(code.contains(
            "backend_mut_of(host).map(|backend| backend.level_set(self)).unwrap_or(false)"
        ));
        let (_, code) = handle_method(&write, Tier::ClientBackendMut, &handles)
            .unwrap()
            .unwrap();
        assert!(code.contains("host : & mut dyn ClientHost"));
        assert!(code.contains("client_backend_mut_of(host)"));
        // A command result refuses with an unsuccessful run and no feedback.
        let command = entry(
            "fn entity_run_command_with_output(&mut self, entity: EntityRef, line: &str, level: PermissionLevel, suppress_output: bool) -> CommandResult;",
        );
        let (handle, code) = handle_method(&command, Tier::BackendMut, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "EntityRef");
        assert!(code.contains(
            "backend_mut_of(host).map(|backend| backend.entity_run_command_with_output(self, line, level, suppress_output)).unwrap_or_default()"
        ));
        // A residency read defaults to `EntityResidency::Gone` without a backend.
        let residency = entry("fn entity_residency(&self, entity: EntityRef) -> EntityResidency;");
        let (handle, code) = handle_method(&residency, Tier::Backend, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "EntityRef");
        assert!(code.contains("pub fn residency (self , host : & dyn Host) -> EntityResidency"));
        assert!(code.contains(
            "backend_of(host).map(|backend| backend.entity_residency(self)).unwrap_or_default()"
        ));
        // A `Result` method refuses with its error's `NoBackend` value on a host without a game.
        let tried = entry("fn level_attempt(&mut self, level: LevelRef) -> Result<u8, Refused>;");
        let (_, code) = handle_method(&tried, Tier::BackendMut, &handles)
            .unwrap()
            .unwrap();
        assert!(code.contains(
            "backend_mut_of(host).map_or_else(|| Err(crate::NoBackend::no_backend()), |backend| backend.level_attempt(self))"
        ));
        // The registry name borrows the host and is absent without a backend.
        let key = entry("fn block_key_id(&self, key: BlockKey) -> Option<&str>;");
        let (handle, code) = handle_method(&key, Tier::Backend, &handles)
            .unwrap()
            .unwrap();
        assert_eq!(handle, "BlockKey");
        assert!(code.contains("pub fn id (self , host : & dyn Host) -> Option < & str >"));
        assert!(code.contains("backend_of(host)?.block_key_id(self)"));
    }

    #[test]
    fn registry_key_reads_cannot_generate_an_infallible_string() {
        let method = entry("fn block_key_id(&self, key: BlockKey) -> String;");
        let handles = handles("pub struct BlockKey;");
        let error = handle_method(&method, Tier::Backend, &handles).unwrap_err();
        assert!(error.starts_with("block_key_id:"));

        let area = Area {
            stem: "keys".into(),
            camel: "Keys".into(),
            tiers: BTreeMap::new(),
            tables: BTreeMap::new(),
            server_only_tiers: std::collections::BTreeSet::new(),
        };
        assert_eq!(
            goal_view(&[], &[(&area, Tier::Backend, &method)], &[], false),
            Err("block_key_id: cannot refuse its backend return type".into())
        );
    }

    #[test]
    fn other_methods_are_the_backends_alone_and_a_wrong_shape_is_refused() {
        let handles = handles("pub struct EntityRef; pub struct LevelRef;");
        // Not named for its handle, or not taking one first: a backend method only.
        for source in [
            "fn mob_x(&self, entity: EntityRef) -> Option<()>;",
            "fn entity_x(&self, level: LevelRef) -> Option<()>;",
            "fn entity_x(&self, count: u32) -> Option<()>;",
            "fn entity_x(&self) -> Option<()>;",
        ] {
            assert!(
                handle_method(&entry(source), Tier::Backend, &handles)
                    .unwrap()
                    .is_none(),
                "{source}"
            );
        }
        for (source, tier, error) in [
            (
                "fn entity_type(&self, entity: EntityRef) -> Option<()>;",
                Tier::Backend,
                "not a method name",
            ),
            (
                "fn entity_x(&self, entity: EntityRef);",
                Tier::Backend,
                "returns Option, bool, CommandResult, EntityResidency or Result",
            ),
            (
                "fn entity_x(&mut self, entity: EntityRef) -> bool;",
                Tier::Backend,
                "disagree",
            ),
        ] {
            assert!(
                handle_method(&entry(source), tier, &handles)
                    .unwrap_err()
                    .contains(error),
                "{source}"
            );
        }
    }

    #[test]
    fn public_ref_structs_define_receivers_without_registration() {
        let types = api_types(
            "pub struct BlockEntityRef; pub struct HTTPRequestRef;
             struct HiddenRef; pub(crate) struct InternalRef;
             pub enum EnumRef {} pub type AliasRef = BlockEntityRef; pub struct Plain;",
        )
        .unwrap();
        for (receiver, ty) in [
            ("block_entity", "BlockEntityRef"),
            ("http_request", "HTTPRequestRef"),
        ] {
            let name = format!("{receiver}_kind");
            let method = entry(&format!("fn {name}(&self, value: {ty}) -> Option<()>;"));
            let (handle, code) = handle_method(&method, Tier::Backend, &types.handles)
                .unwrap()
                .unwrap();
            assert_eq!(handle, ty);
            assert!(code.contains(&format!("backend_of(host)?.{name}(self)")));
        }
        let hidden = entry("fn hidden_kind(&self, value: HiddenRef) -> Option<()>;");
        assert!(
            handle_method(&hidden, Tier::Backend, &types.handles)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            api_types("pub struct URLRef; pub struct UrlRef;")
                .err()
                .unwrap(),
            "duplicate handle receiver url",
        );
    }

    #[test]
    fn text_arguments_refuse_before_even_checked_stack_writes() {
        let text = entry("fn set_text(&mut self, text: &Text) -> Result<(), TextFormError>;");
        let (setup, args, checked) =
            boundary_arguments(&text.sig, "set_text", false, false).unwrap();
        assert!(!checked);
        assert!(setup.contains("text.try_into_engine(&())?"));
        assert_eq!(args, "&text");
        let mixed = entry(
            "fn update(&mut self, stack: ItemStack, text: Option<&Text>) -> Result<Text, Refused>;",
        );
        let (setup, args, checked) =
            boundary_arguments(&mixed.sig, "update", false, false).unwrap();
        assert!(checked);
        assert!(setup.contains("stack.try_into_engine(Self::items(self))?"));
        assert!(setup.contains("text.try_into_engine(&())?"));
        assert_eq!(args, "stack, text.as_ref()");
        let syn::ReturnType::Type(_, output) = &mixed.sig.output else {
            panic!()
        };
        assert!(text_value(output));
        assert!(!boundary_value(output));
    }

    #[test]
    #[ignore = "daily"]
    fn forwarder_routes_compile_without_recursing_on_either_side() {
        let area = Area {
            stem: "fixture".into(),
            camel: "Fixture".into(),
            tiers: BTreeMap::from([
                (
                    Tier::Host,
                    vec![
                        entry("fn local(&self) -> u32;"),
                        entry("#[static_call] fn fixed(&self) -> u32;"),
                        entry("#[published(shared)] fn published(&self) -> u32;"),
                        entry("#[service] fn service(&self) -> &dyn Service;"),
                    ],
                ),
                (Tier::Backend, vec![entry("fn table(&self) -> u32;")]),
                (
                    Tier::BackendMut,
                    vec![entry("fn update(&mut self, value: u32);")],
                ),
                (
                    Tier::ClientBackend,
                    vec![entry("fn desktop(&self) -> Option<u32>;")],
                ),
            ]),
            tables: BTreeMap::from([(Tier::Backend, "crate::game::Tables".into())]),
            server_only_tiers: Default::default(),
        };
        let forwards = render_forwarders(&forwarder_plan(&[area]).unwrap(), None).unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let input = fixture.path().join("forwarders.rs");
        fs::write(
            &input,
            format!(
                r#"
extern crate self as mod_api;
pub trait Service {{}}
pub mod __private {{
    pub fn shared() -> u32 {{ 3 }}
    pub mod game {{
        pub struct CallbackOwner;
        impl CallbackOwner {{
            pub fn call_host<T>(call: impl FnOnce() -> T) -> T {{ call() }}
        }}
    }}
}}
pub mod prelude {{
    pub trait FixtureHost {{
        fn local(&self) -> u32;
        fn fixed(&self) -> u32;
        fn published(&self) -> u32;
        fn service(&self) -> &dyn crate::Service;
    }}
    pub trait FixtureBackend {{ fn table(&self) -> u32; }}
    pub trait FixtureBackendMut {{ fn update(&mut self, value: u32); }}
    pub trait FixtureClientBackend {{ fn desktop(&self) -> Option<u32>; }}
}}
mod game {{
    pub(crate) mod api_values {{
        pub fn call_stack_write() {{}}
        pub fn call_state_write() {{}}
        pub fn call_write() {{}}
        pub trait TryIntoApi {{}}
        pub trait TryIntoEngine {{}}
    }}
    pub struct ServerGame<'a>(pub &'a mut u32);
    pub struct ClientGame<'a>(pub &'a mut u32);
    pub struct Tables<'a>(&'a u32);
    impl crate::Service for ServerGame<'_> {{}}
    impl crate::Service for ClientGame<'_> {{}}
    impl ServerGame<'_> {{
        fn local(&self) -> u32 {{ *self.0 }}
        fn fixed() -> u32 {{ 2 }}
        fn update(&mut self, value: u32) {{ *self.0 = value; }}
    }}
    impl ClientGame<'_> {{
        fn local(&self) -> u32 {{ *self.0 }}
        fn fixed() -> u32 {{ 2 }}
        fn desktop(&self) -> Option<u32> {{ Some(*self.0) }}
    }}
    trait State {{ fn state(&self) -> &u32; }}
    impl State for ServerGame<'_> {{ fn state(&self) -> &u32 {{ self.0 }} }}
    impl State for ClientGame<'_> {{ fn state(&self) -> &u32 {{ self.0 }} }}
    impl<'a> Tables<'a> {{
        fn of(game: &'a impl State) -> Self {{ Self(game.state()) }}
        fn table(&self) -> u32 {{ *self.0 + 1 }}
    }}
    mod generated {{ {forwards} }}
}}
fn main() {{
    use prelude::*;
    fn check(game: &(impl FixtureHost + FixtureBackend), value: u32) {{
        assert_eq!(game.local(), value);
        assert_eq!(game.table(), value + 1);
        assert_eq!(game.fixed(), 2);
        assert_eq!(game.published(), 3);
        let _ = game.service();
    }}
    let mut value = 4;
    let mut server = game::ServerGame(&mut value);
    assert_eq!(server.desktop(), None);
    check(&server, 4);
    server.update(7);
    let client = game::ClientGame(&mut value);
    check(&client, 7);
    assert_eq!(FixtureClientBackend::desktop(&client), Some(7));
}}
"#
            ),
        )
        .unwrap();
        let binary = fixture
            .path()
            .join(format!("forwarders{}", std::env::consts::EXE_SUFFIX));
        let compile = Command::new("rustc")
            .args(["--edition=2024", "--cfg", "feature=\"client\""])
            .arg(&input)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        assert!(Command::new(binary).status().unwrap().success());
    }

    #[test]
    fn game_provides_every_area_method_explicitly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        use std::collections::BTreeSet;
        let expected: BTreeSet<_> = load(&root)
            .unwrap()
            .into_iter()
            .flat_map(|area| {
                area.tiers.into_iter().flat_map(move |(tier, entries)| {
                    let owner = format!("{}{}", area.camel, tier.suffix());
                    let sides: &[&str] = match tier {
                        Tier::ServerHost | Tier::BackendMut => &["ServerGame"],
                        Tier::ClientHost | Tier::ClientBackendMut => &["ClientGame"],
                        Tier::Host | Tier::Backend | Tier::ClientBackend => {
                            &["ServerGame", "ClientGame"]
                        }
                    };
                    entries.into_iter().flat_map(move |entry| {
                        let owner = owner.clone();
                        let name = entry.name;
                        sides
                            .iter()
                            .map(move |&side| (side.to_owned(), owner.clone(), name.clone()))
                    })
                })
            })
            .collect();
        let source = game_forwarders(&root).unwrap();
        let file = syn::parse_file(&source).unwrap();
        let mut actual = BTreeSet::new();
        for item in file.items {
            if let Item::Impl(item) = item {
                assert!(item.attrs.iter().any(|attr| {
                    attr.path().is_ident("deny")
                        && attr
                            .parse_args::<syn::Ident>()
                            .is_ok_and(|lint| lint == "unconditional_recursion")
                }));
                // The generator also emits table-backed view implementations.
                // This assertion covers ServerGame and ClientGame's complete backend surface.
                let syn::Type::Path(receiver) = &*item.self_ty else {
                    panic!("expected named host")
                };
                let side = receiver.path.segments.last().unwrap().ident.to_string();
                if side != "ServerGame" && side != "ClientGame" {
                    continue;
                }
                let owner = item
                    .trait_
                    .unwrap()
                    .1
                    .segments
                    .last()
                    .unwrap()
                    .ident
                    .to_string();
                for member in item.items {
                    let syn::ImplItem::Fn(method) = member else {
                        panic!("non-method in Game impl")
                    };
                    let name = method.sig.ident.to_string();
                    assert!(actual.insert((side.clone(), owner.clone(), name)));
                }
            }
        }
        assert_eq!(actual, expected, "Game must not inherit an area default");
        // Other area traits must not be in scope. The current impl's own trait is
        // implicitly in scope; deny(unconditional_recursion) rejects that fallback.
        assert!(!source.contains("prelude::*"));
    }

    #[test]
    fn an_area_file_reads_its_tier_traits() {
        let file = syn::parse_str(
            "pub trait ItemsHost { fn a(&self); } pub trait ItemsBackendMut { fn b(&mut self); }",
        )
        .unwrap();
        let tiers = area_tiers("Items", file).unwrap();
        assert_eq!(
            tiers.keys().copied().collect::<Vec<_>>(),
            [Tier::Host, Tier::BackendMut]
        );
        assert_eq!(tiers[&Tier::Host][0].name, "a");
    }

    #[test]
    fn an_unrecognised_area_trait_is_refused() {
        for wrong in [
            "ItemHost",
            "ItemsReadBackend",
            "OtherServerHost",
            "ReviewProbeHosts",
            "Unrelated",
        ] {
            for recognised in ["", "pub trait ItemsHost { fn known(&self); }"] {
                let file = syn::parse_str(&format!(
                    "{recognised} pub trait {wrong} {{ fn a(&self) -> &std::path::Path; }}"
                ))
                .unwrap();
                let error = area_tiers("Items", file).err().unwrap();
                assert!(error.contains(wrong), "{error}");
            }
        }
    }

    #[test]
    fn declarations_cannot_hide_in_modules_or_macros() {
        for (kind, source) in [
            (
                "module",
                "pub mod inner { pub trait ItemsHost { fn unsupported(&self) -> &std::path::Path; } }",
            ),
            (
                "macro",
                "macro_rules! hidden { () => { pub trait ItemsHost { fn unsupported(&self) -> &std::path::Path; } }; } hidden!();",
            ),
            (
                "macro",
                "hidden! { pub trait ItemsHost { fn unsupported(&self) -> &std::path::Path; } }",
            ),
        ] {
            let file = syn::parse_file(source).unwrap();
            let error = area_tiers("Items", file).err().unwrap();
            assert!(error.contains(&format!("unsupported {kind}")), "{error}");
            assert!(error.contains("Items"), "{error}");
        }
    }

    #[test]
    fn imports_and_attributes_are_allowed() {
        let file = syn::parse_file(
            "#![allow(unused_variables)] use crate::*; /// An area's methods.\n #[doc(hidden)] pub trait ItemsBackend {}",
        )
        .unwrap();
        let tiers = area_tiers("Items", file).unwrap();
        assert!(tiers.contains_key(&Tier::Backend));
    }

    #[test]
    fn non_host_traits_are_allowed_only_in_their_own_file() {
        for (area, name) in [("Draw", "DrawContext"), ("Services", "Commands")] {
            let file = syn::parse_file(&format!("pub trait {name} {{}}")).unwrap();
            assert!(area_tiers(area, file.clone()).unwrap().is_empty());
            assert!(area_tiers("Items", file).is_err());
        }
    }

    #[test]
    fn goal_view_forwards_entity_residency_to_the_host_backend() {
        let method = entry("fn entity_residency(&self, entity: EntityRef) -> EntityResidency;");
        let area = Area {
            stem: "entities".into(),
            camel: "Entities".into(),
            tiers: BTreeMap::new(),
            tables: BTreeMap::new(),
            server_only_tiers: BTreeSet::new(),
        };
        let code = goal_view(&[], &[(&area, Tier::Backend, &method)], &[], true).unwrap();
        assert!(code.contains(
            "mod_api::__private::backend_read(self.host, |backend| backend.entity_residency(entity)).unwrap_or(EntityResidency::Gone)"
        ));
        assert!(!code.contains("is_game_entity"));
    }

    #[test]
    fn every_area_file_of_the_checkout_loads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let areas = load(&root).unwrap();
        assert!(areas.iter().any(|area| area.stem == "config"));
        assert!(
            areas
                .iter()
                .filter(|area| area.tiers.is_empty())
                .all(|area| matches!(area.stem.as_str(), "draw" | "services"))
        );
    }

    #[test]
    fn only_synchronous_borrowed_callbacks_stay_in_caller_scope() {
        assert!(!borrows_caller_callback(
            &entry("fn living_heal(&mut self, living: LivingRef, amount: f32) -> bool;").sig
        ));
        assert!(!borrows_caller_callback(
            &entry("fn schedule_task(&mut self, delay: u32, task: Box<dyn FnOnce(&mut dyn ServerHost)>) -> Result<TaskId, ScheduleError>;").sig
        ));
        assert!(borrows_caller_callback(
            &entry("fn level_visit_all_entities(&self, level: LevelRef, visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>) -> Result<core::ops::ControlFlow<()>, LevelReadError>;").sig
        ));
        assert!(borrows_caller_callback(
            &entry("fn player_read_inventory(&self, player: PlayerRef, read: &mut dyn for<'a> FnMut(InventoryView<'a>)) -> Option<()>;").sig
        ));
    }
}
