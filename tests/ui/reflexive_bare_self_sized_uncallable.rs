use dyn_shim::dyn_shim;

// A `where Self: Sized` method is omitted from the `reflexive = bare` impl (it
// is not part of the unsized object's surface), so calling it on a `&dyn DynCfg`
// is a compile error rather than a runtime panic — the point of dropping the
// stub on the bare form.
#[dyn_shim(DynCfg, reflexive = bare)]
trait Cfg {
    fn get(&self) -> i32;
    fn rebuild(&self) -> i32
    where
        Self: Sized;
}

fn main() {
    struct S;
    impl Cfg for S {
        fn get(&self) -> i32 {
            0
        }
        fn rebuild(&self) -> i32 {
            1
        }
    }
    let d: &dyn DynCfg = &S;
    let _ = d.rebuild();
}
