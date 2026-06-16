use dyn_shim::dyn_shim_bind;

// `#[dyn_shim_bind(DynHash)]` binds the `DynHash` carrier, so the trait must
// inherit it as a supertrait for `dyn Foo: DynHash` to hold. Without it, the
// attribute rejects the trait up front rather than emitting a bind whose
// forwarding body fails to compile.
#[dyn_shim_bind(DynHash)]
trait Foo {
    fn id(&self) -> u32;
}

fn main() {}
