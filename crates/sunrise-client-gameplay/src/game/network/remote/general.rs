#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use crate::game::client_side::remote_client::general::RemoteGeneral;

    #[cfg(test)]
    use crate::game::client_side::remote_client::general::tick_remote_general;

    #[cfg(test)]
    use sunrise_gameplay_common::game::remote::Lerp;

    use indexmap::IndexMap;

    #[cfg(test)]
    use std::collections::HashMap;

    #[cfg(test)]
    #[test]
    fn a_move_is_reached_in_three_ticks_and_each_tick_keeps_the_one_before() {
        let row = RemoteGeneral::new(
            0,
            1,
            [0.0, 70.0, 0.0],
            [0.0, 0.0],
            mod_api::EntityRenderAttachments::default(),
        );
        let mut rows = IndexMap::from([(5, row)]);
        let mut lerps = HashMap::from([(
            5,
            Lerp {
                target: [3.0, 70.0, 0.0],
                rotation: Some((90.0, 30.0)),
                steps: 3,
            },
        )]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            tick_remote_general(&mut rows, &mut lerps);
            let r = &rows[&5];
            seen.push((r.previous[0], r.position[0], r.rotation, r.age));
        }
        assert_eq!(seen[0], (0.0, 1.0, [30.0, 10.0], 1));
        assert_eq!(seen[1], (1.0, 2.0, [60.0, 20.0], 2));
        assert_eq!(seen[2], (2.0, 3.0, [90.0, 30.0], 3));
        // Reached: it stays, and only ages.
        assert_eq!(seen[3], (3.0, 3.0, [90.0, 30.0], 4));
    }
    /// The ids of the rows present on the window.
    pub fn row_ids(game: &crate::game::ClientGame<'_>) -> Vec<i32> {
        let mut ids: Vec<i32> = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .general
            .rows
            .keys()
            .copied()
            .collect();
        ids.sort_unstable();
        ids
    }
    pub fn rows(game: &crate::game::ClientGame<'_>) -> IndexMap<i32, RemoteGeneral> {
        game.client.remote.as_ref().unwrap().general.rows.clone()
    }
}
