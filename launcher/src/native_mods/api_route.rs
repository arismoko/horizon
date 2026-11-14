//! The API route (SDK design §1, PR 3b): an instance whose native mods are all API-only builds
//! them, from the prebuilt package's `api` files alone, into the `horizon_native_mods` library
//! that the package's prebuilt API executable loads in place of its stub. The executable is never
//! relinked and the `engine` archive is never read; the library, like any native build, is exact
//! to the game build.
use super::{api, dependencies, internals, prebuilt};
use crate::i18n::trf;
use crate::rust_toolchain::Toolchain;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// The template the mods library is written from, in the `api` slice.
pub(super) const TEMPLATE: &str = "native_mods.rs.template";
/// The folder of the `runtime` slice holding the API executable and its stub.
const BIN: &str = "bin";

/// Compiles the mods (`resolution`'s order) and the `horizon_native_mods` library against the
/// `api` slice alone, into `out`, and links the library against the `runtime` slice's game
/// libraries. Returns the `runtime` slice's API executable and the libraries to install beside
/// it: the game's, std, the toolchain's runtime on Windows, and the mods library.
pub(super) fn compile(
    rust: &Toolchain,
    runtime: &Path,
    api: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let sdk = prebuilt::identity(runtime)?;
    let Some(native_mods) = &sdk.native_mods else {
        return Err(crate::i18n::tr(
            "This prebuilt game has no API route. Update Horizon and rebuild the mods against this game.",
        )
        .to_string());
    };
    reserved(projects, &native_mods.krate)?;
    let _ = fs::remove_dir_all(out);
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let deps = api.join("deps");
    let host = api.join("host");
    let search = |edition: &str| prebuilt::rustc_with(rust, &[&deps, &host, out], edition);
    let mut libraries = HashMap::new();
    for (package, name) in api::PACKAGES {
        if package != api::INTERNALS_PACKAGE {
            libraries.insert(name, prebuilt::library(&deps, name)?);
        }
    }
    let mut rlibs: HashMap<&str, PathBuf> = HashMap::new();
    for &index in &resolution.order {
        let project = &projects[index];
        let (rlib, _) = prebuilt::compile_mod(
            project,
            &declared[index],
            out,
            search,
            |package| &libraries[package],
            &rlibs,
        )?;
        rlibs.insert(project.bundle.manifest.id.as_str(), rlib);
    }
    let catalog = internals::Catalog::read(api)?;
    let manifests: Vec<_> = resolution
        .order
        .iter()
        .map(|&i| &projects[i].bundle.manifest)
        .collect();
    let template = api::template(&api.join(TEMPLATE))?;
    let source = out.join(format!("{}.rs", native_mods.krate));
    fs::write(&source, api::main_rs(&template, &manifests, &catalog)).map_err(|e| e.to_string())?;
    let rlib = out.join(format!("lib{}.rlib", native_mods.krate));
    let mut cmd = search("2024");
    cmd.args(["--crate-type", "rlib", "--crate-name", &native_mods.krate])
        .arg("-C")
        .arg(format!("metadata={}", native_mods.metadata));
    for name in ["mod_api", "mod_points"] {
        cmd.arg("--extern")
            .arg(format!("{name}={}", libraries[name].display()));
    }
    for (position, &index) in resolution.order.iter().enumerate() {
        let id = projects[index].bundle.manifest.id.as_str();
        cmd.arg("--extern")
            .arg(format!("mod_{position}={}", rlibs[id].display()));
    }
    cmd.arg("-o").arg(&rlib).arg(&source);
    super::run(cmd, "Compiling the mods library", |_| {})?;

    let objects = out.join("objects");
    let mut members = Vec::new();
    for (n, archive) in std::iter::once(&rlib)
        .chain(
            resolution
                .order
                .iter()
                .map(|&i| &rlibs[projects[i].bundle.manifest.id.as_str()]),
        )
        .enumerate()
    {
        members.extend(extract_objects(archive, &objects.join(n.to_string()))?);
    }
    let runtime_deps = runtime.join("deps");
    let mut installed = prebuilt::runtime_libraries(rust, &runtime_deps)?;
    let library = out.join(library_name(&native_mods.krate));
    let std = installed
        .iter()
        .find(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            name.starts_with("libstd-") || name.starts_with("std-")
        })
        .cloned()
        .ok_or("no dynamic standard library")?;
    // Linux only: on windows-gnu rustc's list adds a second static unwinder (`-lgcc_eh`,
    // `-l:libpthread.a`) beside std's `libgcc_s_seh-1.dll`, and macOS's system libraries come
    // with std's dylib; both keep the link lines the slice proof measured.
    let system = match std::env::consts::OS {
        "windows" | "macos" => Vec::new(),
        _ => native_libraries(rust, runtime, out)?,
    };
    super::run(
        link(rust, &library, &members, &runtime_deps, &std, &system)?,
        "Linking the mods library",
        |_| {},
    )?;
    installed.push(library);
    Ok((runtime.join(BIN).join(crate::backend::CLIENT), installed))
}

