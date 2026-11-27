//! Mods in the game (`docs/modding/src/how-to/content-mods.md`): content mods' blocks, items, data and
//! assets (`content_root`), and the Mods page's rows. Code mods are native (`native_hooks.rs`)
//! with content-only packages loaded separately.
#[allow(unused_imports)]
pub use super::{execution_role::ExecutionRole, registration_capability_coverage as coverage};
#[cfg(test)]
mod content_admission_tests;
pub(in crate::game) mod menu_books;
pub mod package_files;
pub mod runtime_files;
pub mod standalone;
#[allow(unused_imports)]
pub use mod_api::__private::{DeclarationKind, DeclarationRefusal, DeclarationSink, game::*};
#[allow(unused_imports)]
pub use mod_api::{Answered, DeclarationError, LoadingMessage, RegistrationContext};
#[allow(unused_imports)]
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[allow(unused_imports)]
pub use sunrise_game_data::lang;
#[allow(unused_imports)]
pub use sunrise_mod_package::manifest::{MissingJar, collect_jar_requirements, missing_jars};
#[allow(unused_imports)]
pub use sunrise_mod_package::mod_list::{self, Installed};
#[allow(unused_imports)]
use sunrise_sim::{mobs::spawner::SpawnerTypeFacts, mod_content::DeclaredContent};
#[allow(unused_imports)]
use sunrise_world::{
    item_defaults::ItemData,
    native_load_context::{LoadContext, LoadInputs},
};

pub(in crate::game) struct ContentRequest<'a> {
    pub(in crate::game) path: &'a Path,
    pub(in crate::game) manifest: &'a sunrise_mod_package::Manifest,
    pub(in crate::game) admitted: Option<&'a package_files::PackageContent>,
}

