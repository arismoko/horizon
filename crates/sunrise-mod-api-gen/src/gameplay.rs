use crate::catalog::{Catalog, Hook, Side};
use crate::source::{GameDomain, SourceLayout};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GameplayPackage {
    Common,
    Server,
    Client,
}

const COMMON: GameDomain = GameDomain {
    package: "sunrise-gameplay-common",
    key: "sunrise_gameplay_common",
};
const SERVER: GameDomain = GameDomain {
    package: "sunrise-gameplay",
    key: "sunrise_gameplay",
};
const CLIENT: GameDomain = GameDomain {
    package: "sunrise-client-gameplay",
    key: "sunrise_client_gameplay",
};
pub const GAMEPLAY_LAYOUT: SourceLayout = SourceLayout {
    runtime: crate::source::ENTRY_DOMAIN,
    domains: &[COMMON, SERVER, CLIENT],
    entry: Some("src/entry.rs"),
};

impl GameplayPackage {
    pub(crate) const ALL: [Self; 3] = [Self::Common, Self::Server, Self::Client];

    pub(crate) fn domain(self) -> GameDomain {
        GAMEPLAY_LAYOUT.domains[self as usize]
    }

    pub(crate) fn for_side(side: Side) -> Self {
        match side {
            Side::Both => Self::Common,
            Side::Server => Self::Server,
            Side::Client => Self::Client,
        }
    }

    pub(crate) fn from_library(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|package| package.domain().key == name)
    }

    pub(crate) fn game_type(self) -> &'static str {
        match self {
            Self::Common => "CommonGame",
            Self::Server => "ServerGame",
            Self::Client => "ClientGame",
        }
    }

    pub(crate) fn common(self) -> &'static str {
        match self {
            Self::Common => "crate",
            _ => "::sunrise_gameplay_common",
        }
    }
}

