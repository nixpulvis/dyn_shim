//! Shimming a trait you do not own with `#[dyn_shim_foreign]`.
//!
//! `#[dyn_shim]` has to sit on the trait's own definition, so it cannot target
//! a trait from a dependency. `#[dyn_shim_foreign(path)]` fills this gap by
//! annotating the trait that *is* the shim, restating the foreign methods to
//! forward, and the macro fills in the forwarding machinery plus a blanket impl
//! pointing at the foreign path just like it would for `#[dyn_shim]` on your
//! own trait.
//!
//! The `vendor` module here stands in for a third-party crate. A proc macro
//! only ever sees its own input tokens, never another crate's AST, so the
//! signatures have to be restated by hand. A mismatch is caught when the
//! generated forwarding call fails to compile.
//!
//! Run with: `cargo run --example foreign`

use dyn_shim::dyn_shim_foreign;

// Pretend this lives in a dependency. `Widget` is not dyn-compatible since it
// has a receiverless `build() -> Self` constructor and a by-value
// `consume(self)`.
mod vendor {
    pub trait Widget {
        fn build() -> Self; // receiverless: not dyn-compatible
        fn render(&self) -> String;
        fn resize(&mut self, by: i32);
        fn consume(self) -> String; // by-value self
    }
}

// The annotated trait is the shim. Restate only the dyn-compatible methods to
// forward: `build` is omitted, and the by-value `consume` is forwarded through
// `self: Box<Self>`. A `Clone` supertrait makes the boxed shim objects
// cloneable, and `Send` lets them cross threads. The foreign trait's path is
// the attribute's argument.
//
// Note how we still write `fn consume(self) -> String`, the same rewriting
// happens here that converts this to `fn consume(sefl: Box<Self>) -> String`
// as for #[dyn_shim].
#[dyn_shim_foreign(vendor::Widget)]
trait DynWidget: Clone + Send {
    fn render(&self) -> String;
    fn resize(&mut self, by: i32);
    fn consume(self) -> String;
}

#[derive(Clone)]
struct Button {
    label: String,
    width: i32,
}

impl vendor::Widget for Button {
    fn build() -> Self {
        Button {
            label: "OK".into(),
            width: 4,
        }
    }
    fn render(&self) -> String {
        format!("[{}]({})", self.label, self.width)
    }
    fn resize(&mut self, by: i32) {
        self.width += by;
    }
    fn consume(self) -> String {
        self.label
    }
}

#[derive(Clone)]
struct Slider {
    value: i32,
}

impl vendor::Widget for Slider {
    fn build() -> Self {
        Slider { value: 0 }
    }
    fn render(&self) -> String {
        format!("<{}>", self.value)
    }
    fn resize(&mut self, by: i32) {
        self.value += by;
    }
    fn consume(self) -> String {
        format!("slider={}", self.value)
    }
}

fn main() {
    // A mixed set of foreign-trait implementors behind one boxed shim.
    let mut widgets: Vec<Box<dyn DynWidget>> = vec![
        Box::new(<Button as vendor::Widget>::build()),
        Box::new(<Slider as vendor::Widget>::build()),
    ];

    for w in widgets.iter_mut() {
        w.resize(2);
    }

    // The Clone bound gives the boxes a Clone impl.
    let snapshot: Vec<Box<dyn DynWidget>> = widgets.clone();

    println!("rendered:");
    for w in &widgets {
        println!("  {}", w.render());
    }

    // The boxes are Send, so they can move to another thread.
    let consumed = std::thread::spawn(move || {
        widgets
            .into_iter()
            .map(|w| w.consume()) // by-value self through Box<Self>
            .collect::<Vec<_>>()
    })
    .join()
    .unwrap();
    println!("consumed: {consumed:?}");

    // The clone is independent of the originals consumed above.
    println!("snapshot still rendered:");
    for w in &snapshot {
        println!("  {}", w.render());
    }
}
