use dyn_shim::dyn_shim_bind;

// `dyn_shim_bind` binds carrier traits. An auto trait alone selects marker
// combinations but names no carrier to bind, so the attribute rejects it.
#[dyn_shim_bind(Send)]
trait Foo {
    fn id(&self) -> u32;
}

fn main() {}