/// Open package sources and read their declarations in parallel, keeping results in request order.
pub(in crate::game) fn read_package_inputs<T: Sync, U: Send>(
    requests: &[T],
    read: impl Fn(&T) -> U + Sync,
) -> Vec<U> {
    if requests.is_empty() {
        return Vec::new();
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let threads = sunrise_threads::worker_count().clamp(1, requests.len());
    let mut done: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(request) = requests.get(index) else {
                            return done;
                        };
                        done.push((index, read(request)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

pub(in crate::game) fn read_contents(
    requests: &[ContentRequest<'_>],
) -> Vec<Result<sunrise_mod_package::Content, String>> {
    read_package_inputs(
        requests,
        |ContentRequest {
             path,
             manifest,
             admitted,
         }| {
            let content = match admitted {
                Some(source) => source.read(manifest),
                None => sunrise_mod_package::Content::read(path, manifest),
            };
            if let Ok(content) = &content {
                remember_verdict(path, content);
            }
            content
        },
    )
}

#[derive(Default)]
pub struct CodeSwitches {
    pub(in crate::game) disabled: std::collections::BTreeSet<String>,
    pub(in crate::game) applied: std::sync::atomic::AtomicBool,
}

pub(in crate::game) type ContentMaterializer =
    fn(
        &Path,
        PathBuf,
        &Path,
        &[sunrise_sim::mod_content::ContentMod],
        PathBuf,
    ) -> sunrise_game_data::Result<sunrise_assets::resource_source::ResourceRoot>;

#[derive(Clone, Default)]
pub struct ModsState {
    /// The instance's native mod settings, shared by the window and its integrated server.
    pub settings: crate::game::native_hooks::mod_settings::SharedSettings,
    pub active_condition_root: Option<Arc<()>>,
    pub load_condition_candidate: Option<sunrise_world::conditions::LoadConditions>,
    pub load_conditions:
        Arc<std::sync::OnceLock<Result<crate::game::load_conditions::ConditionTable, String>>>,
    pub profession_declarations:
        Arc<std::sync::OnceLock<Vec<(String, mod_api::ProfessionDefinition)>>>,
    pub loot_modifier_serializers:
        Arc<std::sync::OnceLock<super::global_loot_modifiers::Serializers>>,
    /// Saved switches gate the first data inventory while registration keeps
    /// its existing startup ordering. Runtime dispatch owns later switches.
    pub code_switches: Option<Arc<CodeSwitches>>,
    /// The original prepared input is fixed for this engine load, shared by
    /// the window and its server. A later data reload must not overlay an
    /// already generated root recursively.
    pub content_base: Arc<std::sync::OnceLock<PathBuf>>,
    pub content_materializer: Option<ContentMaterializer>,
    /// `--dev-mod PATH`: content mods always on.
    pub dev: Vec<PathBuf>,
    pub native: std::sync::Arc<sunrise_mod_package::runtime::Runtime>,
    pub installation_side: sunrise_mod_package::manifest::Side,
    pub overrides_dir: PathBuf,
    pub load_notices: Arc<Mutex<Vec<sunrise_mod_package::dependencies::Notice>>>,
    pub selected: std::sync::Arc<[sunrise_mod_package::selection::SelectedPackage]>,
    /// The registration inventory and the file indexes it shares with startup content loading.
    pub startup_admission: Option<Arc<package_files::StartupAdmission>>,
    /// The content mods the loaded resources were built with: a switch the
    /// game can apply live moves it to the player's choice (`switch_mods`).
    pub applied: Option<std::collections::BTreeSet<String>>,
    /// A live switch's resource reload is running: when it applied, the server
    /// reloads its data from the new root. Its start, for the log.
    pub live: Option<std::time::Instant>,
    /// The content mods installed, and those of them with only assets and
    /// data (`mod_content::live_switch`): they switch with a world open.
    pub content_ids: std::collections::BTreeSet<String>,
    pub live_ids: std::collections::BTreeSet<String>,
    /// The server's data comes from a root a live switch built, not the one
    /// `res` was loaded from: the world tables are built from it even with no
    /// data pack (`stage_world_tables`).
    pub data_switched: bool,
    /// Structured load refusals, rendered only at the existing Mods-page boundary.
    pub refused: Arc<Mutex<Vec<MissingJar>>>,
    /// Served content packages withheld at startup because their `served_by` host was disabled:
    /// `(content_id, host_id)`.
    pub disabled_served: Arc<[(String, String)]>,
    /// Native hosts with active or withheld served packages at startup.
    pub restart_bound_hosts: BTreeSet<String>,
    /// The entries of the mods' datapack registries this world session has published, at the
    /// generation of the load or reload that built them; `None` when no mod declared one.
    pub datapack_registries: Option<Arc<mod_api::DatapackRegistrySet>>,
    /// Decoded vanilla codec values (`IntProvider`, `BlockStateProvider`, `BlockPredicate`,
    /// `HolderSet`) for [`Self::datapack_registries`], swapped atomically on load and `/reload`.
    pub datapack_vanilla_values: Option<Arc<super::native_datapack_values::DatapackVanillaValues>>,
}

impl ModsState {
    pub fn capture_code_switches(&mut self, disabled: &std::collections::BTreeSet<String>) {
        self.code_switches = Some(Arc::new(CodeSwitches {
            disabled: disabled.clone(),
            applied: Default::default(),
        }));
    }

    pub(in crate::game) fn startup_code_disabled(&self, id: &str) -> bool {
        self.code_switches
            .as_ref()
            .is_some_and(|switches| switches.disabled.contains(id))
    }

    /// Compute the effective disabled native set for the live session and any restart-bound
    /// serving hosts whose saved toggle differs from their applied startup state.
    pub fn reconcile_code_switches(
        &self,
        wanted_disabled: &std::collections::BTreeSet<String>,
    ) -> (std::collections::BTreeSet<String>, Vec<String>) {
        let mut effective = wanted_disabled.clone();
        let mut waiting = Vec::new();
        for host in &self.restart_bound_hosts {
            let applied_off = self.startup_code_disabled(host);
            let wanted_off = wanted_disabled.contains(host);
            if applied_off != wanted_off {
                waiting.push(host.clone());
                if applied_off {
                    effective.insert(host.clone());
                } else {
                    effective.remove(host);
                }
            }
        }
        (effective, waiting)
    }

    /// Whether native mod `id` is effectively disabled in this running session (respecting
    /// restart-bound serving hosts).
    pub fn code_disabled(
        &self,
        wanted_disabled: &std::collections::BTreeSet<String>,
        id: &str,
    ) -> bool {
        if self.restart_bound_hosts.contains(id) {
            self.startup_code_disabled(id)
        } else {
            wanted_disabled.contains(id)
        }
    }

    pub fn apply_code_switches(&self, disabled: &std::collections::BTreeSet<String>) {
        let (effective, _) = self.reconcile_code_switches(disabled);
        for id in mod_api::__private::installed_code_ids() {
            mod_api::__private::set_mod_enabled(id, !effective.contains(id));
        }
        if let Some(switches) = &self.code_switches {
            switches
                .applied
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn code_owner_enabled(&self, owner: &str) -> bool {
        let initially_disabled = self.code_switches.as_ref().is_some_and(|switches| {
            !switches.applied.load(std::sync::atomic::Ordering::Acquire)
                && switches.disabled.contains(owner)
        });
        !initially_disabled
            && mod_api::__private::mod_index(owner).is_some_and(mod_api::__private::mod_index_on)
    }

    /// One admission rule for initial composition and later code-only toggles.
    /// Content acceptance belongs to the generation; code admission belongs to
    /// the installed dispatch entries, including their live enabled switches.
    pub fn enabled_inventory(
        &self,
        accepted_content: impl Fn(&str) -> bool,
    ) -> sunrise_mod_package::enabled::EnabledMods {
        let native = self
            .native
            .packages()
            .iter()
            .map(|package| &package.manifest)
            .filter(|manifest| self.code_owner_enabled(&manifest.id));
        let content = self
            .selected
            .iter()
            .map(|package| &package.package.manifest)
            .filter(|manifest| accepted_content(&manifest.id));
        sunrise_mod_package::enabled::EnabledMods::new(
            native
                .chain(content)
                .map(|manifest| (manifest.id.clone(), manifest.side)),
        )
    }

    pub fn select(
        &mut self,
        game_dir: &std::path::Path,
        enabled: &std::collections::BTreeSet<String>,
    ) {
        let mut disabled_served = Vec::new();
        self.selected =
            sunrise_mod_package::selection::content_packages(game_dir, enabled, &self.dev)
                .filter(|selected| {
                    let manifest = &selected.package.manifest;
                    if self
                        .native
                        .built()
                        .iter()
                        .any(|built| built.id == manifest.id)
                    {
                        return false;
                    }
                    if let Some(host) = manifest.served_by.as_deref()
                        && self.startup_code_disabled(host)
                    {
                        log::warn!(
                            "Sunrise: served mod {} not loaded because its serving host {} is \
                             disabled; enable {} and restart",
                            manifest.id,
                            host,
                            host
                        );
                        disabled_served.push((manifest.id.clone(), host.to_owned()));
                        return false;
                    }
                    true
                })
                .collect();
        self.disabled_served = disabled_served.into();
        self.restart_bound_hosts = self
            .selected
            .iter()
            .filter_map(|p| p.package.manifest.served_by.clone())
            .chain(self.disabled_served.iter().map(|(_, host)| host.clone()))
            .collect();
        self.refused = Default::default();
    }

    pub fn admit_startup_packages(
        &mut self,
        jar_store: &Path,
    ) -> sunrise_game_data::Result<Arc<package_files::StartupAdmission>> {
        let refused = self.missing_jar_refusals(jar_store)?;
        if let Some(missing) = self.fatal_missing_jar(&refused) {
            return Err(missing.clone().into());
        }
        let admission = Arc::new(package_files::StartupAdmission::open(self, refused)?);
        self.startup_admission = Some(admission.clone());
        Ok(admission)
    }

    pub(in crate::game) fn missing_jar_refusals(
        &self,
        jar_store: &Path,
    ) -> sunrise_game_data::Result<Vec<MissingJar>> {
        let requirements = collect_jar_requirements(
            self.native
                .packages()
                .iter()
                .map(|p| &p.manifest)
                .chain(self.selected.iter().map(|p| &p.package.manifest))
                .map(|m| (m.id.as_str(), m.jar.as_slice())),
        )?;
        Ok(missing_jars(jar_store, &requirements)?)
    }

    pub(in crate::game) fn fatal_missing_jar<'a>(
        &self,
        refused: &'a [MissingJar],
    ) -> Option<&'a MissingJar> {
        refused.iter().find(|missing| {
            missing.owners.iter().any(|id| {
                self.native.built().iter().any(|built| built.id == id)
                    || self.selected.iter().any(|p| {
                        p.package.manifest.id == *id && p.package.manifest.served_by.is_some()
                    })
            })
        })
    }

    pub fn report_refusals(&self, rows: &mut [Installed]) {
        let refused = self.refused.lock().unwrap();
        for row in rows {
            let mut problems: Vec<_> = refused
                .iter()
                .filter(|missing| missing.owners.contains(&row.id))
                .map(ToString::to_string)
                .collect();
            if let Some(detail) = self
                .startup_admission
                .as_deref()
                .and_then(|a| a.namespace_refusal(&row.id))
            {
                problems.push(format!("Can't be read: {detail}"));
            }
            for (content_id, host_id) in self.disabled_served.iter() {
                if *content_id == row.id {
                    problems.push(format!(
                        "Not loaded: serving host {host_id} is disabled; enable {host_id} and restart"
                    ));
                }
            }
            if !problems.is_empty() {
                row.problem = Some(problems.join("\n"));
            }
        }
    }
}

pub mod item_data_load;
#[allow(unused_imports)]
pub use item_data_load::{LoadRole, load_item_data};

/// The built-in registries' names in ID order, with Sunrise's own data components.
pub(in crate::game) fn builtin_registry_names(
    root: &Path,
) -> sunrise_game_data::Result<BTreeMap<String, Vec<String>>> {
    let mut registries: BTreeMap<_, _> = builtin_registries(root)?
        .registries
        .into_iter()
        .map(|(name, ids)| (name, ids.names))
        .collect();
    if let Some(types) = registries.get_mut("minecraft:data_component_type") {
        types.extend(sunrise_protocol::item::custom::entries().map(|(_, name)| name.to_owned()));
    }
    Ok(registries)
}

pub(in crate::game) fn registration_registry_names(
    root: &Path,
) -> sunrise_game_data::Result<BTreeMap<String, Vec<String>>> {
    use sunrise_game_data::potions::{EFFECTS, namespaced};
    let mut registries = builtin_registry_names(root)?;
    // Effects resolve against the engine table even if the prepared report omits an entry.
    registries.insert(
        "minecraft:mob_effect".into(),
        EFFECTS
            .iter()
            .map(|def| namespaced(def.id).to_owned())
            .collect(),
    );
    Ok(registries)
}

/// Capture registration's stable registries, settings and stock capability type owners.
pub fn install_registration_data(
    root: &Path,
    directory: PathBuf,
    runtime_files: BTreeMap<String, Arc<dyn mod_api::RuntimeFileSource>>,
    package_files: BTreeMap<String, Arc<dyn mod_api::PackageFileSource>>,
) -> sunrise_game_data::Result<()> {
    let packages = mod_api::__private::loaded_mods();
    (mod_api::__private::platform::platform()?.install_registration_data)(
        mod_api::__private::RegistrationData::new(
            registration_registry_names(root)?,
            sunrise_world::native_load_context::capture_settings(
                &crate::game::mod_data_root(),
                packages.ids(),
            ),
            mod_api::InstanceDirectory::from_path(&directory)
                .expect("the host path has an encoding the mod API carries"),
            sunrise_level::runtime_env::runtime_environment::get().clone(),
        )
        .with_capability_coverage(coverage::builtin, coverage::block_entity)
        .with_runtime_files(runtime_files)
        .with_package_files(package_files),
    );
    Ok(())
}

pub(in crate::game) fn builtin_registries(
    root: &Path,
) -> sunrise_game_data::Result<sunrise_protocol::registry::Builtin> {
    let path = sunrise_protocol::registry::Builtin::find(root).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "registries.json not found")
    })?;
    Ok(sunrise_protocol::registry::Builtin::load(&path)?)
}

/// Freeze native component IDs before reading content or opening a connection.
/// With no mod registering any, nothing is installed, which every reader
/// treats as no custom components, and `registries.json` isn't read.
pub fn register_components(root: &std::path::Path) -> sunrise_game_data::Result<()> {
    let mut entries = Vec::new();
    crate::game::native_hooks::native_component_types(&RegistrationContext::new(), &mut entries);
    if entries.is_empty() {
        return Ok(());
    }
    let builtin = builtin_registries(root)?;
    let types = builtin
        .get("minecraft:data_component_type")
        .ok_or("missing data component registry")?;
    let mut seen = std::collections::HashMap::<String, &'static str>::with_capacity(entries.len());
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = entry.answer;
        let mod_id = entry.mod_id;
        if sunrise_core::identifier::Identifier::parse_namespaced(&id).is_none() {
            return Err(format!("mod {mod_id}: invalid native component name {id:?}").into());
        }
        if types.names.contains(&id) {
            return Err(format!(
                "mod {mod_id}: native component {id} conflicts with a built-in data component type"
            )
            .into());
        }
        if let Some(first) = seen.insert(id.clone(), mod_id) {
            return Err(format!(
                "mod {mod_id}: duplicate native component {id} (already registered by mod {first})"
            )
            .into());
        }
        names.push(id);
    }
    let first = i32::try_from(types.names.len())?;
    sunrise_protocol::item::custom::install(first, names)
        .map_err(|error| format!("invalid native component registration: {error}"))?;
    Ok(())
}

/// Finish every once-only content declaration on the registration thread.
pub fn freeze_declarations() {
    let _ = native_declared();
}

#[derive(Clone, Debug)]
pub struct DeclarationFailures(pub Vec<DeclarationRefusal>);

impl DeclarationFailures {
    pub fn new(refusals: Vec<DeclarationRefusal>) -> Self {
        Self(refusals)
    }

    pub fn messages(&self) -> impl Iterator<Item = LoadingMessage> {
        use sunrise_game_data::text::Text;
        let quoted = |value: &str| Text::literal(format!("{value:?}"));
        self.0.iter().map(move |refusal| {
            let key = match refusal.kind {
                DeclarationKind::Block => "sunrise.mods.declaration.block",
                DeclarationKind::Item => "sunrise.mods.declaration.item",
            };
            let detail = match &refusal.error {
                DeclarationError::RegistrationClosed { id } => {
                    Text::translate("sunrise.mods.declaration.closed", &[quoted(id)])
                }
                DeclarationError::InvalidId { id } => {
                    Text::translate("sunrise.mods.declaration.invalid_id", &[quoted(id)])
                }
                DeclarationError::Rejected { id, reason } => Text::translate(
                    "sunrise.mods.declaration.rejected",
                    &[quoted(id), Text::literal(reason.clone())],
                ),
                error => Text::literal(error.to_string()),
            };
            let detail = detail.plain().replace(['\r', '\n'], " ");
            LoadingMessage::new(key, &[&refusal.mod_id, &detail])
        })
    }

    pub fn lines(&self) -> Vec<String> {
        use sunrise_game_data::text::Text;
        let render = || {
            self.messages()
                .map(|message| {
                    let args: Vec<_> = message
                        .args()
                        .iter()
                        .map(|arg| Text::literal(arg.clone()))
                        .collect();
                    Text::translate(message.key(), &args).plain()
                })
                .collect()
        };
        if lang::current().is_some() {
            render()
        } else {
            lang::scoped(lang::Language::load(&[], lang::DEFAULT), render).0
        }
    }
}

impl std::fmt::Display for DeclarationFailures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.lines().join("\n"))
    }
}
impl std::error::Error for DeclarationFailures {}

#[derive(Clone, Default)]
pub struct NativeDeclared {
    pub refusals: Vec<DeclarationRefusal>,
    pub batches: BTreeMap<&'static str, DeclaredContent>,
}

