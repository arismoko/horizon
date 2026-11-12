//! What a package's manifest says about it beyond its identity, read by other mods.
use std::collections::BTreeMap;

/// How a code package runs.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageRuntime {
    /// Native code, built against the game build it was made for and loaded beside it, not
    /// compiled into the game.
    Native,
    /// A `mod.wasm` run by the wasm runtime.
    Wasm,
}

/// One `[dependencies]` row of a package's manifest.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageDependency {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) optional: bool,
}

impl PackageDependency {
    /// A dependency on `id` within the semver requirement `version`.
    pub fn new(id: String, version: String, optional: bool) -> Self {
        Self {
            id,
            version,
            optional,
        }
    }
    /// The package it depends on.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The semver requirement on that package's version, such as `^1`.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Whether the dependent loads without it.
    pub fn optional(&self) -> bool {
        self.optional
    }
}

/// One value of a manifest's `[properties]` table.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PropertyValue {
    /// A TOML string.
    String(String),
    /// A TOML integer.
    Integer(i64),
    /// A TOML float.
    Float(f64),
    /// A TOML boolean.
    Boolean(bool),
}

/// The descriptive part of a package's manifest. Missing fields are empty or `None`.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PackageDetails {
    /// The display name.
    pub name: String,
    /// The description.
    pub description: String,
    /// The authors, in manifest order.
    pub authors: Vec<String>,
    /// The license name, such as `MIT`.
    pub license: String,
    /// The home page link.
    pub homepage: Option<String>,
    /// Where to report problems.
    pub issues: Option<String>,
    /// The icon's path inside the package.
    pub icon_path: Option<String>,
    /// How its code runs; `None` for a content package.
    pub runtime: Option<PackageRuntime>,
    /// The manifest's `[properties]` table.
    pub properties: BTreeMap<String, PropertyValue>,
}
