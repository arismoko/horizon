use super::*;

#[allow(dead_code)]
impl crate::game::CommonGame<'_> {
    /// This `Game` is the window's, not a server's.
    pub fn native_window(&self) -> bool {
        self.options.execution_role == ExecutionRole::Client
    }
}
