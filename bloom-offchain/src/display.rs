use std::fmt::{Display, Formatter, Pointer, Write};

pub fn display_option<T>(opt: Option<T>) -> DisplayOption<T> {
    DisplayOption(opt)
}

pub struct DisplayOption<T>(Option<T>);

impl<T: Display> Display for DisplayOption<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => f.write_str("null"),
            Some(val) => val.fmt(f),
        }
    }
}

pub fn display_tuple<A, B>(pair: (A, B)) -> DisplayTuple<A, B> {
    DisplayTuple(pair)
}

pub struct DisplayTuple<A, B>((A, B));

impl<A: Display, B: Display> Display for DisplayTuple<A, B> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?;
        self.0.0.fmt(f)?;
        f.write_str(", ")?;
        self.0.1.fmt(f)?;
        f.write_str(")")
    }
} 
