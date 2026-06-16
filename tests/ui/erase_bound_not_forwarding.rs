use dyn_shim::dyn_shim;

// `#[dyn_shim(erase)]` lowers `&mut impl Bound` to `&mut dyn Bound` and forwards
// by reborrow, so the source method's parameter re-infers to the sized reference
// type `&mut dyn Bound`. That type-checks only when the bound forwards through
// references (an `impl Bound for &mut (dyn Bound)`, as std provides for `Write`,
// `Hasher`, and friends). A bound without such an impl is accepted by the macro
// but fails to compile on the generated forwarding call, the same way a foreign
// signature mismatch surfaces.
trait Sink {
    fn put(&mut self, byte: u8);
}

#[dyn_shim(Dyn)]
trait Src {
    #[dyn_shim(erase)]
    fn drain(&self, out: &mut impl Sink);
}

fn main() {}
