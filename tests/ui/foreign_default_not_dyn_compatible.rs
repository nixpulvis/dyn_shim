use dyn_shim::dyn_shim_foreign;

mod upstream {
    pub trait Sink {
        fn total(&self) -> usize;
    }
}

// A `#[dyn_shim_foreign]` method with a default body is shim-local: the shim is
// its only home. A generic one is not dyn-compatible and so cannot be a method
// of the shim, where it would otherwise vanish silently. That is an error.
#[dyn_shim_foreign(upstream::Sink)]
trait DynSink {
    fn total(&self) -> usize;
    fn scaled<T: From<usize>>(&self) -> T {
        T::from(self.total())
    }
}

fn main() {}
