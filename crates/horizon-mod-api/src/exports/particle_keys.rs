mod particle_keys;
pub use particle_keys::ParticleKey;
mod __private {
    pub use crate::particle_keys::{particle_key, particle_kind, particle_name, resolve_particles};
    pub use horizon_mod_runtime::registry_keys::ParticleKind;
}
