//! Sets of section indexes in a chunk column, a bit per section.
//!
//! Vanilla's tallest level has 254 sections (`DimensionType.Y_SIZE / 16`), and
//! light data adds a section below and above it: 256 bits hold every layout a
//! dimension type allows.

/// Section indexes of one column, 0 being the lowest.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionMask([u64; 4]);

impl SectionMask {
    pub const CAPACITY: usize = 256;
    pub const EMPTY: Self = Self([0; 4]);
    pub const ALL: Self = Self([u64::MAX; 4]);

    /// The indexes `0..n`.
    pub const fn below(n: usize) -> Self {
        let mut words = [0; 4];
        let mut w = 0;
        while w < 4 {
            let bits = n.saturating_sub(w * 64);
            words[w] = if bits >= 64 {
                u64::MAX
            } else {
                (1 << bits) - 1
            };
            w += 1;
        }
        Self(words)
    }
    pub fn single(i: usize) -> Self {
        let mut mask = Self::EMPTY;
        mask.insert(i);
        mask
    }
    #[inline]
    pub fn contains(&self, i: usize) -> bool {
        i < Self::CAPACITY && self.0[i >> 6] >> (i & 63) & 1 != 0
    }
    #[inline]
    pub fn insert(&mut self, i: usize) {
        self.0[i >> 6] |= 1 << (i & 63);
    }
    #[inline]
    pub fn remove(&mut self, i: usize) {
        self.0[i >> 6] &= !(1 << (i & 63));
    }
    pub fn is_empty(&self) -> bool {
        self.0 == [0; 4]
    }
    pub fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }
    /// The indexes from `i` up.
    pub fn from(self, i: usize) -> Self {
        self & !Self::below(i)
    }
    /// The lowest index `>= i`, if any.
    #[inline]
    pub fn first_from(self, i: usize) -> Option<usize> {
        if i >= Self::CAPACITY {
            return None;
        }
        let mut word_index = i >> 6;
        let mut word = self.0[word_index] & (!0u64 << (i & 63));
        loop {
            if word != 0 {
                return Some(word_index * 64 + word.trailing_zeros() as usize);
            }
            word_index += 1;
            if word_index == self.0.len() {
                return None;
            }
            word = self.0[word_index];
        }
    }
    /// The indexes, lowest first.
    pub fn iter(self) -> Indexes {
        Indexes(self)
    }
    /// Removes and returns up to `*budget` indexes nearest `center`, taking
    /// their number off `budget`. A column admitted part-way must spend its
    /// slots where the camera is, not on the lowest sections of a tall column.
    pub fn take_near(&mut self, budget: &mut usize, center: i32) -> Self {
        let mut taken = Self::EMPTY;
        while *budget > 0 {
            let mut best: Option<(u32, usize)> = None;
            for i in self.iter() {
                let distance = (i as i32 - center).unsigned_abs();
                if best.is_none_or(|(nearest, _)| distance < nearest) {
                    best = Some((distance, i));
                }
            }
            let Some((_, index)) = best else {
                break;
            };
            self.remove(index);
            taken.insert(index);
            *budget -= 1;
        }
        taken
    }
    /// Removes and returns the lowest `*budget` indexes (all of them when
    /// there are fewer), taking their number off `budget`. The mesh scheduler
    /// spends a budget of sections, and one column's dirty sections can be
    /// more than what is left of it: the rest stay here, for the next update.
    pub fn take_up_to(&mut self, budget: &mut usize) -> Self {
        let mut taken = Self::EMPTY;
        for (word, out) in self.0.iter_mut().zip(&mut taken.0) {
            if *budget == 0 {
                break;
            }
            let ones = word.count_ones() as usize;
            if ones <= *budget {
                *out = std::mem::take(word);
                *budget -= ones;
            } else {
                // Fewer than `ones` (at most 63) bits to take: clear the
                // lowest of them one at a time, what remains is the rest.
                let mut rest = *word;
                for _ in 0..*budget {
                    rest &= rest - 1;
                }
                *out = *word & !rest;
                *word = rest;
                *budget = 0;
            }
        }
        taken
    }
}

impl std::ops::BitOr for SectionMask {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(std::array::from_fn(|w| self.0[w] | other.0[w]))
    }
}
impl std::ops::BitOrAssign for SectionMask {
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}
impl std::ops::BitAnd for SectionMask {
    type Output = Self;
    fn bitand(self, other: Self) -> Self {
        Self(std::array::from_fn(|w| self.0[w] & other.0[w]))
    }
}
impl std::ops::Not for SectionMask {
    type Output = Self;
    fn not(self) -> Self {
        Self(self.0.map(|w| !w))
    }
}
impl std::fmt::Debug for SectionMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}
impl FromIterator<usize> for SectionMask {
    fn from_iter<I: IntoIterator<Item = usize>>(indexes: I) -> Self {
        let mut mask = Self::EMPTY;
        for i in indexes {
            mask.insert(i);
        }
        mask
    }
}

