//! fnr-coarse prototype: the patch crate's single-function copies for a list of targets.
//! `path_proto SOURCE OUT TARGETS_FILE` writes OUT/<crate>/src/lib.rs and OUT/installs.txt.
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (source, out, list) = (&args[1], Path::new(&args[2]), &args[3]);
    let targets: Vec<String> = std::fs::read_to_string(list)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let started = std::time::Instant::now();
    match horizon_mod_api_gen::patch::generate(
        Path::new(source),
        "horizon_patch",
        &targets,
        &Default::default(),
    ) {
        Ok(krate) => {
            for (name, text) in &krate.libs {
                let dir = out.join(name).join("src");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("lib.rs"), text).unwrap();
            }
            let installs: Vec<String> = krate
                .patched
                .iter()
                .map(|p| format!("{}\t{}\t{}", p.target, p.id, p.install))
                .collect();
            std::fs::write(out.join("installs.txt"), installs.join("\n") + "\n").unwrap();
            eprintln!("generated {} copies in {:?}", krate.patched.len(), started.elapsed());
        }
        Err(refusals) => {
            for r in refusals {
                eprintln!("{r}");
            }
            std::process::exit(1);
        }
    }
}
