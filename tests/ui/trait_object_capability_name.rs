use dyn_shim::trait_object;

// The carrier surface names the carrier trait, not the capability. A bare
// `Clone`/`Hash` is the old spelling; it is rejected with a pointer to the
// carrier trait to name and inherit instead.
#[trait_object(Clone)]
trait Foo: DynClone {
    fn id(&self) -> u32;
}

fn main() {}
