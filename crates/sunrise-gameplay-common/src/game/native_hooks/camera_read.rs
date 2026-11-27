thread_local! {
    pub static PROJECT_VIEW: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub struct ProjectionFrame(pub usize);
impl Drop for ProjectionFrame {
    fn drop(&mut self) {
        PROJECT_VIEW.set(self.0);
    }
}
