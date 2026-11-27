//! A hub container click the game owns. `Hub::container_click` used to run inside
//! `self.lan.as_mut()`, so nothing it called could reach the game: a hook fired in the middle
//! of a click had no `&Game` to be given. Here the game lifts the host out of itself for the
//! click, so the click code and the game are separate values and the game is whole while the
//! hub runs.
