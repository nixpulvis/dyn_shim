//! Keeping a consuming `-> Self` builder working on an erased value with
//! `#[dyn_shim(boxed)]`.
//!
//! `Pipeline::then` is a builder: it takes `self` by value and returns `Self`. A
//! `-> Self` cannot ride a vtable — the erased `Self` is unsized — so it would be
//! skipped from the shim, and a reflexive impl could only stub it. `boxed` makes
//! the shim method return `Box<dyn DynPipeline>` instead (the concrete result,
//! boxed and unsized to the shim object), and the `reflexive = boxed` impl then
//! satisfies the source `-> Self`, since there `Self` *is* `Box<dyn DynPipeline>`.
//! So the builder chains on an erased value rather than panicking.
//!
//! This is the general form of the boxing the recognized `Clone` bound applies to
//! `clone`. It needs `reflexive = boxed`: a bare `dyn` is unsized and cannot be a
//! returned `Self`.
//!
//! Run with: `cargo run --example builder`

use dyn_shim::dyn_shim;

#[dyn_shim(DynPipeline, reflexive = boxed)]
trait Pipeline {
    fn label(&self) -> String;

    // Consuming builder: `self` by value and `-> Self`. `boxed` forwards it by
    // boxing the result into a fresh `Box<dyn DynPipeline>`.
    #[dyn_shim(boxed)]
    fn then(self, step: &str) -> Self;
}

struct Steps(Vec<String>);
impl Pipeline for Steps {
    fn label(&self) -> String {
        self.0.join(" -> ")
    }
    fn then(mut self, step: &str) -> Self {
        self.0.push(step.to_string());
        self
    }
}

// Generic over `Pipeline` by value. A `Box<dyn DynPipeline>` satisfies the bound
// through the boxed reflexive impl, and each `then` returns another owned object,
// so the builder chains without naming the concrete type.
fn build(pipeline: impl Pipeline) -> String {
    pipeline.then("validate").then("emit").label()
}

fn main() {
    let erased: Box<dyn DynPipeline> = Box::new(Steps(vec!["parse".to_string()]));
    // The erased pipeline flows into `Pipeline`-generic code and chains there.
    println!("{}", build(erased));

    // Called directly on a shim object, the builder is qualified to the shim
    // (a shim object is both a `DynPipeline` and, reflexively, a `Pipeline`).
    let erased: Box<dyn DynPipeline> = Box::new(Steps(vec!["parse".to_string()]));
    let chained = DynPipeline::then(erased, "compress");
    println!("{}", DynPipeline::label(&*chained));
}
