use std::collections::HashSet;
use std::hash::Hash;
use circular_buffer::CircularBuffer;

pub struct CircularFilter<const N: usize, T> {
    buffer: CircularBuffer<N, T>,
    filter: HashSet<T>,
}

impl<const N: usize, T> CircularFilter<N, T> {
    pub fn new() -> Self {
        Self {
            buffer: CircularBuffer::new(),
            filter: HashSet::new(),
        }
    }
}

impl<const N: usize, T: Hash + Eq + Copy> CircularFilter<N, T> {
    pub fn add(&mut self, a: T) {
        if self.filter.insert(a) {
            if let Err(a) = self.buffer.try_push_back(a) {
                if let Some(element_to_evict) = self.buffer.front() {
                    self.filter.remove(element_to_evict);
                    self.buffer.push_back(a);
                }
            }
        }
    }

    pub fn remove(&mut self, a: &T) -> bool {
        self.filter.remove(a)
    }

    #[cfg(test)]
    fn contains(&self, a: &T) -> bool {
        self.filter.contains(a)
    }
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::circular_filter::CircularFilter;

    #[test]
    fn add_check_rotate() {
        let mut f = CircularFilter::<3, usize>::new();
        f.add(1);
        assert!(f.contains(&1));
        f.add(2);
        assert!(f.contains(&2));
        f.add(3);
        assert!(f.contains(&3));
        f.add(4);
        assert!(f.contains(&4));
        assert!(!f.contains(&1));
        assert!(f.contains(&2));
    }
}