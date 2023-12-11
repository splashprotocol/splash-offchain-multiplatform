pub trait PartiallyFilled<Ord> {
    fn order(&self) -> &Ord;
    /// How much of input asset we take from the order.
    fn removed_input(&self) -> u64;
    /// How much of the output asset we add to the order.
    fn added_output(&self) -> u64;
    /// Should the order be terminated.
    fn has_terminated(&self) -> bool;
}