/// Refuses a mod whose crate takes the mods library's own name: its rlib would be overwritten by
/// the library's.
fn reserved(projects: &[api::Project<'_>], krate: &str) -> Result<(), String> {
    match projects.iter().find(|project| project.crate_name() == krate) {
        Some(project) => Err(trf(
            "{mod}: its crate is named {crate}, the name of the game's mods library. Rename its library.",
            &[("mod", &project.bundle.manifest.id), ("crate", krate)],
        )),
        None => Ok(()),
    }
}

/// The C libraries Rust's standard library needs on this target, as rustc itself lists them for
/// a static library (`--print native-static-libs`): on Linux these include `-lm`, which a mod's
/// float maths calls into. The answer depends only on the toolchain, so it is kept beside the
/// prebuilt game's slices (`runtime`'s folder), in a file named for the Rust version and target,
/// and asked of rustc only when that file is missing.
fn native_libraries(rust: &Toolchain, runtime: &Path, out: &Path) -> Result<Vec<String>, String> {
    let cache = runtime.with_file_name(format!(
        "native-libraries-{}-{}",
        crate::rust_toolchain::VERSION,
        rust.host.target
    ));
    if let Ok(text) = fs::read_to_string(&cache) {
        return Ok(text.lines().map(str::to_owned).collect());
    }
    let source = out.join("native_libraries.rs");
    fs::write(&source, "").map_err(|e| e.to_string())?;
    let output = rust
        .command(&rust.rustc())
        .args([
            "--crate-type",
            "staticlib",
            "--crate-name",
            "native_libraries",
        ])
        .args(["--print", "native-static-libs", "-o"])
        .arg(out.join("native_libraries.a"))
        .arg(&source)
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stderr).into_owned()
        + &String::from_utf8_lossy(&output.stdout);
    let libraries =
        static_libs(&text).ok_or_else(|| format!("rustc listed no native libraries: {text}"))?;
    // Written whole, then renamed: a reader never sees half a list.
    let partial = out.join("native-libraries");
    fs::write(&partial, libraries.join("\n")).map_err(|e| e.to_string())?;
    fs::rename(&partial, &cache)
        .or_else(|_| fs::copy(&partial, &cache).map(drop))
        .map_err(|e| format!("{}: {e}", cache.display()))?;
    Ok(libraries)
}

/// The arguments of rustc's `native-static-libs:` note in `text`.
fn static_libs(text: &str) -> Option<Vec<String>> {
    text.lines().find_map(|line| {
        line.split_once("native-static-libs:")
            .map(|(_, libs)| libs.split_whitespace().map(str::to_owned).collect())
    })
}

/// The mods library's file name on this OS.
fn library_name(krate: &str) -> String {
    match std::env::consts::OS {
        "windows" => format!("{krate}.dll"),
        "macos" => format!("lib{krate}.dylib"),
        _ => format!("lib{krate}.so"),
    }
}

