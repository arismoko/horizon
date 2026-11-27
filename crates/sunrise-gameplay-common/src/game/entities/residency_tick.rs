//! Complete residency journals on both logical-server and remote-client ticks.
use crate::game::SideGame;

pub trait ResidencyTickGameExt<'g>: SideGame<'g> {
    fn with_residency_tick(
        &mut self,
        tick: impl FnOnce(&mut Self) -> super::Result<()>,
    ) -> super::Result<()> {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        let result = catch_unwind(AssertUnwindSafe(|| tick(self)));
        // Always drain and expire pass markers, preserving the original panic
        // when interrupted actor work and completion both fail.
        let finished = catch_unwind(AssertUnwindSafe(|| {
            self.resident_entities.table.get_mut().end_tick()
        }));
        match result {
            Err(payload) => resume_unwind(payload),
            Ok(result) => {
                let finished = match finished {
                    Ok(finished) => finished,
                    Err(payload) => resume_unwind(payload),
                };
                result?;
                finished
                    .map_err(|error| format!("unfinished entity residency pass: {error:?}").into())
            }
        }
    }
}

impl<'g, G: SideGame<'g>> ResidencyTickGameExt<'g> for G {}
