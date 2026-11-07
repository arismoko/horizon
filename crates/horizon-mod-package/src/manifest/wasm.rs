use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

/// The most a `mod.wasm` may weigh.
pub const MAX_WASM: u64 = 64 << 20;

/// A wasm mod's `[wasm]` section, written by `cargo horizon-mod build --wasm` (or Portizon),
/// never by hand: what its one `mod.wasm` is and what it was built and linked against.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(try_from = "Fields")]
pub struct Wasm {
    sha512: String,
    toolchain: String,
    linked: std::collections::BTreeMap<String, String>,
    built_with: Option<String>,
    /// The same fingerprinted internals declaration used by a native mod.
    #[serde(skip)]
    pub(crate) internals: Vec<super::Internal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    sha512: String,
    toolchain: String,
    #[serde(default)]
    linked: std::collections::BTreeMap<String, String>,
    built_with: Option<String>,
}

impl TryFrom<Fields> for Wasm {
    type Error = String;

    fn try_from(fields: Fields) -> Result<Self, Self::Error> {
        sha512_hex(&fields.sha512).map_err(|e| format!("[wasm] sha512 {e}"))?;
        if fields.toolchain.is_empty() {
            return Err("[wasm] toolchain is empty".into());
        }
        for (id, hash) in &fields.linked {
            sha512_hex(hash).map_err(|e| format!("[wasm] linked {id} {e}"))?;
        }
        if fields.built_with.as_deref().is_some_and(str::is_empty) {
            return Err("[wasm] built_with is empty".into());
        }
        Ok(Self {
            sha512: fields.sha512,
            toolchain: fields.toolchain,
            linked: fields.linked,
            built_with: fields.built_with,
            internals: Vec::new(),
        })
    }
}

/// Checks a SHA-512 written as 128 lowercase hex digits, the form `[[jar]]` uses too.
pub(crate) fn sha512_hex(value: &str) -> Result<(), &'static str> {
    if value.len() == 128
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err("must be 128 lowercase hex digits")
    }
}

/// The SHA-512 of `bytes`, as `[wasm]` writes it.
/// The plain name prefix of a wasm mod's exports in its group, `<prefix>-<interface>`: the id
/// in the component model's kebab case. Ids are `[a-z0-9_]`, so the mapping is one to one.
pub fn export_prefix(id: &str) -> String {
    id.replace('_', "-")
}

/// Whether `id` can name a wasm mod's exports: each `_`-separated word starts with a letter,
/// as the component model's names need.
pub(crate) fn exportable(id: &str) -> bool {
    id.split('_')
        .all(|word| word.starts_with(|c: char| c.is_ascii_lowercase()))
}

pub fn sha512_of(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why a package's `mod.wasm` is refused, named on the Mods page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WasmError {
    /// `[wasm]` declared, no `mod.wasm` beside it.
    Missing,
    /// A `mod.wasm` without `[wasm]`.
    Undeclared,
    TooLarge,
    /// Not a WebAssembly file (no `\0asm` header).
    NotWasm,
    /// The bytes aren't the ones `[wasm] sha512` names.
    Hash,
}

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "[wasm] is declared but the package has no mod.wasm",
            Self::Undeclared => "mod.wasm needs a [wasm] section",
            Self::TooLarge => "mod.wasm exceeds 64 MiB",
            Self::NotWasm => "mod.wasm is not a WebAssembly file",
            Self::Hash => "mod.wasm doesn't match its [wasm] sha512",
        })
    }
}

impl std::error::Error for WasmError {}

impl Wasm {
    /// The internals the package declares, checked against the game's generated table at load.
    pub fn internals(&self) -> &[super::Internal] {
        &self.internals
    }

    /// The SHA-512 of the package's `mod.wasm`.
    pub fn sha512(&self) -> &str {
        &self.sha512
    }

    /// The exact toolchain that built it; every member of a linked group shares one.
    pub fn toolchain(&self) -> &str {
        &self.toolchain
    }

    /// The exact `mod.wasm` (by SHA-512) of each dependency it was linked against.
    pub fn linked(&self) -> &std::collections::BTreeMap<String, String> {
        &self.linked
    }

    /// The converter that made it from a Java mod (`portizon@<commit>`), when it has no
    /// Rust source of its own.
    pub fn built_with(&self) -> Option<&str> {
        self.built_with.as_deref()
    }

    /// Checks `bytes` are this section's `mod.wasm`.
    pub fn check(&self, bytes: &[u8]) -> Result<(), WasmError> {
        if bytes.len() as u64 > MAX_WASM {
            return Err(WasmError::TooLarge);
        }
        if !bytes.starts_with(b"\0asm") {
            return Err(WasmError::NotWasm);
        }
        if sha512_of(bytes) != self.sha512 {
            return Err(WasmError::Hash);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(bytes: &[u8]) -> Wasm {
        Wasm::try_from(Fields {
            sha512: sha512_of(bytes),
            toolchain: "1.99.1".into(),
            linked: Default::default(),
            built_with: None,
        })
        .unwrap()
    }

    #[test]
    fn a_module_is_checked_by_header_size_and_hash() {
        let module = b"\0asm\x01\0\0\0".to_vec();
        let wasm = section(&module);
        assert_eq!(wasm.check(&module), Ok(()));
        let mut changed = module.clone();
        changed[7] = 1;
        assert_eq!(wasm.check(&changed), Err(WasmError::Hash));
        assert_eq!(section(b"MZ").check(b"MZ"), Err(WasmError::NotWasm));
    }

    #[test]
    fn the_section_refuses_malformed_hashes() {
        let text = |sha: &str| format!("sha512 = \"{sha}\"\ntoolchain = \"1.99.1\"\n");
        assert!(toml::from_str::<Wasm>(&text(&"a".repeat(128))).is_ok());
        assert!(toml::from_str::<Wasm>(&text(&"A".repeat(128))).is_err());
        assert!(toml::from_str::<Wasm>(&text("abc")).is_err());
    }
}