impl NativeDeclared {
    pub(in crate::game) fn stage<D, T>(
        &mut self,
        listen: &mut dyn FnMut(&mut DeclarationSink<'_, D>) -> bool,
        mut validate: impl FnMut(D) -> Result<T, DeclarationError>,
        mut publish: impl FnMut(&mut DeclaredContent, T),
    ) {
        let mut entries = Vec::new();
        if listen(&mut |Answered { mod_id, answer, .. }| {
            entries.push((mod_id, validate(answer)?));
            Ok(())
        }) {
            for (owner, value) in entries {
                publish(self.batches.entry(owner).or_default(), value);
            }
        }
    }
}

/// The same native declarations serve startup and every pack reload.
pub fn native_declared() -> std::borrow::Cow<'static, NativeDeclared> {
    let collect = || {
        use super::api_values::TryIntoEngine;
        let mut declared = NativeDeclared::default();
        let context = RegistrationContext::new();
        crate::game::native_hooks::native_blocks(&context, &mut |listen| {
            declared.stage(
                listen,
                |answer| {
                    answer
                        .try_into_engine(&())
                        .and_then(|b| b.validate_local())
                        .map_err(|reason| DeclarationError::Rejected {
                            id: answer.id,
                            reason,
                        })
                },
                |batch, block| batch.blocks.push(block),
            );
        });
        crate::game::native_hooks::native_items(&context, &mut |listen| {
            declared.stage(
                listen,
                |answer| {
                    answer
                        .try_into_engine(&())
                        .and_then(|i| i.validate_local())
                        .map_err(|reason| DeclarationError::Rejected {
                            id: answer.id,
                            reason,
                        })
                },
                |batch, item| batch.items.push(item),
            );
        });
        declared.refusals = mod_api::__private::take_declaration_refusals(&context);
        for (owner, kind) in super::api_values::block_entity_declarations() {
            declared.batches.entry(owner).or_default().types.push(kind);
        }
        for &(owner, ref draft) in mod_api::__private::declared_world_fluids() {
            let fluid = draft.try_into_engine(&()).unwrap();
            let batch = declared.batches.entry(owner).or_default();
            if let Some(block) = fluid.world_block() {
                // The generated template and fields are fixed by FluidLook, not the mod.
                batch.blocks.push(block.validate_local().unwrap());
            }
            if let Some(bucket) = fluid.world_bucket() {
                batch.items.push(bucket.validate_local().unwrap());
            }
            batch.fluids.push(fluid);
        }
        declared
    };
    // `Hooks::blocks` and `Hooks::items` are startup registration: the platform holds it
    // open on this thread while they run. Without a platform every declaration is refused
    // as closed, by name.
    let declare = || {
        let mut declared = None;
        if let Ok(platform) = mod_api::__private::platform::platform() {
            (platform.with_registration_open)(&mut || declared = Some(collect()));
        }
        declared.unwrap_or_else(collect)
    };
    // Production freezes one loaded mod set for startup and reload. Tests replace
    // that set in thread-local scopes, so a process-wide snapshot would retain
    // whichever fixture reached content loading first, even on another thread.
    #[cfg(not(any(test, feature = "test-support")))]
    {
        static DECLARED: std::sync::OnceLock<NativeDeclared> = std::sync::OnceLock::new();
        std::borrow::Cow::Borrowed(DECLARED.get_or_init(declare))
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        #[cfg(test)]
        crate::game::api_values::install_test_platform();
        std::borrow::Cow::Owned(declare())
    }
}

pub fn collect_key_mappings<'declared>(
    role: ExecutionRole,
    declared: &'declared [(&'static str, mod_api::KeyRegistration)],
) -> &'declared [(&'static str, mod_api::KeyRegistration)] {
    if role.is_client() { declared } else { &[] }
}

/// The native mods' map decoration types (`native_map_decoration_types`), frozen once at start on
/// every side.
pub fn register_map_decoration_types() {
    let mut types = Vec::new();
    crate::game::native_hooks::native_map_decoration_types(&RegistrationContext::new(), &mut types);
    sunrise_sim::map_decoration_types::register(
        types,
        crate::game::maps::is_vanilla_decoration_sprite,
    );
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
/// The merged content root and the accepted content mods behind it, before
/// any process-wide registration. A caller that only reads the overlay (the
/// reload tests) must not freeze the mod registries for the rest of the
/// process.
pub struct ContentOverlay {
    /// The root with the mods' content merged in.
    pub root: sunrise_assets::resource_source::ResourceRoot,
    /// The accepted content mods, in overlay order.
    pub mods: Vec<sunrise_sim::mod_content::ContentMod>,
    /// The protocol entries of the accepted mods.
    pub announced: Vec<sunrise_protocol::mod_list::Entry>,
    /// Whether the protocol's local mod list is still unset.
    pub announce: bool,
    /// The accepted content-only mods, each with its version, for the worlds saved to record.
    pub content_mods: Vec<sunrise_sim::save::level_mods::ModRecord>,
}

/// A package's primary namespace is not an alias another layer can acquire.
pub fn refuse_primary_namespace_claims(
    manifests: &[&sunrise_mod_package::Manifest],
) -> sunrise_game_data::Result<()> {
    let mut primary: std::collections::HashSet<&str> = manifests
        .iter()
        .map(|manifest| manifest.id.as_str())
        .collect();
    for id in mod_api::__private::installed_code_ids() {
        primary.insert(id);
    }
    for manifest in manifests {
        if let Some(namespace) = manifest
            .namespaces
            .iter()
            .find(|ns| primary.contains(ns.as_str()))
        {
            return Err(format!(
                "mod {} claims namespace {namespace}, which is the primary namespace of mod {namespace}; turn one of them off",
                manifest.id,
            ).into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServedContentStage {
    Read,
    Admit,
}

#[derive(Debug)]
pub struct ServedContentLoadError {
    pub mod_id: String,
    pub stage: ServedContentStage,
    pub detail: String,
}

impl std::fmt::Display for ServedContentLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stage = match self.stage {
            ServedContentStage::Read => "read",
            ServedContentStage::Admit => "admit",
        };
        write!(
            f,
            "Sunrise: served mod {} content failed to {stage}: {}",
            self.mod_id, self.detail
        )
    }
}

impl std::error::Error for ServedContentLoadError {}

#[derive(Debug, PartialEq, Eq)]
pub struct DisabledServingHost {
    pub content: String,
    pub host: String,
}

impl std::fmt::Display for DisabledServingHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "served mod {} cannot load while its serving host {} is disabled",
            self.content, self.host
        )
    }
}

impl std::error::Error for DisabledServingHost {}

/// Validate namespace ownership and `served_by` rules before installing the
/// frozen `PackageInfo` inventory that registration hooks read.
pub fn validate_startup_packages(sources: &ModsState) -> sunrise_game_data::Result<()> {
    for selected in sources.selected.iter() {
        let manifest = &selected.package.manifest;
        if let Some(host) = manifest.served_by.as_deref()
            && sources.startup_code_disabled(host)
        {
            return Err(Box::new(DisabledServingHost {
                content: manifest.id.clone(),
                host: host.to_owned(),
            }));
        }
    }
    let manifests: Vec<&sunrise_mod_package::Manifest> = sources
        .native
        .packages()
        .iter()
        .map(|p| &p.manifest)
        .chain(sources.selected.iter().map(|p| &p.package.manifest))
        .collect();
    refuse_primary_namespace_claims(&manifests)?;
    sunrise_mod_package::dependencies::validate_served_by(&manifests)?;
    Ok(())
}

/// The owning package ID for an admitted content layer: the serving native host for a
/// `served_by` content package, otherwise the layer itself.
pub fn content_layer_owner<'a>(
    layer_id: &'a str,
    selected: &'a [sunrise_mod_package::selection::SelectedPackage],
) -> &'a str {
    selected
        .iter()
        .find(|p| p.package.manifest.id == layer_id)
        .and_then(|p| p.package.manifest.served_by.as_deref())
        .unwrap_or(layer_id)
}

/// Builds the content overlay: every enabled or dev content mod's files
/// merged over `root`. Registration of what the mods declare is
/// [`content_root`]'s step.
pub fn content_overlay(
    root: &std::path::Path,
    sources: &ModsState,
    jar_store: &std::path::Path,
) -> sunrise_game_data::Result<ContentOverlay> {
    let admission = sources.startup_admission();
    let refused = match admission {
        Some(admission) => admission.refused.clone(),
        None => sources.missing_jar_refusals(jar_store)?,
    };
    content_overlay_with_refusals(root, sources, jar_store, admission, &refused)
}