/// `SectionMask::iter`: the set's indexes in ascending order.
#[derive(Clone, Copy, Debug)]
pub struct Indexes(SectionMask);
impl Iterator for Indexes {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        let words = &mut (self.0).0;
        let w = words.iter().position(|&w| w != 0)?;
        let bit = words[w].trailing_zeros() as usize;
        words[w] &= words[w] - 1;
        Some(w * 64 + bit)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.0.len();
        (len, Some(len))
    }
}
impl ExactSizeIterator for Indexes {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_across_words() {
        let mut mask = SectionMask::EMPTY;
        for i in [0, 23, 63, 64, 130, 255] {
            mask.insert(i);
        }
        assert_eq!(mask.iter().collect::<Vec<_>>(), [0, 23, 63, 64, 130, 255]);
        assert_eq!(mask.len(), 6);
        assert!(mask.contains(130) && !mask.contains(131) && !mask.contains(300));
        mask.remove(64);
        assert_eq!(mask.from(24).iter().collect::<Vec<_>>(), [63, 130, 255]);
        assert_eq!(mask.from(256), SectionMask::EMPTY);
        assert_eq!(mask.from(0), mask);
        assert_eq!(mask.first_from(0), Some(0));
        assert_eq!(mask.first_from(24), Some(63));
        assert_eq!(mask.first_from(63), Some(63));
        assert_eq!(mask.first_from(64), Some(130));
        assert_eq!(mask.first_from(131), Some(255));
        assert_eq!(mask.first_from(255), Some(255));
        assert_eq!(mask.first_from(256), None);
        assert!(SectionMask::EMPTY.is_empty() && !mask.is_empty());
    }

    #[test]
    fn below_counts_whole_and_partial_words() {
        for n in [0, 1, 24, 63, 64, 65, 131, 254, 256] {
            let mask = SectionMask::below(n);
            assert_eq!(mask.len(), n);
            assert_eq!(mask.iter().collect::<Vec<_>>(), (0..n).collect::<Vec<_>>());
        }
        assert_eq!(SectionMask::below(1000), SectionMask::ALL);
    }

    #[test]
    fn operators() {
        let a: SectionMask = [1, 70, 200].into_iter().collect();
        let b: SectionMask = [70, 3].into_iter().collect();
        assert_eq!((a | b).iter().collect::<Vec<_>>(), [1, 3, 70, 200]);
        assert_eq!((a & b).iter().collect::<Vec<_>>(), [70]);
        assert_eq!(
            (!a & SectionMask::below(4)).iter().collect::<Vec<_>>(),
            [0, 2, 3]
        );
        assert_eq!(format!("{a:?}"), "{1, 70, 200}");
    }

    /// A partial column spends its slots on the sections nearest the camera,
    /// and the halves still make up the original set.
    #[test]
    fn take_near_spends_the_budget_around_the_centre() {
        let original: SectionMask = [0, 2, 12, 20, 23].into_iter().collect();
        let take = |budget: usize, center: i32| {
            let (mut rest, mut left) = (original, budget);
            let taken = rest.take_near(&mut left, center);
            assert_eq!(taken | rest, original);
            assert_eq!(taken & rest, SectionMask::EMPTY);
            (taken.iter().collect::<Vec<_>>(), rest, left)
        };
        // Nothing to spend: nothing moves.
        assert_eq!(take(0, 12), (vec![], original, 0));
        // The two nearest: 12 (0 away) then 20 (8), not the lowest sections.
        let (taken, rest, left) = take(2, 12);
        assert_eq!((taken, left), (vec![12, 20], 0));
        assert_eq!(rest.iter().collect::<Vec<_>>(), [0, 2, 23]);
        // A budget past the count takes everything and leaves the rest unspent.
        assert_eq!(take(9, 0), (vec![0, 2, 12, 20, 23], SectionMask::EMPTY, 4));
        // An empty mask spends nothing.
        let (mut rest, mut left) = (SectionMask::EMPTY, 5);
        assert_eq!(rest.take_near(&mut left, 3), SectionMask::EMPTY);
        assert_eq!((rest, left), (SectionMask::EMPTY, 5));
    }
    /// `take_up_to` splits off the lowest indexes, across words, and the two
    /// halves always make up the original set.
    #[test]
    fn take_up_to_takes_the_lowest_within_the_budget() {
        let original: SectionMask = [2, 5, 63, 64, 100, 190, 255].into_iter().collect();
        let take = |budget: usize| {
            let (mut rest, mut left) = (original, budget);
            let taken = rest.take_up_to(&mut left);
            assert_eq!(taken | rest, original, "budget {budget}");
            assert_eq!(taken & rest, SectionMask::EMPTY, "budget {budget}");
            (taken.iter().collect::<Vec<_>>(), rest, left)
        };
        // No budget: nothing taken, nothing changed.
        assert_eq!(take(0), (vec![], original, 0));
        // More budget than indexes: all of them, the unspent budget left.
        assert_eq!(
            take(10),
            (vec![2, 5, 63, 64, 100, 190, 255], SectionMask::EMPTY, 3)
        );
        assert_eq!(
            take(7),
            (vec![2, 5, 63, 64, 100, 190, 255], SectionMask::EMPTY, 0)
        );
        // Part of the budget: the lowest, inside the first word and past it.
        let (taken, rest, left) = take(2);
        assert_eq!((taken, left), (vec![2, 5], 0));
        assert_eq!(rest.iter().collect::<Vec<_>>(), [63, 64, 100, 190, 255]);
        let (taken, rest, left) = take(4);
        assert_eq!((taken, left), (vec![2, 5, 63, 64], 0));
        assert_eq!(rest.iter().collect::<Vec<_>>(), [100, 190, 255]);
        // A full word split mid-way.
        let (mut rest, mut left) = (SectionMask::ALL, 70);
        let taken = rest.take_up_to(&mut left);
        assert_eq!(
            (taken, rest, left),
            (SectionMask::below(70), !SectionMask::below(70), 0)
        );
        // An empty mask spends nothing.
        let (mut rest, mut left) = (SectionMask::EMPTY, 5);
        assert_eq!(rest.take_up_to(&mut left), SectionMask::EMPTY);
        assert_eq!((rest, left), (SectionMask::EMPTY, 5));
    }
}
