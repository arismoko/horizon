//! One-shot, verbatim item extraction; run through the api-gen example driver.
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use std::{fs, path::Path};
use syn::{Item, spanned::Spanned};

fn mentions(tokens: TokenStream, needle: &str) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == needle,
        TokenTree::Group(group) => mentions(group.stream(), needle),
        _ => false,
    })
}

fn authoring(item: &Item) -> bool {
    match item {
        Item::Impl(item) => mentions(item.self_ty.to_token_stream(), "PartRegistrar"),
        Item::Struct(item) => item.ident == "PartRegistrar",
        Item::Fn(item) => {
            matches!(
                item.sig.ident.to_string().as_str(),
                "insert_part" | "namespaced" | "with_registering_keys" | "install_parts"
            ) || ["PartRegistrar", "register_all", "install_registrar"]
                .iter()
                .any(|name| mentions(item.to_token_stream(), name))
        }
        Item::Type(item) => mentions(item.ty.to_token_stream(), "PartRegistrar"),
        Item::Macro(item) => mentions(item.mac.tokens.clone(), "REGISTERING_KEYS"),
        Item::Mod(item) => item.ident == "tests",
        _ => false,
    }
}

fn byte_offset(source: &str, location: proc_macro2::LineColumn) -> usize {
    source
        .split_inclusive('\n')
        .take(location.line - 1)
        .map(str::len)
        .sum::<usize>()
        + location.column
}

fn move_file(source: &Path, destination: &Path) -> Result<bool, String> {
    let text = fs::read_to_string(source).map_err(|e| e.to_string())?;
    let parsed = syn::parse_file(&text).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut ranges = Vec::new();
    for item in parsed.items.iter().filter(|item| authoring(item)) {
        let span = item.span();
        let mut start = byte_offset(&text, span.start());
        // Keep ordinary comments such as the public-type No Wire reason with
        // the item as well as doc attributes already covered by its AST span.
        while start > 0 {
            let previous = text[..start].trim_end_matches([' ', '\t', '\r', '\n']);
            let line = previous
                .rsplit_once('\n')
                .map_or(previous, |(_, line)| line);
            if !line.trim_start().starts_with("//") || line.trim_start().starts_with("//!") {
                break;
            }
            start = previous.len() - line.len();
        }
        ranges.push((start, byte_offset(&text, span.end())));
    }
    if ranges.is_empty() {
        return Ok(false);
    }
    let mut retained = String::new();
    let mut moved = String::from(
        "//! Concrete authoring extracted verbatim from the vocabulary layer.\nuse super::*;\n\n",
    );
    let mut offset = 0;
    for (start, end) in ranges {
        if start < offset {
            return Err(format!("overlapping extraction in {}", source.display()));
        }
        retained.push_str(&text[offset..start]);
        moved.push_str(&text[start..end]);
        moved.push_str("\n\n");
        offset = end;
    }
    retained.push_str(&text[offset..]);
    fs::write(destination, moved).map_err(|e| e.to_string())?;
    fs::write(source, retained).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn run(root: &Path) -> Result<(), String> {
    let source = root.join("crates/horizon-mod-vocab/src/parts");
    let destination = root.join("crates/horizon-mod-api/src/parts");
    if destination.exists() {
        return Err("Parts authoring destination already exists".into());
    }
    fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    let mut files = fs::read_dir(&source)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    files.sort_by_key(|entry| entry.file_name());
    let mut count = 0;
    for entry in files {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") || entry.file_name() == "mod.rs" {
            continue;
        }
        if move_file(&path, &destination.join(entry.file_name()))? {
            count += 1;
        }
    }
    fs::write(destination.join("mod.rs"), "//! Owned authoring and validation; lookup storage stays below the engine.\ninclude!(concat!(env!(\"OUT_DIR\"), \"/parts_registration_modules.rs\"));\nimpl Registrar for PartRegistrar {}\n")
        .map_err(|e| e.to_string())?;
    let module = source.join("mod.rs");
    let text = fs::read_to_string(&module).map_err(|e| e.to_string())?;
    fs::write(
        module,
        text.replace("impl Registrar for PartRegistrar {}\n", ""),
    )
    .map_err(|e| e.to_string())?;
    println!("Moved concrete authoring and registration tests from {count} Parts modules");
    Ok(())
}
