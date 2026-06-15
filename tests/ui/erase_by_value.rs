use dyn_shim::dyn_shim;

// `#[dyn_shim(erase)]` lowers a generic parameter only where it is used behind a
// reference, since only `&dyn Bound` / `&mut dyn Bound` enters a vtable. A
// by-value generic argument has no reference to lower, so the macro rejects it
// directly rather than emitting code that fails to compile.
#[dyn_shim(Dyn)]
trait Src {
    #[dyn_shim(erase)]
    fn sink<T: std::fmt::Debug>(&self, value: T);
}

fn main() {}