pub(in crate::game) fn content_overlay_with_refusals(
    root: &Path,
    sources: &ModsState,
    jar_store: &Path,
    admission: Option<&package_files::StartupAdmission>,
    refused: &[MissingJar],
) -> sunrise_game_data::Result<ContentOverlay> {
    use sunrise_sim::mod_content::{self, ContentMod};
    let original_base = root.to_owned();

    *sources.refused.lock().unwrap() = refused.to_vec();
    if let Some(missing) = sources.fatal_missing_jar(refused) {
        return Err(missing.clone().into());
    }

    let mut known = None;
    let mut mods: Vec<ContentMod> = Vec::new();
    let mut content_mods = Vec::new();
    let announce = !sunrise_protocol::mod_list::is_set();
    let mut announced: Vec<_> = sources
        .native
        .built()
        .iter()
        .map(|m| {
            let runtime = sources
                .native
                .packages()
                .iter()
                .find(|p| p.manifest.id == m.id)
                .and_then(|p| p.manifest.runtime.as_ref());
            announced_entry(m.id, m.version, m.side, runtime)
        })
        .collect();
    struct PackageSource<'a> {
        path: &'a std::path::Path,
        manifest: &'a sunrise_mod_package::Manifest,
        content_only: bool,
    }
    let native = sources.native.packages().iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.manifest,
        content_only: false,
    });
    let content = sources.selected.iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.package.manifest,
        content_only: true,
    });
    let mut seen = std::collections::HashSet::new();
    let packages: Vec<_> = native
        .chain(content)
        .filter(|source| {
            !refused
                .iter()
                .any(|missing| missing.owners.contains(&source.manifest.id))
        })
        .filter(|source| {
            let code = source.manifest.native.is_some();
            !code || seen.insert(source.manifest.id.as_str())
        })
        .collect();
    let manifests: Vec<_> = packages.iter().map(|source| source.manifest).collect();
    refuse_primary_namespace_claims(&manifests)?;
    let overrides = sunrise_mod_package::dependencies::Overrides::read(&sources.overrides_dir)?;
    let resolution = sunrise_mod_package::dependencies::resolve_manifests(
        &manifests,
        sources.installation_side,
        &overrides,
    )
    .map_err(|error| error.to_string())?;
    let order = resolution.order;
    // Hooks are queried on the calling thread, never from a content reader.
    let declared = native_declared();
    let requests: Vec<_> = order
        .iter()
        .map(|&index| {
            let source = &packages[index];
            ContentRequest {
                path: source.path,
                manifest: source.manifest,
                admitted: admission.and_then(|a| a.package(&source.manifest.id, source.path)),
            }
        })
        .collect();
    let read = read_contents(&requests);
    for (&index, content) in order.iter().zip(read) {
        let source = &packages[index];
        let m = source.manifest;
        let content = match content {
            Ok(content) => content,
            Err(e) => {
                if m.served_by.is_some() {
                    return Err(Box::new(ServedContentLoadError {
                        mod_id: m.id.clone(),
                        stage: ServedContentStage::Read,
                        detail: e,
                    }));
                }
                log::warn!("Sunrise: mod {} content not loaded: {e}", m.id);
                continue;
            }
        };
        // What every client must have refuses a server package outright, code or content alone.
        if let Some(e) = content.server_only_refusal(m) {
            return Err(e.into());
        }
        let known = match &mut known {
            Some(types) => types,
            None => match mod_content::known_components(root) {
                Ok(types) => known.insert(types),
                Err(e) => {
                    log::warn!("Sunrise: content mods not loaded: {e}");
                    return Err(e);
                }
            },
        };
        let candidate = ContentMod {
            id: m.id.clone(),
            content,
            jars: m.jar.clone(),
        };
        match mod_content::accept_mod(&mut mods, candidate, known) {
            Ok(_) => {}
            Err(e) if m.served_by.is_some() => {
                return Err(Box::new(ServedContentLoadError {
                    mod_id: m.id.clone(),
                    stage: ServedContentStage::Admit,
                    detail: e.to_string(),
                }));
            }
            Err(e) => log::warn!("Sunrise: mod {} content not loaded: {e}", m.id),
        }
    }
    let mut refusals = declared.refusals.clone();
    if !declared.batches.is_empty() {
        let known = match &mut known {
            Some(types) => types,
            None => known.insert(mod_content::known_components(root)?),
        };
        let admission = mod_content::accept_declared(
            &mut mods,
            &declared.batches,
            &manifests,
            known,
            &mut refusals,
        );
        if !refusals.is_empty() {
            return Err(DeclarationFailures::new(refusals).into());
        }
        admission?;
    } else if !refusals.is_empty() {
        return Err(DeclarationFailures::new(refusals).into());
    }
    for (id, detail) in mod_content::drop_dangling_remainders(root, &mut mods)? {
        if manifests
            .iter()
            .any(|m| m.id == id && m.served_by.is_some())
        {
            return Err(Box::new(ServedContentLoadError {
                mod_id: id,
                stage: ServedContentStage::Admit,
                detail,
            }));
        }
        // A built native package keeps its code and Parts only with the content it declared:
        // refusing the content refuses the package, so no dependent starts without it.
        if sources.native.built().iter().any(|native| native.id == id) {
            return Err(format!("{id}: native content refused at admission: {detail}").into());
        }
        log::warn!("Sunrise: mod {id} content not loaded: {detail}");
    }
    for m in &manifests {
        let suppliers: Vec<&sunrise_mod_package::Manifest> = std::iter::once(*m)
            .chain(
                manifests
                    .iter()
                    .copied()
                    .filter(|content| content.served_by.as_deref() == Some(m.id.as_str())),
            )
            .collect();
        let batch_declarations =
            declared
                .batches
                .get(m.id.as_str())
                .into_iter()
                .flat_map(|batch| {
                    batch
                        .blocks
                        .iter()
                        .map(|b| ("block", b.get().id.as_str()))
                        .chain(batch.items.iter().map(|i| ("item", i.get().id.as_str())))
                        .chain(
                            batch
                                .types
                                .iter()
                                .map(|t| ("block entity type", t.id.as_str())),
                        )
                        .chain(batch.fluids.iter().map(|f| ("fluid", f.id.as_str())))
                });
        let registrar_declarations = mod_api::__private::client_required_declarations(&m.id);
        let all_declarations = batch_declarations.chain(
            registrar_declarations
                .iter()
                .map(|(kind, id)| (*kind, id.as_str())),
        );
        if let Some(refusal) = m.server_only_declaration_for(&suppliers, all_declarations) {
            return Err(Box::new(refusal));
        }
        if m.side == sunrise_mod_package::manifest::Side::Server
            && let Some(requirement) = mod_api::__private::client_requirement(&m.id)
        {
            return Err(format!(
                "{}: a side = \"server\" mod {requirement}; declare side = \"both\"",
                m.id
            )
            .into());
        }
    }
    validate_entity_type_families(&mods, super::general::general_type_catalog())?;
    // Content validation may refuse a package. Revalidate against what really
    // loaded so dependents cannot start with a missing required content mod.
    let loaded: Vec<_> = manifests
        .into_iter()
        .filter(|package| {
            mods.iter().any(|content| content.id == package.id)
                || sources
                    .native
                    .built()
                    .iter()
                    .any(|native| native.id == package.id)
        })
        .collect();
    let resolved = sunrise_mod_package::dependencies::resolve_manifests(
        &loaded,
        sources.installation_side,
        &overrides,
    )
    .map_err(|error| error.to_string())?;
    for &index in &order {
        let source = &packages[index];
        if !source.content_only || !mods.iter().any(|loaded| loaded.id == source.manifest.id) {
            continue;
        }
        let m = source.manifest;
        content_mods.push(sunrise_sim::save::level_mods::ModRecord {
            id: m.id.clone(),
            version: m.version.to_string(),
        });
        if announce {
            announced.push(announced_entry(
                &m.id,
                &m.version.to_string(),
                m.side,
                m.runtime.as_ref(),
            ));
        }
    }
    let cache = sunrise_level::logging::cache_dir();
    // Content admission and the live code-dispatch boundary jointly determine
    // what loaded. Namespaces and raw startup metadata cannot answer this.
    let inventory = sources.enabled_inventory(|id| mods.iter().any(|loaded| loaded.id == id));
    let prepared = super::load_conditions::content_root_with_conditions(
        root, jar_store, &mods, &cache, &inventory, sources,
    )?;
    let materialize = if sources.installation_side == sunrise_mod_package::manifest::Side::Server {
        mod_content::resolved_server_resources
    } else {
        sources
            .content_materializer
            .unwrap_or(mod_content::resolved_server_resources)
    };
    let root = materialize(root, prepared, jar_store, &mods, cache)?;
    let _ = sources.content_base.set(original_base);
    *sources.load_notices.lock().unwrap() = resolved.warnings;
    Ok(ContentOverlay {
        root,
        mods,
        announced,
        announce,
        content_mods,
    })
}

pub fn resolve_particle_providers(
    source: &sunrise_assets::resource_source::ResourceSource,
) -> Result<(), String> {
    if mod_api::__private::particles_resolved() {
        return Ok(());
    }
    // Resolve all particle providers once, alongside the other load-owned registries.
    let mut particles: Vec<String> = sunrise_level::particle_options::ParticleType::ALL
        .iter()
        .map(|ty| format!("minecraft:{}", ty.name()))
        .collect();
    particles.extend(
        source
            .list_under("assets", &["particles"])
            .into_iter()
            .filter_map(|path| {
                let path = path.strip_prefix("assets/")?;
                let (namespace, path) = path.split_once("/particles/")?;
                let path = path.strip_suffix(".json")?;
                let id = format!("{namespace}:{path}");
                mod_api::Identifier::parse_namespaced(&id)?;
                // A vanilla-namespace file the game has no particle for is a leftover of an
                // older resource extraction, not a mod's declaration: skip it, never refuse startup.
                if namespace == "minecraft"
                    && sunrise_level::particle_options::ParticleType::from_name(&id).is_none()
                {
                    log::warn!("Sunrise: ignoring {id}: no such vanilla particle in this version");
                    return None;
                }
                Some(id)
            }),
    );
    mod_api::__private::resolve_particles(particles.iter().map(String::as_str), |id| {
        sunrise_level::particle_options::ParticleType::from_name(id)
            .map(|ty| mod_api::__private::ParticleKind::Vanilla(ty as u32))
            .or_else(|| {
                (!id.starts_with("minecraft:")).then_some(mod_api::__private::ParticleKind::Custom)
            })
    })
    .map_err(|errors| errors.join("\n"))?;
    Ok(())
}
#[cfg(test)]
pub(in crate::game) mod particle_registry_reload_tests {
    #[test]
    fn an_unlisted_vanilla_provider_file_does_not_refuse_startup() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let source = sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/particles/ambient_entity_effect.json".into(),
                sunrise_assets::resource_source::Resource::bytes(Vec::new()),
            )]
            .into(),
        );
        super::resolve_particle_providers(&source).unwrap();
    }

    #[test]
    fn repeated_particle_content_load_keeps_the_published_keys() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let source = sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/invalid namespace/particles/bad name.json".into(),
                sunrise_assets::resource_source::Resource::bytes(Vec::new()),
            )]
            .into(),
        );
        super::resolve_particle_providers(&source).unwrap();
        let first = mod_api::__private::particle_vanilla_key(
            sunrise_level::particle_options::ParticleType::Smoke as u32,
        )
        .unwrap();
        super::resolve_particle_providers(&source).unwrap();
        assert_eq!(
            mod_api::__private::particle_vanilla_key(
                sunrise_level::particle_options::ParticleType::Smoke as u32
            ),
            Some(first)
        );
    }
}

/// The content root with everything the mods declare registered against the
/// process: block entities, mob effects, entity templates and menu definitions,
/// fluid looks, and the protocol's local mod list. Scalar registry keys are
/// published only after the gameplay loader chooses its resource generation.
pub fn content_root(
    root: &std::path::Path,
    sources: &ModsState,
) -> sunrise_game_data::Result<sunrise_assets::resource_source::ResourceRoot> {
    content_root_with(root, sources, sunrise_sim::mod_content::set_fluid_looks)
}

