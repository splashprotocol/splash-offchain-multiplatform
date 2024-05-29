use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

pub struct FocusSet<T> {
    queue: VecDeque<T>,
    filter: HashSet<T>,
}

impl<T> FocusSet<T> {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            filter: HashSet::new(),
        }
    }
}

impl<T: Hash + Eq + Copy> FocusSet<T> {
    pub fn push_back(&mut self, a: T) {
        if self.filter.insert(a) {
            self.queue.push_back(a);
        }
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if let Some(a) = self.queue.pop_front() {
            self.filter.remove(&a);
            return Some(a);
        }
        None
    }
}
