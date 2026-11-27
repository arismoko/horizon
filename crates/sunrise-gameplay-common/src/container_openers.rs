//! Container opener counting: `ContainerOpenersCounter` as Sunrise can answer it.
use std::collections::HashMap;

/// Of the containers players have open now, the ones at `key` (a double chest counts for both halves).
pub fn openers_at(open: &[(i32, i32, i32)], key: (i32, i32, i32)) -> i32 {
    open.iter().filter(|&&at| at == key).count() as i32
}

/// `ContainerOpenersCounter.recheckOpeners`: the recount at `key` and the count it replaces, dropping the entry at zero.
pub fn recheck_opener_count(
    counts: &mut HashMap<(i32, i32, i32), i32>,
    open: &[(i32, i32, i32)],
    key: (i32, i32, i32),
) -> (i32, i32) {
    let previous = counts.get(&key).copied().unwrap_or(0);
    let count = openers_at(open, key);
    if count == 0 {
        counts.remove(&key);
    } else if count != previous {
        counts.insert(key, count);
    }
    (previous, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chest a player opened, then closed without the close path (a death, a
    /// teleport, a dropped connection): the tracked count is stale and the
    /// five-tick recheck has to correct it, which is what closes the lid and
    /// plays the close sound.
    #[test]
    fn a_stale_opener_count_is_recounted_away() {
        let mut counts = HashMap::new();
        let chest = (1, 2, 3);
        // Opened: `incrementOpeners` made it one.
        assert_eq!(recheck_opener_count(&mut counts, &[chest], chest), (0, 1));
        assert_eq!(counts.get(&chest), Some(&1));
        // Nobody has it open any more.
        assert_eq!(recheck_opener_count(&mut counts, &[], chest), (1, 0));
        assert_eq!(counts.get(&chest), None, "the entry goes at zero");
        // The next opener counts from zero, so the lid still opens and the
        // sound still plays.
        assert_eq!(recheck_opener_count(&mut counts, &[chest], chest), (0, 1));
    }

    #[test]
    fn a_recount_that_agrees_leaves_the_count_alone() {
        let mut counts = HashMap::new();
        let chest = (1, 2, 3);
        recheck_opener_count(&mut counts, &[chest], chest);
        assert_eq!(
            recheck_opener_count(&mut counts, &[chest], chest),
            (1, 1),
            "no change, so no sound and no block event"
        );
        assert_eq!(counts.get(&chest), Some(&1));
    }

    /// A double chest answers for both halves: a player with the left half open
    /// is an opener of the right half too.
    #[test]
    fn both_halves_of_a_double_chest_count() {
        let (left, right) = ((1, 2, 3), (1, 2, 4));
        let open = [left, right];
        let mut counts = HashMap::new();
        assert_eq!(recheck_opener_count(&mut counts, &open, right), (0, 1));
        assert_eq!(counts.get(&right), Some(&1));
        // Two players on the same half, and someone on another chest.
        let elsewhere = (9, 9, 9);
        let open = [left, left, elsewhere];
        assert_eq!(recheck_opener_count(&mut counts, &open, left), (0, 2));
        assert_eq!(counts.get(&left), Some(&2));
        assert_eq!(recheck_opener_count(&mut counts, &open, elsewhere), (0, 1));
        assert_eq!(openers_at(&open, elsewhere), 1);
        assert_eq!(openers_at(&open, (0, 0, 0)), 0);
    }
}
