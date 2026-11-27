//! Minecraft's pathfinder open set (also used by the ender dragon).
use std::borrow::BorrowMut;

#[derive(Clone, Copy, Debug)]
pub struct Key {
    cost: f32,
    slot: i32,
}

impl Default for Key {
    fn default() -> Self {
        Self {
            cost: 0.0,
            slot: -1,
        }
    }
}

impl Key {
    pub fn is_open(self) -> bool {
        self.slot >= 0
    }
}

#[derive(Default)]
pub struct OpenSet(Vec<u32>);

impl OpenSet {
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The caller resets or reuses a fresh node arena for each search.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn insert<N: BorrowMut<Key>>(&mut self, nodes: &mut [N], node: u32, cost: f32) {
        let i = self.0.len();
        nodes[node as usize].borrow_mut().cost = cost;
        self.0.push(node);
        up_heap(nodes, &mut self.0, i);
    }

    pub fn pop<N: BorrowMut<Key>>(&mut self, nodes: &mut [N]) -> Option<u32> {
        let last = self.0.pop()?;
        let root = if let Some(first) = self.0.first_mut() {
            let root = std::mem::replace(first, last);
            down_heap(nodes, &mut self.0, 0);
            root
        } else {
            last
        };
        nodes[root as usize].borrow_mut().slot = -1;
        Some(root)
    }

    pub fn change_cost<N: BorrowMut<Key>>(&mut self, nodes: &mut [N], node: u32, cost: f32) {
        let key = nodes[node as usize].borrow_mut();
        let old = std::mem::replace(&mut key.cost, cost);
        let i = key.slot as usize;
        if cost < old {
            up_heap(nodes, &mut self.0, i);
        } else {
            down_heap(nodes, &mut self.0, i);
        }
    }
}

fn up_heap<N: BorrowMut<Key>>(nodes: &mut [N], heap: &mut [u32], mut i: usize) {
    let node = heap[i];
    let cost = nodes[node as usize].borrow().cost;
    while i > 0 {
        let parent_i = (i - 1) >> 1;
        let parent = heap[parent_i];
        if cost < nodes[parent as usize].borrow().cost {
            heap[i] = parent;
            nodes[parent as usize].borrow_mut().slot = i as i32;
            i = parent_i;
        } else {
            break;
        }
    }
    heap[i] = node;
    nodes[node as usize].borrow_mut().slot = i as i32;
}

fn down_heap<N: BorrowMut<Key>>(nodes: &mut [N], heap: &mut [u32], mut i: usize) {
    let node = heap[i];
    let cost = nodes[node as usize].borrow().cost;
    loop {
        let left_i = 1 + (i << 1);
        let Some(&left) = heap.get(left_i) else { break };
        let right_i = left_i + 1;
        let left_cost = nodes[left as usize].borrow().cost;
        let right_cost = heap
            .get(right_i)
            .map_or(f32::INFINITY, |&n| nodes[n as usize].borrow().cost);
        // Equal children favor the right. If it is absent (+infinity),
        // its cost cannot be strictly less than any node's cost.
        let (child_i, child_cost) = if left_cost < right_cost {
            (left_i, left_cost)
        } else {
            (right_i, right_cost)
        };
        if child_cost < cost {
            let child = heap[child_i];
            heap[i] = child;
            nodes[child as usize].borrow_mut().slot = i as i32;
            i = child_i;
        } else {
            break;
        }
    }
    heap[i] = node;
    nodes[node as usize].borrow_mut().slot = i as i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_moves_only_on_strictly_lower_cost() {
        // Vanilla's BinaryHeap uses strict float comparisons throughout.
        let mut nodes = [Key::default(); 3];
        let mut heap = [0, 1, 2];
        nodes[1].cost = f32::NAN;
        up_heap(&mut nodes, &mut heap, 1);
        assert_eq!(heap, [0, 1, 2]);

        nodes[0].cost = f32::NAN;
        nodes[1].cost = f32::INFINITY;
        down_heap(&mut nodes, &mut heap[..2], 0);
        assert_eq!(heap, [0, 1, 2]);

        nodes[0].cost = 2.0;
        nodes[1].cost = 1.0;
        nodes[2].cost = 1.0;
        down_heap(&mut nodes, &mut heap, 0);
        assert_eq!(heap, [2, 1, 0]);
    }
}
