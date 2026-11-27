pub(in crate::game) use super::format_state::{
    LevelEnvironments, LevelFormat, pack_environment, same_stacks, sane_facts,
};
use sunrise_gameplay_common::game::Dim;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The client's table of the server's levels on entering `entered`: a level past vanilla's
    /// three that the process does not know yet is added, so that [`Dim::from_id`] finds it, and
    /// the level left is dropped when the server opened it after Login (a runtime level), so that a
    /// long session through many of them keeps none. A viewed world of such a level needs no
    /// entry: it keeps its own name. Only a level this connection added leaves the table: in single
    /// player the integrated server shares the table, and a runtime level it opened is its own
    /// until it closes it (`runtime_levels.rs`).
    pub(in crate::game::client_side) fn track_remote_levels(&mut self, entered: &str) {
        if let Some(r) = self.client.remote.as_mut()
            && r.level != entered
            && !Dim::is_vanilla(&r.level)
            && !r.levels.contains(&r.level)
        {
            if r.added_levels.remove(&r.level) {
                Dim::remove_extra(&r.level);
            }
            r.facts_cache.remove(&r.level);
        }
        if !Dim::is_vanilla(entered)
            && !Dim::exists(entered)
            && let Some(r) = self.client.remote.as_mut()
        {
            Dim::add_extra(entered);
            r.added_levels.insert(entered.to_owned());
        }
    }
}