pub fn content_root_with(
    root: &Path,
    sources: &ModsState,
    set_fluid_looks: impl FnOnce(&[sunrise_sim::mod_content::ContentMod]),
) -> sunrise_game_data::Result<sunrise_assets::resource_source::ResourceRoot> {
    let _step = sunrise_level::spans::startup::step("mods.content_root");
    let jar_store = sunrise_level::logging::game_home().join("jars");
    let overlay = {
        let _step = sunrise_level::spans::startup::step("mods.overlay");
        content_overlay(root, sources, &jar_store)?
    };
    let world_fluids = sunrise_sim::mod_content::register_world_fluids(&overlay.mods)?;
    register_block_entity_types(overlay.root.prepared(), &overlay.mods)?;
    if overlay.announce {
        sunrise_protocol::mod_list::set_local(overlay.announced);
    }
    // What every level.dat saved from here on records, and what a world is checked against
    // before it opens (`level_mods`).
    sunrise_sim::save::level_mods::set_content_mods(Some(overlay.content_mods));
    if overlay.root.prepared() != root {
        log::info!(
            "Sunrise: content from {} mod(s) in {}",
            overlay.mods.len(),
            overlay.root.prepared().display()
        );
    }
    register_mob_effects(&overlay.mods, &sources.selected);
    resolve_particle_providers(overlay.root.source())?;
    // A package whose content was refused is not admitted: an optional dependent's effect Part
    // on it is dropped, while an admitted provider that lacks the effect is still refused.
    let provider_admitted = |package: &str| {
        package == "minecraft"
            || overlay.mods.iter().any(|content| content.id == package)
            || sources
                .native
                .built()
                .iter()
                .any(|native| native.id == package)
    };
    mod_api::__private::resolve_effects(
        sunrise_game_data::potions::all_effects()
            .map(|def| sunrise_game_data::potions::namespaced(def.id)),
        |id| sunrise_game_data::potions::effect(id).map(|def| def.index),
        provider_admitted,
    )
    .map_err(|errors| errors.join("\n"))?;
    mod_api::__private::validate_effect_part_owners(
        |id| sunrise_game_data::potions::effect(id)?.owner,
        provider_admitted,
    )
    .map_err(|errors| errors.join("\n"))?;
    let behavior: serde_json::Value =
        sunrise_world::json_file::read(&overlay.root.prepared().join("block-behaviours.json"))?;
    let fluids = behavior["fluids"]
        .as_array()
        .ok_or("missing fluid registry")?;
    let mut fluids = fluids
        .iter()
        .map(|value| value.as_str().ok_or("invalid fluid registry entry"))
        .collect::<Result<Vec<_>, _>>()?;
    fluids.extend(world_fluids.iter().map(String::as_str));
    mod_api::__private::resolve_fluids(
        &fluids,
        &sunrise_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    register_entity_types(&overlay.mods);
    register_menu_types(&overlay.mods)?;
    set_fluid_looks(&overlay.mods);
    sunrise_sim::mod_content::set_fluid_interactions(&overlay.mods);
    // Freeze ownership with the startup registries, before preparing Parts. Pack and data
    // reloads can change layers but cannot replace the registered blocks and block entity types.
    mod_api::__private::install_content_owners(
        overlay.mods.iter().flat_map(|m| {
            let owner = content_layer_owner(&m.id, &sources.selected);
            m.content
                .blocks
                .iter()
                .map(move |block| (block.id.as_str(), owner))
        }),
        overlay.mods.iter().flat_map(|m| {
            m.content.items.iter().map(|item| item.id.as_str()).chain(
                m.content
                    .blocks
                    .iter()
                    .filter(|block| block.item)
                    .map(|block| block.id.as_str()),
            )
        }),
        overlay.mods.iter().flat_map(|m| {
            let owner = content_layer_owner(&m.id, &sources.selected);
            m.content
                .block_entity_types
                .iter()
                .map(move |kind| (kind.id.as_str(), owner))
        }),
        sunrise_level::containers::mod_menu_types()
            .iter()
            .map(|menu| (menu.id, content_layer_owner(menu.owner, &sources.selected))),
    );
    Ok(overlay.root)
}

/// Publish the registry orders mods iterate from the accepted gameplay root.
/// Read spawn facts for those same entity keys before publishing any of them.
/// Native general types follow the prepared types in API order, not in the
/// protocol ID table; their facts come from the same frozen general catalog.
pub fn register_iterated_registries(
    root: &Path,
) -> sunrise_game_data::Result<HashMap<String, SpawnerTypeFacts>> {
    use sunrise_protocol::registry::Builtin;
    let builtin = Builtin::load(&root.join("registries.json"))?;
    let ids = &builtin
        .get("minecraft:entity_type")
        .ok_or("missing entity type registry")?
        .names;
    let prepared = ids.iter().map(String::as_str).filter(|id| !id.is_empty());
    let general = crate::game::general::general_type_catalog();
    let mut spawner_types = SpawnerTypeFacts::load_registered(root, prepared.clone())?;
    for slot in 0..general.len() {
        let (width, height) = general.dimensions(slot);
        spawner_types.insert(
            general.key(slot).to_owned(),
            SpawnerTypeFacts::general_entity(width, height),
        );
    }
    let rows = super::entity_type_rows::entity_type_rows(root);
    mod_api::__private::resolve_entity_types(prepared.chain(general.keys()), |id| {
        rows.contains(id)
            .then(|| rows.is_living(id))
            .or_else(|| general.slot(id).map(|_| false))
    })
    .map_err(|errors| errors.join("\n"))?;
    if let Some(ids) = builtin.get("minecraft:game_event") {
        mod_api::__private::resolve_game_events(ids.names.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    if let Some(ids) = builtin.get("minecraft:menu") {
        mod_api::__private::resolve_menus(ids.names.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    Ok(spawner_types)
}

pub(in crate::game) fn register_block_entity_types(
    root: &std::path::Path,
    mods: &[sunrise_sim::mod_content::ContentMod],
) -> sunrise_game_data::Result<()> {
    use sunrise_game_data::block_entity_types::{self, BlockEntityTypes};
    use sunrise_protocol::registry::Builtin;
    let path = Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = Builtin::load(&path)?;
    let names = &builtin
        .get("minecraft:block_entity_type")
        .ok_or("missing block entity type registry")?
        .names;
    let membership = mods
        .iter()
        .flat_map(|m| &m.content.block_entity_types)
        .flat_map(|kind| {
            kind.blocks
                .iter()
                .map(|block| (block.clone(), kind.id.clone()))
        })
        .collect();
    let mut types = BlockEntityTypes::new(
        membership,
        names.clone(),
        mods.iter().flat_map(|m| {
            m.content
                .block_entity_types
                .iter()
                .map(|kind| kind.id.clone())
        }),
    );
    let blocks = builtin.get("minecraft:block");
    let content_blocks: std::collections::HashSet<&str> = mods
        .iter()
        .flat_map(|m| m.content.blocks.iter().map(|block| block.id.as_str()))
        .collect();
    for block in blocks
        .into_iter()
        .flat_map(|ids| &ids.names)
        .filter(|name| !content_blocks.contains(name.as_str()))
    {
        if let Some(kind) = sunrise_sim::lan::chunks::block_entity_type(block) {
            types.template(block.clone(), block.clone(), kind.to_owned());
        }
    }
    for block in mods.iter().flat_map(|m| &m.content.blocks) {
        if let Some(kind) = sunrise_sim::lan::chunks::block_entity_type(&block.template) {
            types.template(block.id.clone(), block.template.clone(), kind.to_owned());
        }
    }
    mod_api::__private::resolve_block_entity_types(types.names(), |id| types.key(id))
        .map_err(|errors| errors.join("\n"))?;
    types.bind_tick_parts(|id| {
        let key = mod_api::__private::block_entity_type_key(id);
        let bind = |hook| key.filter(|_| mod_api::__private::block_entity_type_listens(hook, id));
        (
            bind(mod_api::HookId::BlockEntityTick),
            bind(mod_api::HookId::BlockEntityClientTick),
        )
    });
    let additions: Vec<(String, String)> = if let Some(current) = block_entity_types::current() {
        current
            .memberships()
            .map(|(block, kind)| (kind.to_owned(), block.to_owned()))
            .collect()
    } else {
        let keys: Vec<_> = types
            .names()
            .filter_map(mod_api::__private::block_entity_type_key)
            .collect();
        let mut additions = Vec::new();
        crate::game::native_hooks::native_block_entity_type_add_blocks(
            &RegistrationContext::new(),
            &keys,
            &mut additions,
        );
        additions
            .into_iter()
            .map(|(kind, block)| {
                Ok((
                    mod_api::__private::block_entity_type_name(kind)
                        .ok_or("unknown block entity type key")?
                        .to_owned(),
                    mod_api::__private::block_key_id(block)
                        .ok_or("unknown block key")?
                        .to_owned(),
                ))
            })
            .collect::<sunrise_game_data::Result<_>>()?
    };
    if additions.is_empty() {
        block_entity_types::install(types)?;
        return Ok(());
    }
    // Empty registration needs no block IDs or class data; requested additions do.
    let blocks = blocks.ok_or("missing block registry")?;
    let behavior: serde_json::Value =
        sunrise_world::json_file::read(&root.join("block-behaviours.json"))?;
    let classes: std::collections::BTreeMap<&str, Vec<&str>> = behavior["blocks"]
        .as_array()
        .ok_or("missing block behaviors")?
        .iter()
        .filter_map(|entry| {
            Some((
                entry["name"].as_str()?,
                entry["class"]
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect(),
            ))
        })
        .collect();
    // Use the original valid sets for every addition; listener order cannot relax the constraint.
    let bases: std::collections::BTreeMap<String, String> = types
        .names()
        .filter_map(|kind| {
            let mut valid = types
                .memberships()
                .filter(|(_, k)| *k == kind)
                .filter_map(|(b, _)| classes.get(b));
            let first = valid.next()?;
            let rest: Vec<_> = valid.collect();
            first
                .iter()
                .find(|class| rest.iter().all(|chain| chain.contains(class)))
                .map(|class| (kind.to_owned(), (*class).to_owned()))
        })
        .collect();
    for (kind, block) in additions {
        let (kind, block) = (kind.as_str(), block.as_str());
        if blocks.id(block).is_none() {
            return Err(format!("unknown block {block}").into());
        }
        if let Some(base) = bases.get(kind)
            && !classes
                .get(block)
                .is_some_and(|chain| chain.contains(&base.as_str()))
        {
            return Err(format!("block {block} must inherit {base} to be valid for {kind}").into());
        }
        types.add_valid(block, kind)?;
    }
    block_entity_types::install(types)?;
    Ok(())
}

/// Manifest identity for the configuration-phase mod check.
pub(in crate::game) fn announced_entry(
    id: &str,
    version: &str,
    side: mod_api::Side,
    runtime: Option<&sunrise_mod_package::manifest::RuntimeTable>,
) -> sunrise_protocol::mod_list::Entry {
    use mod_api::Side;
    use sunrise_protocol::mod_list;
    mod_list::Entry {
        id: id.to_owned(),
        version: version.to_owned(),
        side: match side {
            Side::Both => mod_list::Side::Both,
            Side::Server => mod_list::Side::Server,
            // `Side` is open to new kinds; until the list carries one, such a mod stays local.
            _ => mod_list::Side::Client,
        },
        runtime: runtime.map(sunrise_mod_package::manifest::RuntimeTable::digest),
    }
}

pub fn mob_effect_entries<'a>(
    mods: &'a [sunrise_sim::mod_content::ContentMod],
    selected: &'a [sunrise_mod_package::selection::SelectedPackage],
) -> Vec<(&'a str, &'a sunrise_mod_package::content::MobEffect)> {
    let mut effects: Vec<_> = mods
        .iter()
        .flat_map(|m| {
            let owner = content_layer_owner(&m.id, selected);
            m.content.effects.iter().map(move |effect| (owner, effect))
        })
        .collect();
    effects.sort_by(|(_, a), (_, b)| a.id.cmp(&b.id));
    effects
}

/// The mods' mob effects join vanilla's (`brewing::effect`), once the root
/// holding their network ids is built. The first root that declares effects
/// sets them, until restart: a pack reload after a Mods switch builds a root
/// with others. A root declaring none freezes nothing, and a game without
/// mob effects reads the same empty list either way.
pub fn register_mob_effects(
    mods: &[sunrise_sim::mod_content::ContentMod],
    selected: &[sunrise_mod_package::selection::SelectedPackage],
) {
    use sunrise_level::brewing::{self, Category};
    use sunrise_mod_package::content::{EffectCategory, ModifierOperation};
    use sunrise_world::attributes::Operation;
    let effects = mob_effect_entries(mods, selected);
    if effects.is_empty() {
        return;
    }
    let ids: Vec<_> = effects
        .iter()
        .map(|(owner, e)| (e.id.clone(), (*owner).to_owned()))
        .collect();
    let set = brewing::register_mod_effects(effects.into_iter().map(|(owner, e)| {
        let category = match e.category {
            EffectCategory::Beneficial => Category::Beneficial,
            EffectCategory::Harmful => Category::Harmful,
            EffectCategory::Neutral => Category::Neutral,
        };
        let modifiers = e
            .attribute_modifiers
            .iter()
            .map(|m| {
                let operation = match m.operation {
                    ModifierOperation::AddValue => Operation::AddValue,
                    ModifierOperation::AddMultipliedBase => Operation::AddMultipliedBase,
                    ModifierOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
                };
                (m.attribute.clone(), m.id.clone(), m.amount, operation)
            })
            .collect();
        brewing::ModEffect {
            sort_order: e.sort_order,
            modifiers,
            ..brewing::ModEffect::new(&e.id, e.color, category, owner)
        }
    }));
    if !set
        && brewing::mod_effects()
            .iter()
            .map(|e| (e.id, e.owner))
            .ne(ids
                .iter()
                .map(|(id, owner)| (id.as_str(), Some(owner.as_str()))))
    {
        log::warn!("Sunrise: the mods' mob effects changed; restart the game to use them");
    }
}

/// Content and general types share the entity-type key space even though their definitions
/// enter through different paths. Refuse the collision before building the content root.
pub fn validate_entity_type_families(
    mods: &[sunrise_sim::mod_content::ContentMod],
    general: &crate::game::general::GeneralTypeCatalog,
) -> Result<(), String> {
    for package in mods {
        for entity in &package.content.entity_types {
            if general.slot(&entity.id).is_some() {
                return Err(format!(
                    "mod {} declares entity type {} as both a content and a general entity type",
                    package.id, entity.id,
                ));
            }
        }
    }
    Ok(())
}

/// the mods' entity types' templates, which the tracker reads their settings
/// from, and their thrown types, as their mob effects: set once, from the
/// startup root. Living types are mobs, loaded with vanilla's from the
/// content root's `entity-types.json`.
pub(in crate::game) fn register_entity_types(mods: &[sunrise_sim::mod_content::ContentMod]) {
    use sunrise_sim::{lan::tracking, projectiles};
    let mut types = sunrise_sim::mod_content::entity_types(mods);
    let templates = types.iter().map(|t| (t.id.clone(), t.template.clone()));
    let templates_set = tracking::register_templates(templates);
    let templates_changed = !templates_set
        && tracking::templates()
            .iter()
            .copied()
            .ne(types.iter().map(|t| (t.id.as_str(), t.template.as_str())));
    types.retain(|t| t.item.is_some());
    let set = projectiles::register_thrown_types(
        types
            .iter()
            .filter_map(|t| Some((t.id.clone(), t.item.clone()?, t.damage))),
    );
    let thrown_changed = !set
        && projectiles::thrown_types()
            .iter()
            .map(|t| t.id)
            .ne(types.iter().map(|t| t.id.as_str()));
    if templates_changed || thrown_changed {
        log::warn!("Sunrise: the mods' entity types changed; restart the game to use them");
    }
}

/// the mods' menu types, as their entity types: set once, from the startup
/// root.
pub(in crate::game) fn register_menu_types(
    mods: &[sunrise_sim::mod_content::ContentMod],
) -> sunrise_game_data::Result<()> {
    use sunrise_level::containers;
    let menus = sunrise_sim::mod_content::menu_types(mods);
    menu_books::validate_registered(&menus.iter().map(|&(menu, _)| menu).collect::<Vec<_>>())?;
    let set = containers::register_mod_menu_types(menus.iter().map(|&(m, owner)| {
        let results = m.results.iter().map(|&r| usize::from(r)).collect();
        let inputs = m.inputs.iter().copied().map(usize::from).collect();
        let crafting = m.crafting.map(|c| containers::ModCrafting {
            grid: c.grid.into(),
            size: c.size.into(),
            result: c.result.into(),
        });
        let links = m.links.map(|l| containers::ModLinks {
            from: l.from.into(),
            count: l.count.into(),
        });
        (
            m.id.clone(),
            owner.to_owned(),
            m.slots.into(),
            m.data_slots,
            results,
            inputs,
            crafting,
            links,
            m.player_inventory,
            m.owned_slots.clone(),
        )
    }));
    if !set
        && containers::mod_menu_types()
            .iter()
            .map(|m| m.id)
            .ne(menus.iter().map(|(m, _)| m.id.as_str()))
    {
        log::warn!("Sunrise: the mods' menu types changed; restart the game to use them");
    }
    Ok(())
}

/// The embedded inventory, formatted for the crash report.
pub fn native_mods_detail(built: &[mod_api::__private::native::BuiltMod]) -> Option<String> {
    let list = built
        .iter()
        .map(|m| format!("{} {}", m.id, m.version))
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

/// Whether a content mod's files can switch with a world open (`mod_content::live_switch`);
/// `None` when its content can't be read.
pub(in crate::game) type Verdict = Option<std::result::Result<(), &'static str>>;

/// Each mod's [`Verdict`] by its path and modification time. Reading a mod's content walks and
/// checks every file (seconds for a large pack), and a start read each mod three times: the
/// content root, then the window's and the server thread's `start_mods`.
/// shortcut: a file edited inside an unpacked mod folder keeps the folder's time; `--dev-mod`
/// folders are never remembered, a player's are replaced whole by the launcher.
pub(in crate::game) static VERDICTS: std::sync::Mutex<
    std::collections::BTreeMap<(PathBuf, std::time::SystemTime), Verdict>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

pub(in crate::game) fn verdict_key(
    path: &std::path::Path,
) -> Option<(PathBuf, std::time::SystemTime)> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some((path.to_owned(), modified))
}

/// Remembers the verdict of `content`, read from `path` as [`sunrise_mod_package::Content::read`] does.
pub(in crate::game) fn remember_verdict(
    path: &std::path::Path,
    content: &sunrise_mod_package::Content,
) {
    if let Some(key) = verdict_key(path) {
        let verdict = Some(sunrise_sim::mod_content::live_switch(content));
        VERDICTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, verdict);
    }
}

/// The mod at `path`'s [`Verdict`], read once per version of it (never remembered for `dev`).
pub fn verdict(
    path: &std::path::Path,
    manifest: &sunrise_mod_package::Manifest,
    dev: bool,
) -> Verdict {
    let key = if dev { None } else { verdict_key(path) };
    if let Some(key) = &key
        && let Some(verdict) = VERDICTS.lock().unwrap_or_else(|e| e.into_inner()).get(key)
    {
        return *verdict;
    }
    let verdict = sunrise_mod_package::Content::read(path, manifest)
        .ok()
        .map(|content| sunrise_sim::mod_content::live_switch(&content));
    if let Some(key) = key {
        VERDICTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, verdict);
    }
    verdict
}

/// Notes a content mod's id, and whether its files can switch with a world
/// open (`mod_content::live_switch`).
pub fn live_switch_of(
    path: &std::path::Path,
    package: &sunrise_mod_package::Package,
    dev: bool,
    content_ids: &mut std::collections::BTreeSet<String>,
    live_ids: &mut std::collections::BTreeSet<String>,
) {
    if package.is_native() {
        return;
    }
    let id = &package.manifest.id;
    content_ids.insert(id.clone());
    if package.manifest.served_by.is_some() {
        log::debug!("Sunrise: mod {id} switches on a restart: served by a native mod");
        return;
    }
    match verdict(path, &package.manifest, dev) {
        // Jar layers can declare worldgen that Content.files does not list.
        Some(Ok(())) if package.manifest.jar.is_empty() => {
            live_ids.insert(id.clone());
        }
        Some(Err(why)) => log::debug!("Sunrise: mod {id} switches on a restart: {why}"),
        _ => {}
    }
}

/// Installed native mods not compiled into this executable get an explanatory row.
pub fn not_built(
    dir: &std::path::Path,
    built: &[mod_api::__private::native::BuiltMod],
    rows: &[Installed],
) -> Vec<Installed> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "smod"));
    paths.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in paths {
        // The row names the mod, so the manifest is all it needs: the package's other entries,
        // its source among them, stay unread.
        let Ok(package) = sunrise_mod_package::Package::read(&path) else {
            continue;
        };
        if !package.is_native() {
            continue;
        }
        let m = package.manifest;
        let listed = rows.iter().chain(&out).any(|row| row.id == m.id);
        if !listed && !built.iter().any(|b| b.id == m.id) {
            out.push(Installed {
                title: format!("{} {}", m.id, m.version),
                id: m.id,
                description: String::new(),
                problem: Some(mod_list::NOT_BUILT.to_owned()),
                dev: false,
                native: None,
            });
        }
    }
    out
}