/// Link the object files `members` into the shared library `library`, against the game's two
/// libraries in `deps`, `std` and (Linux) the C libraries `system`, with this OS's C toolchain:
/// the rlibs' metadata members are left out, which rustc's own dylib link cannot do without the
/// game's crates as rlibs.
fn link(
    rust: &Toolchain,
    library: &Path,
    members: &[PathBuf],
    deps: &Path,
    std: &Path,
    system: &[String],
) -> Result<std::process::Command, String> {
    let game = |krate: &str| prebuilt::library(deps, krate);
    let name = library
        .file_name()
        .ok_or("invalid library path")?
        .to_string_lossy()
        .into_owned();
    let mut cmd = match std::env::consts::OS {
        "windows" => {
            // MinGW's linker takes the DLLs themselves; no import libraries are needed.
            let mut cmd = rust.command(&rust.mingw_bin().join("gcc.exe"));
            cmd.arg("-shared").arg("-o").arg(library).args(members);
            cmd.arg(game("horizon_mod_host")?)
                .arg(game("horizon_mod_wasm")?)
                .arg(std)
                .arg("-Wl,--no-undefined");
            cmd
        }
        "macos" => {
            let mut cmd = rust.command(Path::new("cc"));
            // The toolchain's architecture, not the launcher's (which may run under Rosetta).
            let arch = match rust.host.target.split('-').next() {
                Some("aarch64") => "arm64",
                Some(arch) => arch,
                None => "arm64",
            };
            cmd.args(["-arch", arch])
                .arg("-dynamiclib")
                .arg("-o")
                .arg(library)
                .args(members);
            // Found through the executable's rpath, where the stub sits.
            cmd.arg("-install_name")
                .arg(format!("@rpath/{name}"))
                .arg(game("horizon_mod_host")?)
                .arg(game("horizon_mod_wasm")?)
                .arg(std);
            cmd
        }
        _ => {
            let mut cmd = rust.command(Path::new("cc"));
            cmd.arg("-shared").arg("-o").arg(library).args(members);
            // By file name, so each needed entry names a library beside the executable.
            let file = |path: &Path| {
                format!(
                    "-l:{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            };
            cmd.arg("-Wl,--no-undefined")
                .arg("-L")
                .arg(deps)
                .arg(file(&game("horizon_mod_host")?))
                .arg(file(&game("horizon_mod_wasm")?))
                .arg("-L")
                .arg(std.parent().ok_or("invalid std path")?)
                .arg(file(std))
                .args(system);
            cmd
        }
    };
    cmd.current_dir(library.parent().ok_or("invalid library path")?);
    Ok(cmd)
}

/// Write the object files of the archive `rlib` into `into`, returning their paths: every member
/// but the crate metadata (`lib.rmeta`) and the archive's symbol tables.
fn extract_objects(rlib: &Path, into: &Path) -> Result<Vec<PathBuf>, String> {
    let shown = |e: &dyn std::fmt::Display| format!("{}: {e}", rlib.display());
    let data = fs::read(rlib).map_err(|e| shown(&e))?;
    let archive = object::read::archive::ArchiveFile::parse(&*data).map_err(|e| shown(&e))?;
    fs::create_dir_all(into).map_err(|e| shown(&e))?;
    let mut objects = Vec::new();
    for member in archive.members() {
        let member = member.map_err(|e| shown(&e))?;
        let name = String::from_utf8_lossy(member.name()).into_owned();
        if !name.ends_with(".o") {
            continue;
        }
        // A bare file name only: never a folder, `..` or an absolute path.
        let mut parts = Path::new(&name).components();
        if !matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(_)), None)
        ) {
            return Err(shown(&format!("an archive member named {name}")));
        }
        let path = into.join(&name);
        fs::write(&path, member.data(&*data).map_err(|e| shown(&e))?).map_err(|e| shown(&e))?;
        objects.push(path);
    }
    if objects.is_empty() {
        return Err(shown(&"no object files"));
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_libraries_are_rustcs_own_note() {
        let note = "note: Link against the following native artifacts when linking against this static library.\n\
                    note: native-static-libs: -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc\n";
        assert_eq!(
            static_libs(note).unwrap(),
            [
                "-lgcc_s",
                "-lutil",
                "-lrt",
                "-lpthread",
                "-lm",
                "-ldl",
                "-lc"
            ]
        );
        assert_eq!(static_libs("warning: nothing"), None);
    }

    #[test]
    fn the_native_libraries_are_asked_once_per_toolchain() {
        let folder = super::super::tests::dir("api-route-native-libraries");
        let runtime = folder.join("runtime");
        // A toolchain folder with no rustc: only the cached answer can be returned.
        for rust in super::super::tests::toolchains(folder.join("no-rust")) {
            let cache = folder.join(format!(
                "native-libraries-{}-{}",
                crate::rust_toolchain::VERSION,
                rust.host.target
            ));
            fs::write(&cache, "-lm\n-lc").unwrap();
            assert_eq!(
                native_libraries(&rust, &runtime, &folder).unwrap(),
                ["-lm", "-lc"]
            );
            fs::remove_file(&cache).unwrap();
            assert!(native_libraries(&rust, &runtime, &folder).is_err());
        }
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_mod_crate_with_the_mods_librarys_name_is_refused() {
        let folder = super::super::tests::dir("api-route-reserved");
        let write = |name: &str| {
            let dir = folder.join(name);
            super::super::tests::write_bundle(&dir, name, "");
            fs::write(
                dir.join("Cargo.toml"),
                format!("[package]\nname='{name}'\nversion='0.1.0'\n[lib]\nname='{name}'\n[dependencies]\nhorizon-mod-api={{path='.horizon/sdk/api/crates/horizon-mod-api'}}\n"),
            )
            .unwrap();
            super::super::bundle_in(&dir).unwrap().unwrap()
        };
        let (taken, fine) = (write("horizon_native_mods"), write("other"));
        let projects = [
            api::Project::read(&fine).unwrap(),
            api::Project::read(&taken).unwrap(),
        ];
        let error = reserved(&projects, "horizon_native_mods").unwrap_err();
        assert!(error.contains("horizon_native_mods"), "{error}");
        assert!(reserved(&projects[..1], "horizon_native_mods").is_ok());
    }

    #[test]
    fn only_object_members_come_out_of_an_rlib() {
        let folder = super::super::tests::dir("api-route-objects");
        let mut archive = b"!<arch>\n".to_vec();
        for (name, data) in [
            ("lib.rmeta", &b"meta"[..]),
            ("a.o", b"obj1"),
            ("b.o", b"obj2"),
        ] {
            archive.extend(
                format!(
                    "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    format!("{name}/"),
                    0,
                    0,
                    0,
                    644,
                    data.len()
                )
                .as_bytes(),
            );
            archive.extend(data);
        }
        let rlib = folder.join("libx.rlib");
        fs::write(&rlib, &archive).unwrap();
        let objects = extract_objects(&rlib, &folder.join("out")).unwrap();
        let names: Vec<_> = objects
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.o", "b.o"]);
        assert_eq!(fs::read(&objects[1]).unwrap(), b"obj2");
        // A member named out of its folder (BSD names, `#1/<length>`) is refused, never written.
        for name in ["../x.o", "a/b.o"] {
            let mut archive = b"!<arch>\n".to_vec();
            let data = [name.as_bytes(), b"obj"].concat();
            archive.extend(
                format!(
                    "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    format!("#1/{}", name.len()),
                    0,
                    0,
                    0,
                    644,
                    data.len()
                )
                .as_bytes(),
            );
            archive.extend(&data);
            fs::write(&rlib, &archive).unwrap();
            let error = extract_objects(&rlib, &folder.join("escape")).unwrap_err();
            assert!(error.contains("an archive member named"), "{name}: {error}");
        }
        assert!(!folder.join("x.o").exists());
        fs::write(&rlib, b"!<arch>\n").unwrap();
        assert!(extract_objects(&rlib, &folder.join("none"))
            .unwrap_err()
            .contains("no object files"));
    }
}
