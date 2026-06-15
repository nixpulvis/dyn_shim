use dyn_shim::trait_object;

// `#[trait_object(DynHash)]` mounts the `DynHash` carrier, so the trait must
// inherit it as a supertrait for `dyn Foo: DynHash` to hold. Without it, the
// attribute rejects the trait up front rather than emitting a mount whose
// forwarding body fails to compile.
#[trait_object(DynHash)]
trait Foo {
    fn id(&self) -> u32;
}

fn main() {}
