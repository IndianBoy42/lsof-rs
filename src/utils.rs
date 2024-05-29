pub trait StrLeakExt {
    fn leak_str(self) -> &'static str;
}
impl<T: Into<Box<str>>> StrLeakExt for T {
    fn leak_str(self) -> &'static str {
        std::boxed::Box::<str>::leak(self.into())
    }
}
pub fn leak_str(s: impl StrLeakExt) -> &'static str {
    StrLeakExt::leak_str(s)
}

#[macro_export]
macro_rules! bfmt {
    ($($args:tt)*) => {{
        Box::leak(format!($($args)*).into_boxed_str()) as &'static str
    }};
}

use std::io::BufWriter;

use fxhash::{FxHashMap, FxHashSet};
pub type FSet<T> = FxHashSet<T>;
pub type FMap<K, V> = FxHashMap<K, V>;
#[must_use]
pub fn fmap<K, V>(cap: usize) -> FMap<K, V> {
    FMap::with_capacity_and_hasher(cap, std::hash::BuildHasherDefault::default())
}
#[must_use]
pub fn fset<V>(cap: usize) -> FSet<V> {
    FSet::with_capacity_and_hasher(cap, std::hash::BuildHasherDefault::default())
}

pub fn buf_stdout<'a>(all: impl ExactSizeIterator) -> BufWriter<std::io::StdoutLock<'a>> {
    BufWriter::with_capacity((all.len() * 80 / 8).min(8192), std::io::stdout().lock())
}
