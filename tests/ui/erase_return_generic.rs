use dyn_shim::dyn_shim;

// `#[dyn_shim(erase)]` lowers a generic parameter used behind a reference to a
// trait object. A parameter in the return type has no reference to lower and no
// single concrete type to pick, so the macro rejects it directly rather than
// emitting code that fails to compile.
#[dyn_shim(Dyn)]
trait Src {
    #[dyn_shim(erase)]
    fn make<T: Default>(&self) -> T;
}

fn main() {}
