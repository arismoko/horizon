fn head(game: &mut Game<'_>, pos: (i32, i32, i32)) -> ::core::ops::ControlFlow<()> {
    match game.hoppers.get_mut(&pos) {
        Some(state) if state.cooldown > 1 => {
            state.cooldown -= 1;
            ::core::ops::ControlFlow::Break(())
        }
        _ => ::core::ops::ControlFlow::Continue(()),
    }
}
