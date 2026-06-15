use dyn_shim::dyn_shim;

// The supported helper arguments on a method are `skip`, `panic`, and `erase`;
// anything else (here a typo) is an error instead of being silently ignored.
#[dyn_shim(Dyn)]
trait Src {
    #[dyn_shim(skpi)]
    fn keep(&self) -> i32;
}

fn main() {}
