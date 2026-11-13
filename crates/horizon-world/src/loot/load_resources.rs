use super::*;

pub(super) fn load_kind(
    data: &Path,
    kind: &str,
    out: &mut HashMap<String, Value>,
) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(data) else {
        return Ok(());
    };
    let mut namespaces: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    namespaces.sort();
    for ns in namespaces {
        load_dir(&data.join(&ns).join(kind), &ns, "", out)?;
    }
    Ok(())
}

/// Every file's path, size and modification time under `dir`, in name order.
pub(super) fn hash_files(dir: &Path, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            hash_files(&path, hasher);
        } else {
            path.hash(hasher);
            meta.len().hash(hasher);
            meta.modified().ok().hash(hasher);
        }
    }
}

pub(super) fn load_dir(
    dir: &Path,
    namespace: &str,
    prefix: &str,
    out: &mut HashMap<String, Value>,
) -> io::Result<()> {
    // Through `json_folder`: a load holding the folder (the recipes, read by
    // the item tables, the recipe book and the server's sync too) shares one
    // read of it.
    let Ok(folder) = crate::json_folder::read(dir) else {
        return Ok(());
    };
    for file in &folder.files {
        let value = serde_json::from_slice(file.bytes()?)?;
        out.insert(format!("{namespace}:{prefix}{}", file.name), value);
    }
    Ok(())
}