pub(crate) const ROUTING: &str = "crates/sunrise-mod-api-gen/gameplay-routes.toml";
pub(crate) const FIRE: &str = "mod_api_fire_generated.rs";
pub(crate) const GOAL: &str = "mod_api_goal_view_generated.rs";

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Routing {
    pub(crate) hooks: BTreeMap<String, HookRoute>,
    pub(crate) host: BTreeMap<GameplayPackage, BTreeMap<String, HostRoute>>,
    pub(crate) items: BTreeMap<GameplayPackage, HostRoute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HookRoute {
    pub(crate) target: String,
    pub(crate) receiver_type: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Borrow {
    None,
    Shared,
    Mutable,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RouteKind {
    Local,
    Common,
    SharedTrait,
    SideOperation,
    Static,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostRoute {
    pub(crate) kind: RouteKind,
    pub(crate) owner: String,
    pub(crate) borrow: Borrow,
}

impl HostRoute {
    pub(crate) fn call(
        &self,
        package: GameplayPackage,
        method: &str,
        args: &str,
    ) -> Result<String, String> {
        use quote::ToTokens;
        let ty: syn::TypePath = syn::parse_str(&self.owner)
            .map_err(|error| format!("{method}: invalid callee owner: {error}"))?;
        let owner = ty.to_token_stream();
        let trait_route = matches!(self.kind, RouteKind::SharedTrait | RouteKind::SideOperation);
        if (self.kind == RouteKind::Static) != (self.borrow == Borrow::None)
            || (trait_route && package == GameplayPackage::Common)
        {
            return Err(format!("{method}: callee kind and receiver disagree"));
        }
        let module = ty
            .path
            .segments
            .iter()
            .next()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        let local = module == "crate";
        let shared = module == package.common().trim_start_matches("::");
        let reachable = match self.kind {
            RouteKind::Local => {
                local
                    && ty
                        .path
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == package.game_type())
            }
            RouteKind::Common | RouteKind::SharedTrait | RouteKind::SideOperation => shared,
            RouteKind::Static => local || shared,
        };
        if !reachable || ty.qself.is_some() {
            return Err(format!(
                "{method}: callee owner is outside the target's dependency graph"
            ));
        }
        let receiver = match (self.kind, self.borrow, package) {
            (_, Borrow::None, _) => None,
            (RouteKind::Common, Borrow::Shared, p) if p != GameplayPackage::Common => {
                Some("&self.common")
            }
            (RouteKind::Common, Borrow::Mutable, p) if p != GameplayPackage::Common => {
                Some("&mut self.common")
            }
            _ => Some("self"),
        };
        let args = match (receiver, args.is_empty()) {
            (None, _) => args.to_owned(),
            (Some(receiver), true) => receiver.to_owned(),
            (Some(receiver), false) => format!("{receiver}, {args}"),
        };
        if trait_route {
            let game = package.game_type();
            Ok(format!(
                "<crate::game::{game}<'_> as {owner}>::{method}({args})"
            ))
        } else {
            if ty
                .path
                .segments
                .last()
                .is_some_and(|segment| !segment.arguments.is_empty())
            {
                Ok(format!("<{owner}>::{method}({args})"))
            } else {
                Ok(format!("{owner}::{method}({args})"))
            }
        }
    }
}

impl Routing {
    pub(crate) fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(ROUTING);
        crate::rerun_if_changed(&path.display());
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
    }

    pub(crate) fn host_route(&self, package: GameplayPackage, key: &str) -> Option<&HostRoute> {
        self.host.get(&package)?.get(key)
    }

    pub(crate) fn catalog(&self, root: &Path) -> Result<Catalog, String> {
        let mut catalog = Catalog::load(root)?;
        for hook in &mut catalog.hooks {
            if let Some(route) = self.hooks.get(&hook.name) {
                hook.target.clone_from(&route.target);
                hook.receiver_type.clone_from(&route.receiver_type);
            } else {
                hook.apply_gameplay_default();
            }
        }
        Ok(catalog)
    }
}

#[derive(Serialize)]
pub struct RoutingIssue {
    pub entry: String,
    pub target: GameplayPackage,
    pub kind: &'static str,
    pub reason: String,
}

#[derive(Default, Serialize)]
pub struct GameplayAudit {
    pub routes: BTreeMap<GameplayPackage, BTreeMap<&'static str, usize>>,
    pub files: BTreeMap<GameplayPackage, Vec<&'static str>>,
    pub issues: Vec<RoutingIssue>,
    pub entries: BTreeMap<GameplayPackage, BTreeMap<String, &'static str>>,
}

impl GameplayAudit {
    pub(crate) fn count(&mut self, target: GameplayPackage, entry: &str, kind: &'static str) {
        self.entries
            .entry(target)
            .or_default()
            .insert(entry.into(), kind);
        *self
            .routes
            .entry(target)
            .or_default()
            .entry(kind)
            .or_default() += 1;
    }

    pub(crate) fn issue(
        &mut self,
        target: GameplayPackage,
        entry: &str,
        kind: &'static str,
        reason: String,
    ) {
        self.issues.push(RoutingIssue {
            entry: entry.into(),
            target,
            kind,
            reason,
        });
    }
}

pub fn audit_gameplay(root: &Path, out: &Path) -> Result<GameplayAudit, String> {
    let routing = Routing::load(root)?;
    let catalog = routing.catalog(root)?;
    let areas = crate::host::load(root)?;
    let mut audit = GameplayAudit::default();
    for package in GameplayPackage::ALL {
        let files = plan(&routing, &catalog, &areas, package, &mut audit)?;
        let dir = out.join(package.domain().package);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, code) in &files {
            crate::write_if_changed(dir.join(name), code).map_err(|e| e.to_string())?;
        }
        audit
            .files
            .insert(package, files.into_iter().map(|(name, _)| name).collect());
    }
    Ok(audit)
}

pub(crate) fn generate(
    root: &Path,
    package: GameplayPackage,
) -> Result<Vec<(&'static str, String)>, String> {
    let routing = Routing::load(root)?;
    let catalog = routing.catalog(root)?;
    let areas = crate::host::load(root)?;
    let mut audit = GameplayAudit::default();
    let files = plan(&routing, &catalog, &areas, package, &mut audit)?;
    if !audit.issues.is_empty() {
        return Err(audit
            .issues
            .into_iter()
            .map(|issue| format!("{}: {}", issue.entry, issue.reason))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    Ok(files)
}

pub(crate) fn host_source(root: &Path, package: GameplayPackage) -> Result<String, String> {
    let routing = Routing::load(root)?;
    let areas = crate::host::load(root)?;
    let mut audit = GameplayAudit::default();
    let host = crate::host::gameplay_forwarders(&areas, package, &routing, &mut audit)?;
    if let Some(issue) = audit.issues.first() {
        return Err(format!("{}: {}", issue.entry, issue.reason));
    }
    Ok(format!("mod generated {{\n{host}\n}}\n"))
}

fn plan(
    routing: &Routing,
    catalog: &Catalog,
    areas: &[crate::host::Area],
    package: GameplayPackage,
    audit: &mut GameplayAudit,
) -> Result<Vec<(&'static str, String)>, String> {
    let fire = fire(catalog, package, audit)?;
    let host = crate::host::gameplay_forwarders(areas, package, routing, audit)?;
    let mut files = vec![
        (
            crate::build::HOST_OUTPUT,
            format!("mod generated {{\n{host}\n}}\n"),
        ),
        (FIRE, fire),
    ];
    if package == GameplayPackage::Server {
        files.push((GOAL, crate::host::generate_gameplay_goal_view(areas)?));
    }
    for (name, code) in &mut files {
        let parsed =
            syn::parse_file(code).map_err(|error| format!("{package:?}/{name}: {error}"))?;
        if *name == FIRE {
            *code = prettyplease::unparse(&parsed);
        }
    }
    Ok(files)
}

fn fire(
    catalog: &Catalog,
    package: GameplayPackage,
    audit: &mut GameplayAudit,
) -> Result<String, String> {
    let mut fire = Vec::new();
    for hook in &catalog.hooks {
        let target = match hook.gameplay_binding() {
            Ok(Some(binding)) => binding,
            Ok(None) if hook.target.starts_with("sunrise_client::game::") => {
                if GameplayPackage::for_side(hook.side) == package {
                    audit.issue(
                        package,
                        &hook.name,
                        "hook",
                        format!(
                            "legacy binding {} needs its defining package and module",
                            hook.target
                        ),
                    );
                }
                continue;
            }
            Ok(None) => continue,
            Err(reason) => {
                if hook
                    .target
                    .split("::")
                    .next()
                    .and_then(GameplayPackage::from_library)
                    .unwrap_or_else(|| GameplayPackage::for_side(hook.side))
                    == package
                {
                    audit.issue(package, &hook.name, "hook", reason);
                }
                continue;
            }
        };
        if target.package != package {
            continue;
        }
        if hook.dispatch == crate::catalog::DispatchStyle::Direct {
            audit.count(package, &hook.name, "hook_direct");
            continue;
        }
        match crate::dispatch::emit(std::iter::once(hook), &catalog.draw_contexts, true) {
            Ok(mut output) => {
                audit.count(package, &hook.name, target.kind.label());
                fire.append(&mut output.fire);
            }
            Err(reason) => audit.issue(package, &hook.name, "hook", reason),
        }
    }
    Ok(crate::dispatch::render_fire(&fire))
}

pub(crate) fn validate_shared_hook(
    hook: &Hook,
    package: Option<GameplayPackage>,
    contexts: &[crate::catalog::DrawContext],
) -> Result<(), String> {
    if package == Some(GameplayPackage::Common)
        && hook.call.iter().any(|(_, ty)| {
            syn::parse_str(ty).is_ok_and(|ty| crate::DrawContext::of(contexts, &ty).is_some())
        })
    {
        return Err(format!(
            "{}: common gameplay cannot borrow a client drawing surface",
            hook.name
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