pub struct PreparedLoopContext {
    pub(in crate::game) game: Arc<sunrise_world::native_load_context::LoopContext>,
    pub(in crate::game) audio: Option<sunrise_world::native_load_context::LoopContext>,
    /// A server's context for the features it generates (`alter_ground`).
    pub(in crate::game) worldgen: Option<sunrise_world::native_load_context::WorldgenContext>,
}

/// What a live content mod switch changes on the server (`Game::apply_mods_reload`).
pub struct ModsReload {
    pub root: PathBuf,
    pub data_source: sunrise_assets::resource_source::ResourceSource,
    pub data_changes: sunrise_assets::resource_source::ResourceSource,
    pub additional_data: Arc<[Arc<sunrise_level::datapack::DataPack>]>,
    pub enabled: std::collections::BTreeSet<String>,
    pub selected: Arc<[sunrise_mod_package::selection::SelectedPackage]>,
    pub refused: Arc<Mutex<Vec<MissingJar>>>,
}

impl crate::game::CommonGame<'_> {
    pub fn capture_candidate_settings(
        &self,
        resources: &std::path::Path,
    ) -> sunrise_game_data::Result<sunrise_world::native_load_context::SettingsCapture> {
        let inventory = sunrise_mod_package::enabled::EnabledMods::read(resources)?;
        // EnabledMods yields ids in sorted order.
        let ids: Vec<_> = if self.is_logical_server() {
            inventory.server_ids().collect()
        } else {
            inventory.client_ids().collect()
        };
        let registered = super::native_hooks::mod_settings::registered_setting_keys()
            .filter(|(owner, _)| ids.binary_search(owner).is_ok());
        Ok(sunrise_world::native_load_context::SettingsCapture::new(
            &crate::game::mod_data_root(),
            ids.iter().copied(),
            registered,
        ))
    }

    pub fn prepare_loop_context(
        &self,
        resources: &std::path::Path,
    ) -> sunrise_game_data::Result<PreparedLoopContext> {
        Ok(self.prepare_loop_context_from(self.capture_candidate_settings(resources)?))
    }

    /// The loop readers of a generation over the capture it was prepared from.
    pub fn prepare_loop_context_from(
        &self,
        captured: sunrise_world::native_load_context::SettingsCapture,
    ) -> PreparedLoopContext {
        let side = self.options.execution_role.loop_side();
        let context = self
            .res
            .core
            .item_data
            .context()
            .loop_context_with_capture(side, captured);
        self.loop_context_readers(context)
    }

    pub(in crate::game) fn loop_context_readers(
        &self,
        context: sunrise_world::native_load_context::LoopContext,
    ) -> PreparedLoopContext {
        let game = Arc::new(context);
        let audio = self
            .options
            .execution_role
            .is_client()
            .then(|| game.as_ref().clone());
        let worldgen = self.is_logical_server().then(|| game.worldgen_context());
        PreparedLoopContext {
            game,
            audio,
            worldgen,
        }
    }

    /// Stage a fresh root when admission changes or registered conditions need reevaluation.
    /// The caller publishes it after every server reload listener prepared.
    pub fn stage_enabled_inventory(
        &self,
    ) -> sunrise_game_data::Result<Option<sunrise_assets::resource_source::ResourceRoot>> {
        let previous = sunrise_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        let next = self
            .mods
            .enabled_inventory(|id| previous.client_ids().any(|loaded| loaded == id));
        if previous == next && !self.condition_root_needs_reload() {
            return Ok(None);
        }
        let base = self
            .mods
            .content_base
            .get()
            .cloned()
            .unwrap_or_else(|| self.vanilla_root());
        let jars = sunrise_level::logging::game_home().join("jars");
        let refused = self.mods.missing_jar_refusals(&jars)?;
        let overlay = content_overlay_with_refusals(
            &base,
            &self.mods,
            &jars,
            self.mods.startup_admission.as_deref(),
            &refused,
        )?;
        Ok(Some(overlay.root))
    }
}

