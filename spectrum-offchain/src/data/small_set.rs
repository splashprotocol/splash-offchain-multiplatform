use derive_more::Into;

#[derive(Debug, Copy, Clone, Into)]
pub struct SmallVec<T>([Option<T>; SIZE]);

const SIZE: usize = 32;

impl<T> SmallVec<T> {
    pub fn new<I: Iterator<Item = T>>(refs: I) -> Self
    where
        T: Copy,
    {
        let mut bf = [None; SIZE];
        for (i, x) in refs.enumerate() {
            if i >= SIZE {
                break;
            }
            bf[i] = Some(x);
        }
        Self(bf)
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|e| e.is_none())
    }

    pub fn exists<F>(&self, f: F) -> bool
    where
        F: Fn(&T) -> bool,
    {
        self.0
            .iter()
            .find(move |opt| match opt {
                Some(oref) => f(oref),
                None => false,
            })
            .is_some()
    }

    pub fn count<F>(&self, f: F) -> usize
    where
        F: Fn(&T) -> bool,
    {
        self.0.iter().fold(0, |t, x| match x {
            Some(elt) if f(elt) => t + 1,
            _ => t,
        })
    }
}

impl<T: Copy> Into<Vec<T>> for SmallVec<T> {
    fn into(self) -> Vec<T> {
        let mut acc = vec![];
        self.0.iter().for_each(|opt_elem| {
            if let Some(elem) = opt_elem {
                acc.push(*elem)
            }
        });
        acc
    }
}

impl<T: Copy> Default for SmallVec<T> {
    fn default() -> Self {
        Self([None; SIZE])
    }
}
