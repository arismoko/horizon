use super::*;
use crate::storage::tests::Home;

#[test]
fn referenced_jar_path_is_kept_and_unused_jar_is_removed() {
    use horizon_mod_package::{manifest::JarRef, Package};
    use sha2::{Digest, Sha512};
    use std::io::Write;
    let home = Home::new();
    fs::create_dir_all(home.0.join("jars")).unwrap();
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(
            "assets/example/lang/en_us.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"{}").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let jar = JarRef::new(
        "original".into(),
        "1.0.0".into(),
        format!("{:x}", Sha512::digest(&bytes)),
    )
    .unwrap();
    let referenced = jar.cache_path(&home.0.join("jars"));
    fs::write(&referenced, bytes).unwrap();
    let manifest = home.seed("horizon-mods/example/horizon-mod.toml");
    fs::write(&manifest, format!("id='example'\nname='Example'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[[jar]]\nid='original'\nversion='1.0.0'\nsha512='{}'\n", jar.sha512())).unwrap();
    Package::read(manifest.parent().unwrap()).unwrap();
    let unused = home.seed("jars/unused.jar");
    let storage = Storage::at(&home.0).unwrap();
    let mut entries = Vec::new();
    unused_jars(&mut entries, &home.0).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, unused);
    storage
        .apply(entries, Some(Category::OtherCaches), false)
        .unwrap();
    assert!(referenced.exists());
    assert!(!unused.exists());
}

#[cfg(unix)]
#[test]
fn entry_uses_held_directory_after_root_path_is_redirected() {
    let home = Home::new();
    let outside = Home::new();
    home.seed("cache/output");
    let precious = outside.seed("cache/precious");
    let storage = Storage::at(&home.0).unwrap();
    let entry = Entry::fixed(&home.0, Path::new("cache"), Category::OtherCaches).unwrap();
    let moved = home.0.with_extension("original");
    fs::rename(&home.0, &moved).unwrap();
    std::os::unix::fs::symlink(&outside.0, &home.0).unwrap();
    storage
        .apply(vec![entry], Some(Category::OtherCaches), false)
        .unwrap();
    assert!(precious.exists());
    assert!(!moved.join("cache/output").exists());
    fs::remove_file(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[cfg(unix)]
#[test]
fn hard_linked_outputs_across_entries_count_once() {
    use std::os::unix::fs::MetadataExt;
    let home = Home::new();
    let output = home.seed("native/source/first");
    fs::create_dir_all(home.0.join("native/patched")).unwrap();
    fs::hard_link(&output, home.0.join("native/patched/second")).unwrap();
    let expected = [
        output,
        home.0.join("native/source"),
        home.0.join("native/patched"),
    ]
    .into_iter()
    .map(|path| fs::metadata(path).unwrap().blocks() * 512)
    .sum::<u64>();
    let storage = Storage::at(&home.0).unwrap();
    let entries = ["native/source", "native/patched"]
        .into_iter()
        .map(|path| Entry::fixed(&home.0, Path::new(path), Category::BuildFiles).unwrap())
        .collect();
    let report = storage.apply(entries, None, false).unwrap();
    assert_eq!(
        report
            .rows
            .iter()
            .find(|row| row.category == Category::BuildFiles)
            .unwrap()
            .bytes,
        expected
    );
}
