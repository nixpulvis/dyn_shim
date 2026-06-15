use dyn_shim::dyn_shim_bind;

// `dyn_shim_bind` names the carrier trait, not the capability. A bare
// `Clone`/`Hash` names a capability, so it is rejected with a pointer to the
// carrier trait to name and inherit instead.
#[dyn_shim_bind(Clone)]
trait Foo: DynClone {
    fn id(&self) -> u32;
}

fn main() {}
