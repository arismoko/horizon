use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A slime, magma cube or sulfur cube is dying and about to split into
    /// smaller ones (NeoForge's `MobSplitEvent`, vanilla's
    /// `AbstractCubeMob.remove`): `parent` is its entity id, `kind` its type,
    /// `size` its size and `child_size` the size of the children (half of
    /// it). `children` names the engine-owned list of the children vanilla
    /// already constructed and `edits` changes it (edit, replace, remove,
    /// append); cancel spawns none. Only for a parent above size 1, once its
    /// children are made. On the server.
    pub(in crate::game) fn native_mob_split(&mut self, call: mod_api::__private::MobSplitCall<'_>) {
        self.fire_mob_split(call);
    }
}
