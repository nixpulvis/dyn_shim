use dyn_shim::dyn_shim;

// `#[dyn_shim(boxed)]` boxes a `-> Self` builder into the marker-free `Box<dyn
// Shim>`, which cannot carry an auto-trait marker. Combined with a reflexive
// shim that lists one (`Send` here), the boxed return could not satisfy the
// `+ Send` object form, so the macro rejects the combination directly.
#[dyn_shim(DynStep: Send, reflexive = boxed)]
trait Step {
    fn value(&self) -> i32;
    #[dyn_shim(boxed)]
    fn add(self, n: i32) -> Self;
}

fn main() {}
