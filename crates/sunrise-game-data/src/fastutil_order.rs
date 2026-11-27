//! Iteration order of the Java hash collections vanilla's scoreboard keeps
//! its names in: fastutil's `Object2ObjectOpenHashMap<String, _>` (open
//! addressing with linear probing, backward-shift deletion and rehashing at
//! the load factor) and `java.util.HashSet<String>` (chained buckets split on
//! resize). `/team list` walks team names in fastutil iterator order;
//! `scoreboard.dat` saves them in fastutil's forward stream order and saves
//! team members in Java HashSet order. `/team list <team>` sorts the members.

use sunrise_core::{java_hash_set::JavaHashSet, random::java_string_hash};

/// The keys of a fastutil open hash map in their table slots.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenOrder {
    keys: Vec<Option<String>>,
    size: usize,
    min_n: usize,
    load: f32,
}
impl Default for OpenOrder {
    /// `new Object2ObjectOpenHashMap<>()`: 16 expected entries, load 0.75.
    fn default() -> Self {
        Self::new(16, 0.75)
    }
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}
/// `HashCommon.arraySize`.
fn array_size(expected: usize, f: f32) -> usize {
    let wanted = (expected as f64 / f64::from(f)).ceil() as usize;
    wanted.max(2).next_power_of_two()
}
/// `HashCommon.maxFill`.
fn max_fill(n: usize, f: f32) -> usize {
    ((n as f64 * f64::from(f)).ceil() as usize).min(n - 1)
}
impl OpenOrder {
    pub fn new(expected: usize, load: f32) -> Self {
        let n = array_size(expected, load);
        Self {
            keys: vec![None; n],
            size: 0,
            min_n: n,
            load,
        }
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    fn slot(&self, key: &str) -> usize {
        mix(java_string_hash(key.encode_utf16())) & (self.keys.len() - 1)
    }
    pub fn contains(&self, key: &str) -> bool {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        while let Some(k) = &self.keys[pos] {
            if k == key {
                return true;
            }
            pos = (pos + 1) & mask;
        }
        false
    }
    /// `put`: false when the key was already there.
    pub fn insert(&mut self, key: &str) -> bool {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        while let Some(k) = &self.keys[pos] {
            if k == key {
                return false;
            }
            pos = (pos + 1) & mask;
        }
        self.keys[pos] = Some(key.to_owned());
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`.
        let old = self.size;
        self.size += 1;
        if old >= max_fill(self.keys.len(), self.load) {
            self.rehash(array_size(self.size + 1, self.load));
        }
        true
    }
    /// `remove`: false when the key was not there.
    pub fn remove(&mut self, key: &str) -> bool {
        let mask = self.keys.len() - 1;
        let mut pos = self.slot(key);
        loop {
            match &self.keys[pos] {
                None => return false,
                Some(k) if k == key => break,
                Some(_) => pos = (pos + 1) & mask,
            }
        }
        self.size -= 1;
        self.shift_keys(pos);
        let n = self.keys.len();
        if n > self.min_n && self.size < max_fill(n, self.load) / 4 && n > 16 {
            self.rehash(n / 2);
        }
        true
    }
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.keys.len() - 1;
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            loop {
                let Some(curr) = self.keys[pos].clone() else {
                    self.keys[last] = None;
                    return;
                };
                let slot = self.slot(&curr);
                let stop = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stop {
                    self.keys[last] = Some(curr);
                    break;
                }
                pos = (pos + 1) & mask;
            }
        }
    }
    fn rehash(&mut self, new_n: usize) {
        let old = std::mem::replace(&mut self.keys, vec![None; new_n]);
        let mask = new_n - 1;
        // `for (int j = realSize(); j-- != 0;) { while (key[--i] == null); ... }`
        for key in old.into_iter().rev().flatten() {
            let mut pos = mix(java_string_hash(key.encode_utf16())) & mask;
            while self.keys[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = Some(key);
        }
    }
    /// `iterator()` and `forEach`: from the last slot down.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().rev().flatten().map(String::as_str)
    }
    /// `spliterator()` (what `stream()` uses): from the first slot up.
    pub fn stream(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().flatten().map(String::as_str)
    }
}

/// Score holder names in `java.util.HashSet<String>` traversal order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HashSetOrder(JavaHashSet<String>);

impl HashSetOrder {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
    }
    pub fn insert(&mut self, key: &str) -> bool {
        self.0.insert(key.to_owned())
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.0.remove(key)
    }
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_names_follow_java_hashset_tree_order() {
        let keys: Vec<String> = (0..16)
            .map(|i| {
                (0..4)
                    .map(|b| if i & (1 << b) == 0 { "Aa" } else { "BB" })
                    .collect()
            })
            .collect();
        let mut members = HashSetOrder::default();
        for k in &keys {
            assert!(members.insert(k));
        }
        // Temurin 25.0.5, HashSet populated by adding keys 0 through 15.
        let order = [1, 0, 2, 3, 13, 4, 12, 5, 6, 14, 7, 15, 11, 8, 9, 10];
        assert!(members.iter().eq(order.iter().map(|&i| &keys[i])));
        assert!(members.contains(&keys[11]));
        assert!(members.remove(&keys[11]));
        assert!(!members.contains(&keys[11]));
        assert!(!members.remove(&keys[11]));
    }

    #[test]
    fn hash_and_order() {
        // String.hashCode over UTF-16 code units: the emoji is two surrogates.
        assert_eq!(java_string_hash("\u{1F600}".encode_utf16()), 1_772_899);
        let mut o = OpenOrder::new(16, 0.5);
        for k in ["harness", "harness_h"] {
            o.insert(k);
        }
        assert_eq!(o.iter().count(), 2);
        let mut asc: Vec<&str> = o.stream().collect();
        asc.reverse();
        assert_eq!(asc, o.iter().collect::<Vec<_>>());
        o.remove("harness");
        assert_eq!(o.iter().collect::<Vec<_>>(), vec!["harness_h"]);
        for i in 0..40 {
            o.insert(&format!("k{i}"));
        }
        assert_eq!(o.iter().count(), 41);
        for i in 0..40 {
            o.remove(&format!("k{i}"));
        }
        assert_eq!(o.iter().collect::<Vec<_>>(), vec!["harness_h"]);
    }

    #[test]
    fn hash_set_buckets() {
        let mut s = HashSetOrder::default();
        for i in 0..30 {
            assert!(s.insert(&format!("p{i}")));
        }
        assert!(!s.insert("p3"));
        assert_eq!(s.len(), 30);
        assert!(s.remove("p3") && !s.contains("p3"));
        // "Aa" and "BB" share a hash: insertion order within the bucket.
        let mut t = HashSetOrder::default();
        t.insert("BB");
        t.insert("Aa");
        assert_eq!(t.iter().collect::<Vec<_>>(), vec!["BB", "Aa"]);
    }
}
