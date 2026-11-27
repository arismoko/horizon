/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A slime, magma cube or sulfur cube is dying and about to split into
    /// smaller ones (NeoForge's `MobSplitEvent`, vanilla's
    /// `AbstractCubeMob.remove`): `parent` is its entity id, `kind` its type,
    /// `size` its size and `child_size` the size of the children (half of
    /// it). `children` (`MobSplitChildren<'_>`) lends the unbounded mutable list
    /// of children vanilla already constructed (inspect, edit, replace, insert,
    /// push, remove, clear); cancel spawns none. Only for a parent above size 1,
    /// once its children are made. On the server.
    pub(in crate::game) fn native_mob_split(&mut self, call: mod_api::__private::MobSplitCall<'_>) {
        self.fire_mob_split(call);
    }
}
