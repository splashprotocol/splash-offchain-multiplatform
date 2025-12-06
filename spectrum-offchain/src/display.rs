use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter, Write};

pub fn display_option<T>(opt: &Option<T>) -> DisplayOption<T> {
    DisplayOption(opt)
}

pub struct DisplayOption<'a, T>(&'a Option<T>);

impl<'a, T: Display> Display for DisplayOption<'a, T> {
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
        self.0 .0.fmt(f)?;
        f.write_str(", ")?;
        self.0 .1.fmt(f)?;
        f.write_str(")")
    }
}

pub struct DisplayVec<'a, T>(&'a Vec<T>);

impl<'a, T: Display> Display for DisplayVec<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for x in self.0 {
            x.fmt(f)?;
            f.write_str(", ")?;
        }
        f.write_str("]")
    }
}

pub fn display_vec<T>(vec: &Vec<T>) -> DisplayVec<T> {
    DisplayVec(vec)
}

pub struct DisplaySet<'a, T>(&'a HashSet<T>);

impl<'a, T: Display> Display for DisplaySet<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        for x in self.0 {
            x.fmt(f)?;
            f.write_str(", ")?;
        }
        f.write_str("}")
    }
}

pub fn display_set<T>(set: &HashSet<T>) -> DisplaySet<T> {
    DisplaySet(set)
}

pub struct DisplayMap<'a, T1, T2>(&'a HashMap<T1, T2>);

impl<'a, T1: Display, T2: Display> Display for DisplayMap<'a, T1, T2> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        for (k, v) in self.0 {
            k.fmt(f)?;
            f.write_str(" -> ")?;
            v.fmt(f)?;
            f.write_str(", ")?;
        }
        f.write_str("}")
    }
}

pub fn display_map<T1, T2>(map: &HashMap<T1, T2>) -> DisplayMap<T1, T2> {
    DisplayMap(map)
}
