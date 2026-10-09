//! Sac (inventory grid) and item stacks.

use crate::content::ItemKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    pub kind: ItemKind,
    pub count: u32,
}

impl Stack {
    pub fn new(kind: ItemKind, count: u32) -> Stack {
        Stack { kind, count }
    }
}

/// Fixed slot grid; `slots.len()` is the capacity. Empty slots are `None`; a stack
/// never holds 0 items or more than its kind's stack size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sac {
    pub slots: Vec<Option<Stack>>,
}

impl Sac {
    pub fn new(capacity: usize) -> Sac {
        Sac {
            slots: vec![None; capacity],
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Grows to `capacity` slots; never shrinks.
    pub fn upgrade(&mut self, capacity: usize) {
        if capacity > self.slots.len() {
            self.slots.resize(capacity, None);
        }
    }

    pub fn get(&self, slot: usize) -> Option<Stack> {
        self.slots.get(slot).copied().flatten()
    }

    pub fn count(&self, kind: ItemKind) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|s| s.kind == kind)
            .map(|s| s.count)
            .sum()
    }

    /// How many more of `kind` fit.
    pub fn room_for(&self, kind: ItemKind) -> u32 {
        let max = kind.stack_size();
        self.slots
            .iter()
            .map(|s| match s {
                None => max,
                Some(s) if s.kind == kind => max.saturating_sub(s.count),
                Some(_) => 0,
            })
            .sum()
    }

    pub fn first_empty(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    /// Adds up to `n`, topping up existing stacks first. Returns the leftover that did not fit.
    pub fn add(&mut self, kind: ItemKind, mut n: u32) -> u32 {
        let max = kind.stack_size();
        for s in self.slots.iter_mut().flatten() {
            if n == 0 {
                return 0;
            }
            if s.kind == kind && s.count < max {
                let put = n.min(max - s.count);
                s.count += put;
                n -= put;
            }
        }
        for slot in self.slots.iter_mut() {
            if n == 0 {
                break;
            }
            if slot.is_none() {
                let put = n.min(max);
                *slot = Some(Stack::new(kind, put));
                n -= put;
            }
        }
        n
    }

    /// Adds all `n` or nothing.
    pub fn add_all(&mut self, kind: ItemKind, n: u32) -> bool {
        if self.room_for(kind) < n {
            return false;
        }
        self.add(kind, n);
        true
    }

    /// Removes `n` of `kind` (last slots first) or nothing.
    pub fn remove(&mut self, kind: ItemKind, mut n: u32) -> bool {
        if self.count(kind) < n {
            return false;
        }
        for slot in self.slots.iter_mut().rev() {
            if n == 0 {
                break;
            }
            if let Some(s) = slot {
                if s.kind == kind {
                    let take = n.min(s.count);
                    s.count -= take;
                    n -= take;
                    if s.count == 0 {
                        *slot = None;
                    }
                }
            }
        }
        true
    }

    /// Empties one slot and returns its stack.
    pub fn take(&mut self, slot: usize) -> Option<Stack> {
        self.slots.get_mut(slot).and_then(Option::take)
    }

    /// Removes `n` items from one slot (all or nothing).
    pub fn take_from(&mut self, slot: usize, n: u32) -> Option<Stack> {
        let s = self.slots.get_mut(slot)?;
        let stack = (*s)?;
        if stack.count < n || n == 0 {
            return None;
        }
        if stack.count == n {
            *s = None;
        } else {
            *s = Some(Stack::new(stack.kind, stack.count - n));
        }
        Some(Stack::new(stack.kind, n))
    }

    /// Empties every slot whose kind passes `filter` and returns those stacks in slot order.
    pub fn take_all(&mut self, filter: impl Fn(ItemKind) -> bool) -> Vec<Stack> {
        let mut out = Vec::new();
        for slot in self.slots.iter_mut() {
            if slot.is_some_and(|s| filter(s.kind)) {
                out.extend(slot.take());
            }
        }
        out
    }

    /// Moves slot `from` onto slot `to`: merges equal kinds up to the stack size (the
    /// rest stays in `from`), otherwise swaps. False if a slot index is out of range.
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.slots.len() || to >= self.slots.len() {
            return false;
        }
        if from == to {
            return true;
        }
        let mut src = self.slots[from];
        exchange(&mut src, &mut self.slots[to]);
        self.slots[from] = src;
        true
    }

