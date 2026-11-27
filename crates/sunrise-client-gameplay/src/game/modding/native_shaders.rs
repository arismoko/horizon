//! The pick in `shaderpacks/` as the game tracks it (`game/interface/shader_packs.rs`), and what
//! became of each pack the loader was given. Sunrise's own WGSL packs (a folder with
//! `sunrise-shaders.toml`, or a `.sshader`) are no longer run: the pick of one falls back to no
//! pack with a note (`shader_pack_list::UNSUPPORTED_NOTE`).

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

thread_local! {
    // Loader outcomes survive renderer replacement; rejected files must not cause a rebuild loop.
    // The pack's files when it was last read (`fingerprint`), and whether it was accepted.
    static OUTCOMES: std::cell::RefCell<std::collections::HashMap<PathBuf, (Option<u64>, bool)>> =
        std::cell::RefCell::default();
}

/// Whether the loader accepted the pack at `path`; `None` before it answered.
pub(in crate::game) fn outcome(path: &Path) -> Option<bool> {
    OUTCOMES.with_borrow(|packs| packs.get(path).map(|(_, accepted)| *accepted))
}

pub(in crate::game) fn remember(path: &Path, stamp: Option<u64>, accepted: bool) {
    OUTCOMES.with_borrow_mut(|packs| packs.insert(path.to_owned(), (stamp, accepted)));
}

pub(in crate::game) fn refused(path: &Path, stamp: Option<u64>) -> bool {
    OUTCOMES.with_borrow(|packs| packs.get(path) == Some(&(stamp, false)))
}

#[derive(Default)]
pub(in crate::game) struct NativeShaders {
    /// The path of the picked pack (`game/interface/shader_packs.rs`); empty: none.
    pub picked: String,
    /// The pick of a pack in the removed WGSL format that already got its note in the chat.
    pub noted: String,
    /// The pick's `sunrise.hdr` request as last read (`ClientGame::picked_pack_shape`).
    pub pick_hdr: std::cell::RefCell<super::shader_packs::PickHdr>,
}

/// `SUNRISE_HDR_FRAME=float`: the frame is float where the renderer is made with no pack picked,
/// one refused, or one asking for Sunrise's HDR, as HDR without a pack is
/// (`PickedPack::hdr_float`).
pub(in crate::game) fn linear_frame_switch() -> bool {
    std::env::var("SUNRISE_HDR_FRAME").is_ok_and(|v| v == "float")
}

/// A hash of the names, sizes and modification times of the pack's files (`None` when the pack
/// is unreadable).
pub(in crate::game) fn fingerprint(path: &Path) -> Option<u64> {
    fn walk(path: &Path, hasher: &mut std::hash::DefaultHasher) -> Option<()> {
        let meta = std::fs::metadata(path).ok()?;
        path.hash(hasher);
        if meta.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(path)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for entry in entries {
                walk(&entry, hasher)?;
            }
        } else {
            meta.len().hash(hasher);
            meta.modified().ok()?.hash(hasher);
        }
        Some(())
    }
    let mut hasher = std::hash::DefaultHasher::new();
    walk(path, &mut hasher)?;
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, outcome, refused, remember};

    #[test]
    fn outcomes_and_refusals_survive_renderer_resets() {
        let root = std::env::temp_dir().join(format!("sunrise-acceptance-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("post.fsh");
        std::fs::write(&file, "first files").unwrap();
        let stamp = fingerprint(&root);
        assert_eq!(outcome(&root), None);
        remember(&root, stamp, true);
        assert_eq!(outcome(&root), Some(true));
        // Policy, SDK and GPU refusals all reach this same outcome before any resource reload.
        remember(&root, stamp, false);
        assert_eq!(outcome(&root), Some(false));
        assert!(refused(&root, fingerprint(&root)));
        std::fs::write(&file, "repaired files with a different size").unwrap();
        assert!(!refused(&root, fingerprint(&root)));
        remember(&root, fingerprint(&root), true);
        assert_eq!(outcome(&root), Some(true));
        std::fs::remove_dir_all(root).unwrap();
    }
}