pub trait ModGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Publish the initial generation's captured settings to its loop readers.
    /// An integrated server has a distinct origin and captures its own inputs.
    fn refresh_loop_context(&mut self) -> sunrise_game_data::Result<()> {
        let role = self.options.execution_role;
        let load = self.res.core.item_data.context();
        let context = if load.origin() == role.data_load_origin() {
            self.loop_context_readers(load.captured_loop_context(role.loop_side()))
        } else {
            self.prepare_loop_context(&self.options.resources)?
        };
        self.install_loop_context(context);
        Ok(())
    }

    fn install_loop_context(&mut self, context: PreparedLoopContext) {
        self.protect_condition_root();
        self.loop_context = context.game;
        self.refresh_recipe_property_context();
        self.side_set_audio_loop_context(context.audio);
        if let Some(worldgen) = context.worldgen {
            sunrise_world::native_worldgen_hooks::publish_context(Arc::new(worldgen));
        }
        self.sync_native_settings_from_loop_context();
    }

    /// Apply the player's switches at the canonical native dispatch boundary.
    fn sync_native_switches(&self) {
        let (effective, _) = self
            .mods
            .reconcile_code_switches(&crate::game::Side::mods_list(self.side_state()).disabled);
        self.side_retire_disabled_instance_groups(&effective);
        if self.options.execution_role.is_client() {
            lang::set_disabled_mod_entries(&effective);
        }
        self.mods.apply_code_switches(&effective);
    }

    /// Lists the installed mods on the Mods page. Content mods load with the resources
    /// (`content_root`); native ones are built into the game (`native_hooks.rs`). Unsupported
    /// files get the same unreadable-file row as any other invalid package.
    fn start_mods(&mut self) {
        let dir = crate::game::game_dir().join(mod_list::DIR);
        let mut paths = sunrise_mod_package::find(&dir);
        paths.retain(|p| !self.mods.dev.contains(p));
        paths.extend(self.mods.dev.iter().cloned());
        let (mut content_ids, mut live_ids) = Default::default();
        let mut info = std::collections::BTreeMap::new();
        let installed = paths
            .iter()
            .map(|path| {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let package = match sunrise_mod_package::Package::read(path) {
                    Ok(package) => package,
                    Err(e) => {
                        return Installed {
                            id: String::new(),
                            title: file.into_owned(),
                            description: String::new(),
                            problem: Some(format!("Can't be read: {e}")),
                            dev: false,
                            native: None,
                        };
                    }
                };
                let m = &package.manifest;
                live_switch_of(
                    path,
                    &package,
                    self.mods.dev.contains(path),
                    &mut content_ids,
                    &mut live_ids,
                );
                info.insert(
                    m.id.clone(),
                    mod_list::ModInfo::of(m, package.icon.as_ref()),
                );
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: m.description.clone(),
                    problem: None,
                    dev: self.mods.dev.contains(path),
                    native: None,
                }
            })
            .collect();
        crate::game::Side::mods_list_mut(self.side_state_mut()).installed = installed;
        for (id, info) in &mut info {
            info.warnings = mod_api::__private::registration_warnings(id)
                .into_iter()
                .map(|w| (w.key().to_owned(), w.args().to_vec()))
                .collect();
        }
        crate::game::Side::mods_list_mut(self.side_state_mut()).info = info;
        (self.mods.content_ids, self.mods.live_ids) = (content_ids, live_ids);
        if self.mods.applied.is_none() {
            self.mods.applied = Some(
                crate::game::Side::mods_list(self.side_state())
                    .enabled
                    .clone(),
            );
        }
        if self.mods.code_switches.is_none() {
            let (common, state) = self.common_and_state_mut();
            common
                .mods
                .capture_code_switches(&crate::game::Side::mods_list(state).disabled);
        }
        // A mod with a native version built in says so instead, and isn't news.
        // The switches first: a row's config screen is offered only while its mod is on.
        self.sync_native_switches();
        let built = self.mods.native.built();
        let built_mods = native_mods_detail(built);
        if let Some(list) = built_mods {
            mod_api::content::crash::set_detail("Native Mods", list);
        }

        let native_dir = crate::game::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(
            &native_dir,
            built,
            &crate::game::Side::mods_list(self.side_state()).installed,
        );
        crate::game::Side::mods_list_mut(self.side_state_mut())
            .installed
            .extend(unbuilt);
        let (common, state) = self.common_and_state_mut();
        common
            .mods
            .report_refusals(&mut crate::game::Side::mods_list_mut(state).installed);
        crate::game::Side::mods_list_mut(state)
            .apply_dependency_notices(&common.mods.load_notices.lock().unwrap());
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ModGameExt1<'__sg> for G {}

/// Mob types loaded from a scratch root whose `entity-types.json` has a
/// zombie, a villager, a wandering trader, a wolf, a cat and a parrot, and the
/// rows a content root writes for a mod's types (`mod_content`): `ruby:ghoul`
/// (a zombie with 40 health), `ruby:settler` (a villager as it is),
/// `ruby:merchant` (a wandering trader), the tameable `ruby:hound`,
/// `ruby:tabby` and `ruby:macaw` (a wolf, a cat and a parrot) and `ruby:bolt`
/// (a thrown snowball, not a mob).
#[cfg(any(test, feature = "test-support"))]
pub fn living_mod_types()
-> std::collections::HashMap<String, std::sync::Arc<sunrise_sim::mobs::MobType>> {
    with_living_mod_type_fixture(|_, types| types)
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_living_mod_type_fixture<R>(
    run: impl FnOnce(
        &Path,
        std::collections::HashMap<String, std::sync::Arc<sunrise_sim::mobs::MobType>>,
    ) -> R,
) -> R {
    let zombie =
        r#""width":0.6,"height":1.95,"eye_height":1.74,"category":"monster","fire_immune":false"#;
    let villager =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"misc","fire_immune":false"#;
    let trader =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"creature","fire_immune":false"#;
    let wolf =
        r#""width":0.6,"height":0.85,"eye_height":0.68,"category":"creature","fire_immune":false"#;
    let cat =
        r#""width":0.6,"height":0.7,"eye_height":0.35,"category":"creature","fire_immune":false"#;
    let parrot =
        r#""width":0.5,"height":0.9,"eye_height":0.54,"category":"creature","fire_immune":false"#;
    let table = format!(
        r#"{{"minecraft:zombie":{{{zombie},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.23000000417232513}}}},
        "minecraft:villager":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}}}},
        "minecraft:wandering_trader":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}}}},
        "minecraft:wolf":{{{wolf},"attributes":{{"minecraft:max_health":8.0,"minecraft:movement_speed":0.30000001192092896}}}},
        "minecraft:cat":{{{cat},"attributes":{{"minecraft:max_health":10.0,"minecraft:movement_speed":0.30000001192092896}}}},
        "minecraft:parrot":{{{parrot},"attributes":{{"minecraft:max_health":6.0,"minecraft:movement_speed":0.20000000298023224}}}},
        "minecraft:snowball":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false}},
        "ruby:ghoul":{{{zombie},"attributes":{{"minecraft:max_health":40.0,"minecraft:movement_speed":0.23000000417232513}},"template":"minecraft:zombie"}},
        "ruby:settler":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}},"template":"minecraft:villager"}},
        "ruby:merchant":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}},"template":"minecraft:wandering_trader"}},
        "ruby:hound":{{{wolf},"attributes":{{"minecraft:max_health":8.0,"minecraft:movement_speed":0.30000001192092896}},"template":"minecraft:wolf"}},
        "ruby:tabby":{{{cat},"attributes":{{"minecraft:max_health":10.0,"minecraft:movement_speed":0.30000001192092896}},"template":"minecraft:cat"}},
        "ruby:macaw":{{{parrot},"attributes":{{"minecraft:max_health":6.0,"minecraft:movement_speed":0.20000000298023224}},"template":"minecraft:parrot"}},
        "ruby:bolt":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false,"template":"minecraft:snowball"}}}}"#
    );
    // A folder per call: tests run in parallel in one process, and each removes its own.
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "sunrise-living-mod-types-{}-{call}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("entity-types.json"), table).unwrap();
    std::fs::write(
        root.join("entity-models.json"),
        r#"{"minecraft:zombie#main":{}}"#,
    )
    .unwrap();
    // No attribute catalog: the rows' values stay as written.
    let attributes = &Arc::new(
        sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [],
            "entities": {},
        }))
        .unwrap(),
    );
    let types = sunrise_sim::mobs::MobTypes::load_for_server(&root, attributes).unwrap();
    let result = run(
        &root,
        types
            .types
            .into_iter()
            .map(|(id, t)| (id, std::sync::Arc::new(t)))
            .collect(),
    );
    let _ = std::fs::remove_dir_all(&root);
    result
}

