use dyn_shim::trait_object;

// `trait_object` names the carrier trait, not the capability. A bare
// `Clone`/`Hash` names a capability, so it is rejected with a pointer to the
// carrier trait to name and inherit instead.
#[trait_object(Clone)]
trait Foo: DynClone {
    fn id(&self) -> u32;
}

fn main() {}