    /// Exchanges slot `slot` with an outside stack such as a hotbar hand: equal kinds merge
    /// into `hand`, otherwise they swap. False if `slot` is out of range.
    pub fn exchange_with(&mut self, slot: usize, hand: &mut Option<Stack>) -> bool {
        match self.slots.get_mut(slot) {
            Some(s) => {
                exchange(s, hand);
                true
            }
            None => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// Every stack with its slot index.
    pub fn stacks(&self) -> impl Iterator<Item = (usize, Stack)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|s| (i, s)))
    }
}

/// Moves `src` onto `dst`: equal kinds merge into `dst` up to the stack size (the rest
/// stays in `src`), otherwise the two swap.
pub fn exchange(src: &mut Option<Stack>, dst: &mut Option<Stack>) {
    match (src.as_mut(), dst.as_mut()) {
        (Some(a), Some(b)) if a.kind == b.kind => {
            let put = a.count.min(a.kind.stack_size().saturating_sub(b.count));
            b.count += put;
            a.count -= put;
            if a.count == 0 {
                *src = None;
            }
        }
        _ => std::mem::swap(src, dst),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{Fish, ItemKind::*};

    #[test]
    fn add_stacks_then_fills_empty_slots() {
        let mut sac = Sac::new(3);
        assert_eq!(sac.add(Plank, 4), 0);
        assert_eq!(sac.add(Plank, 8), 0);
        assert_eq!(sac.get(0), Some(Stack::new(Plank, 10)));
        assert_eq!(sac.get(1), Some(Stack::new(Plank, 2)));
        assert_eq!(sac.count(Plank), 12);
        assert_eq!(sac.add(Bait, 25), 5, "one slot left holds 20 bait");
        assert_eq!(sac.room_for(Plank), 8);
        assert_eq!(sac.room_for(ScrapChunk), 0);
        assert!(!sac.add_all(Plank, 9));
        assert_eq!(sac.count(Plank), 12, "add_all is all-or-nothing");
        assert!(sac.add_all(Plank, 8));
        assert_eq!(sac.first_empty(), None);
    }

    #[test]
    fn remove_and_take() {
        let mut sac = Sac::new(4);
        sac.add(ScrapChunk, 15);
        sac.add(Fish(Fish::Sardine), 3);
        assert!(!sac.remove(ScrapChunk, 16));
        assert_eq!(sac.count(ScrapChunk), 15);
        assert!(sac.remove(ScrapChunk, 6));
        assert_eq!(sac.count(ScrapChunk), 9);
        assert_eq!(sac.get(1), None, "the partial last stack is used first");
        assert_eq!(
            sac.take_from(2, 1),
            Some(Stack::new(Fish(Fish::Sardine), 1))
        );
        assert_eq!(sac.take_from(2, 5), None);
        let debris = sac.take_all(|k| k.is_debris());
        assert_eq!(debris, vec![Stack::new(ScrapChunk, 9)]);
        assert_eq!(sac.count(Fish(Fish::Sardine)), 2);
        assert_eq!(sac.take(2), Some(Stack::new(Fish(Fish::Sardine), 2)));
        assert!(sac.is_empty());
        assert_eq!(sac.take(99), None);
    }

    #[test]
    fn move_merges_or_swaps() {
        let mut sac = Sac::new(3);
        sac.slots[0] = Some(Stack::new(Plank, 7));
        sac.slots[1] = Some(Stack::new(Plank, 6));
        sac.slots[2] = Some(Stack::new(Bait, 1));
        assert!(sac.move_item(0, 1));
        assert_eq!(sac.get(1), Some(Stack::new(Plank, 10)));
        assert_eq!(sac.get(0), Some(Stack::new(Plank, 3)));
        assert!(sac.move_item(0, 2));
        assert_eq!(sac.get(0), Some(Stack::new(Bait, 1)));
        assert_eq!(sac.get(2), Some(Stack::new(Plank, 3)));
        assert!(!sac.move_item(0, 3));
    }

    #[test]
    fn hand_exchange_and_upgrade() {
        let mut sac = Sac::new(2);
        sac.add(Bandage, 4);
        let mut hand = Some(Stack::new(Bandage, 8));
        assert!(sac.exchange_with(0, &mut hand));
        assert_eq!(hand, Some(Stack::new(Bandage, 10)));
        assert_eq!(sac.get(0), Some(Stack::new(Bandage, 2)));
        let mut empty = None;
        sac.exchange_with(0, &mut empty);
        assert_eq!(empty, Some(Stack::new(Bandage, 2)));
        assert_eq!(sac.get(0), None);
        sac.upgrade(8);
        assert_eq!(sac.capacity(), 8);
        sac.upgrade(4);
        assert_eq!(sac.capacity(), 8);
    }
}