/// The block states native mods add to vanilla POI types (`native_extend_poi_types`),
/// resolved against `registry` once its block definitions are published and before any
/// world's POI tables are built; collected again when the resources reload. Each refusal
/// is logged once.
pub fn extend_poi_types(registry: &sunrise_world::registry::BlockRegistry) {
    let (additions, refusals) = collect_poi_extensions(registry);
    for refusal in refusals
        .into_iter()
        .chain(sunrise_world::poi::extend(additions))
    {
        log::warn!("Sunrise: POI type extension refused: {refusal}");
    }
}

/// Ask the mods, then turn each predicate into the complete states of `registry` it
/// selects, in registration order. A tag predicate is refused here: additions name blocks.
pub(in crate::game) fn collect_poi_extensions(
    registry: &sunrise_world::registry::BlockRegistry,
) -> (Vec<sunrise_world::poi::PoiExtension>, Vec<String>) {
    let Some(metadata) = mod_api::__private::block_definitions() else {
        return (Vec::new(), Vec::new());
    };
    let mut extensions = mod_api::PoiTypeExtensionRegistry::new(metadata);
    crate::game::native_hooks::native_extend_poi_types(
        &RegistrationContext::new(),
        &mut extensions,
    );
    let mut additions = Vec::new();
    let mut refusals = Vec::new();
    for (poi, predicate) in extensions.into_entries() {
        let Some(block) = predicate.block_key() else {
            refusals.push(format!(
                "{poi} asked for a block tag's states; an addition names a block"
            ));
            continue;
        };
        let name = match mod_api::BlockMetadata::name(metadata, block) {
            Ok(name) => name,
            Err(error) => {
                refusals.push(format!("{poi} could not resolve its block: {error:?}"));
                continue;
            }
        };
        for index in 0..registry.len() {
            let state = registry
                .state(sunrise_world::registry::BlockStateId(index as u32))
                .expect("registry index is in bounds");
            if state.name == name
                && predicate.constraints().iter().all(|constraint| {
                    state.properties.get(constraint.name.as_str()) == Some(&constraint.value)
                })
            {
                additions.push(sunrise_world::poi::PoiExtension {
                    state: state.clone(),
                    poi: poi.to_string(),
                });
            }
        }
    }
    (additions, refusals)
}

/// The fluid contents native mods give their cauldron blocks
/// (`native_register_cauldron_fluid_content`), resolved against `registry` once its block
/// definitions are published, in one table with vanilla's water and lava, which the fluid
/// capability reads; collected again when the resources reload. Each refusal is logged once.
pub fn register_cauldron_fluid_contents(registry: &sunrise_world::registry::BlockRegistry) {
    let (contents, refusals) = collect_cauldron_fluid_contents(registry);
    for refusal in refusals {
        log::warn!("Sunrise: cauldron fluid content refused: {refusal}");
    }
    sunrise_gameplay_common::cauldron_contents::install(contents);
}

/// Ask the mods, then resolve each registration against `registry` into the table after
/// vanilla's contents, in registration order: a block or fluid already in the table, an
/// unknown block or fluid, a level property the block lacks or that isn't an integer, and a
/// total of 0 or one the levels don't divide evenly are refused, with their reasons.
pub fn collect_cauldron_fluid_contents(
    registry: &sunrise_world::registry::BlockRegistry,
) -> (
    sunrise_gameplay_common::cauldron_contents::CauldronContents,
    Vec<String>,
) {
    use sunrise_gameplay_common::cauldron_contents::{CauldronContent, CauldronContents};
    let mut registrations = mod_api::CauldronFluidContentRegistry::default();
    crate::game::native_hooks::native_register_cauldron_fluid_content(
        &RegistrationContext::new(),
        &mut registrations,
    );
    let mut contents = CauldronContents::vanilla();
    let mut refusals = Vec::new();
    for registration in registrations.into_entries() {
        let resolved = match (
            mod_api::__private::block_key_id(registration.block),
            mod_api::__private::fluid_id(registration.fluid),
        ) {
            (None, _) => Err("an unknown block was given a cauldron fluid content".to_owned()),
            (Some(block), None) => Err(format!("{block} was given an unknown fluid")),
            (Some(block), Some(fluid)) => CauldronContent::resolve(
                registry,
                block,
                fluid,
                registration.total,
                registration.level.as_deref(),
            ),
        };
        if let Err(refusal) = resolved.and_then(|content| contents.add(content)) {
            refusals.push(refusal);
        }
    }
    (contents, refusals)
}

#[cfg(test)]
pub(in crate::game) mod concurrent_content_tests {
    use super::*;
    #[test]
    fn mods_read_at_once_keep_their_order_and_their_own_errors() {
        let dir = std::env::temp_dir().join(format!("sunrise-read-at-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths: Vec<PathBuf> = (0..12).map(|i| dir.join(format!("m{i}"))).collect();
        for (i, path) in paths.iter().enumerate() {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(
                path.join("sunrise-mod.toml"),
                format!(
                    "id = \"m{i}\"\nname = \"M\"\nversion = \"0.1.0\"\napi = \"0.2\"\n\
                     side = \"both\"\ncapabilities = [\"registries\"]\n"
                ),
            )
            .unwrap();
            // Mod i has i + 1 language files; mod 5 also writes into another mod's namespace.
            let lang = path.join(format!("assets/m{i}/lang"));
            std::fs::create_dir_all(&lang).unwrap();
            for n in 0..=i {
                std::fs::write(lang.join(format!("l{n}.json")), "{}").unwrap();
            }
            if i == 5 {
                std::fs::create_dir_all(path.join("assets/m0/lang")).unwrap();
                std::fs::write(path.join("assets/m0/lang/x.json"), "{}").unwrap();
            }
        }
        let packages: Vec<(&PathBuf, sunrise_mod_package::Package)> = paths
            .iter()
            .map(|p| (p, sunrise_mod_package::Package::read(p).unwrap()))
            .collect();
        let requests: Vec<_> = packages
            .iter()
            .map(|(path, package)| ContentRequest {
                path,
                manifest: &package.manifest,
                admitted: None,
            })
            .collect();
        let read = read_contents(&requests);
        assert_eq!(read.len(), 12);
        for (i, content) in read.iter().enumerate() {
            if i == 5 {
                assert!(content.as_ref().unwrap_err().contains("own namespaces"));
                continue;
            }
            let files = &content.as_ref().unwrap().files;
            assert_eq!(files.len(), i + 1, "mod {i}");
            assert!(
                files
                    .iter()
                    .all(|f| f.starts_with(&format!("assets/m{i}/")))
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
pub(in crate::game) mod server_side_tests {
    use mod_api::*;

    struct Reloads;
    #[mod_api::__subscriptions]
    impl Hooks for Reloads {
        fn add_client_reload_listeners(
            &self,
            _: &dyn ClientReloadListenerContext,
            _: &mut AddClientReloadListenersEvent<'_>,
        ) {
        }
    }
    struct Hider;
    #[mod_api::sunrise_part]
    impl BlockPart for Hider {
        fn block_render_visibility(&self, _: &dyn ClientHost, _: &mut BlockRenderVisibilityEvent) {}
    }
    struct HidesLogs;
    #[mod_api::__subscriptions]
    impl Hooks for HidesLogs {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.block_part_tag("minecraft:logs", Hider);
        }
    }
    struct Pings;
    #[mod_api::__subscriptions]
    impl Hooks for Pings {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let mut payloads = r.payload_handlers();
            let mut required = payloads.registrar("1");
            required.play_to_client(PayloadId("pings:ping".into()), None);
            required.play_to_client(PayloadId("pings:pong".into()), None);
            required
                .optional()
                .play_to_client(PayloadId("pings:hint".into()), None);
        }
    }
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.payload_handlers()
                .registrar("1")
                .optional()
                .play_to_client(PayloadId("quiet:hint".into()), None);
        }
    }

    /// What a joining client must have is named for its own mod only: a client hook, a part
    /// listening to one, the first required clientbound payload. An optional payload is not.
    #[test]
    fn what_a_client_must_have_is_named_for_its_mod_only() {
        let mods: Mods = vec![
            ModEntry::new("reloads", Box::new(Reloads)),
            ModEntry::new("hides_logs", Box::new(HidesLogs)),
            ModEntry::new("pings", Box::new(Pings)),
            ModEntry::new("quiet", Box::new(Quiet)),
        ]
        .into();
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            assert_eq!(
                mod_api::__private::client_requirement("reloads").unwrap(),
                "listens to the client hook add_client_reload_listeners"
            );
            assert_eq!(
                mod_api::__private::client_requirement("hides_logs").unwrap(),
                format!(
                    "attaches the part {}, which listens to the client hook block_render_visibility",
                    std::any::type_name::<Hider>()
                )
            );
            assert_eq!(
                mod_api::__private::client_requirement("pings").unwrap(),
                "sends the required payload pings:ping to clients"
            );
            assert_eq!(mod_api::__private::client_requirement("quiet"), None);
        });
    }
}

impl ModsState {
    pub(in crate::game) fn startup_admission(&self) -> Option<&package_files::StartupAdmission> {
        self.startup_admission
            .as_deref()
            .filter(|admission| admission.matches_selection(&self.selected))
    }
}

impl ModsState {
    pub fn perf_metadata<'a>(
        &self,
        packages: impl Iterator<Item = &'a mod_api::PackageInfo>,
    ) -> Vec<sunrise_perf_format::v1::Mod> {
        use sunrise_perf_format::v1::{Mod, ModKind};
        packages
            .map(|package| {
                let native = self
                    .native
                    .built()
                    .iter()
                    .any(|candidate| candidate.id == package.id());
                Mod {
                    id: package.id().to_owned(),
                    version: package.version().to_owned(),
                    kind: match package.kind() {
                        mod_api::PackageKind::Content => ModKind::Content,
                        mod_api::PackageKind::Code if native => ModKind::Native,
                        _ => ModKind::Unspecified,
                    } as i32,
                    // AdjacentManifest does not expose a compiled crate name.
                    crate_name: String::new(),
                }
            })
            .collect()
    }
}

use crate::game::native_hooks::mod_settings::ModSettingsGameExt as _;
use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::tables::WorldTablesExt as _;
