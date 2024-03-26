use derive_more::Into;

use spectrum_cardano_lib::OutputRef;

#[derive(Debug, Copy, Clone, Into)]
pub struct ConsumedInputs([Option<OutputRef>; 16]);

const SIZE: usize = 16;

impl ConsumedInputs {
    pub fn new<I: Iterator<Item = OutputRef>>(refs: I) -> Self {
        let mut bf = [None; SIZE];
        for (i, x) in refs.enumerate() {
            if i >= SIZE {
                break;
            }
            bf[i] = Some(x);
        }
        Self(bf)
    }

    pub fn find<F>(&self, f: F) -> bool
    where
        F: Fn(&OutputRef) -> bool,
    {
        self.0
            .iter()
            .find(move |opt| match opt {
                Some(oref) => f(oref),
                None => false,
            })
            .is_some()
    }
}
