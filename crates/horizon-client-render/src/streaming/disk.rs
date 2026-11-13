//! Do not confuse a refused disk attachment load with an absent chunk.
use super::*;

pub(super) fn generate<G: ChunkGenerator>(
    generator: &G,
    worker: &mut G::Worker,
    x: i32,
    z: i32,
) -> Option<Done> {
    if let Some(chunk) = generator.load(x, z) {
        generator.prepare_loaded(worker, &chunk);
        return Some(Done::Generated(chunk));
    }
    // `load` also answers `None` for a full record whose load callback refused;
    // generating then would overwrite the saved chunk with defaults.
    if generator.load_refused(x, z) {
        return None;
    }
    let proto = generator.proto(worker, x, z);
    // `proto` reads a saved proto record itself, and that load can refuse too;
    // only this second check sees that refusal.
    if generator.load_refused(x, z) {
        return None;
    }
    let decorated = generator.is_decorated(&proto);
    Some(Done::Proto(x, z, proto, decorated))
}
