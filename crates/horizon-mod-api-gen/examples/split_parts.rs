//! Driver for the committed one-shot Parts source migration.
#[path = "../../../tools/refactors/split_parts.rs"]
mod split;

fn main() -> Result<(), String> {
    let root = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?);
    split::run(&root)
}
