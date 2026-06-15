use dyn_shim::trait_object;

// `trait_object` mounts carrier traits. An auto trait alone selects marker
// combinations but names no carrier to mount, so the attribute rejects it.
#[trait_object(Send)]
trait Foo {
    fn id(&self) -> u32;
}

fn main() {}
