//! Binding the `Clone` and `Hash` carriers onto the objects of an existing
//! dyn-compatible trait with `#[dyn_shim_bind]`.
//!
//! `Widget` below is already dyn-compatible: it has only `&self` methods, so
//! `dyn Widget` works on its own. What it lacks is `Hash` and `Clone` on the
//! trait object. `#[dyn_shim]` would answer by generating a separate `DynWidget`
//! shim, but here `dyn Widget` is the type used directly, so generating a second
//! trait is not wanted.
//!
//! `#[dyn_shim_bind(DynHash + DynClone)]` binds those carriers in place instead.
//! The trait inherits `DynHash` and `DynClone` as supertraits, and the attribute
//! invokes their bind macros to make `dyn Widget` implement `Hash` and `Box<dyn
//! Widget>` implement `Clone`. Because the carriers are supertraits, every
//! `Widget` implementor must be `Hash` and `Clone`.
//!
//! See `bind_trait.rs` for naming a `#[dyn_shim]` shim as the carrier, which
//! gives `dyn Widget` an arbitrary non-dyn-compatible trait the same way.
//!
//! Run with: `cargo run --example dyn_shim_bind --features "dyn_hash dyn_clone"`

use dyn_shim::{DynClone, DynHash, dyn_shim_bind};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

#[dyn_shim_bind(DynHash + DynClone)]
trait Widget: DynHash + DynClone {
    fn render(&self) -> String;
}

#[derive(Hash, Clone)]
struct Button {
    label: String,
}
impl Widget for Button {
    fn render(&self) -> String {
        format!("[{}]", self.label)
    }
}

#[derive(Hash, Clone)]
struct Spacer(u32);
impl Widget for Spacer {
    fn render(&self) -> String {
        " ".repeat(self.0 as usize)
    }
}

fn fingerprint<T: std::hash::Hash + ?Sized>(value: &T) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

fn main() {
    let toolbar: Vec<Box<dyn Widget>> = vec![
        Box::new(Button { label: "ok".into() }),
        Box::new(Spacer(3)),
        Box::new(Button {
            label: "cancel".into(),
        }),
    ];

    // Box<dyn Widget> is Clone, so the whole layout duplicates.
    let mut copy = toolbar.clone();
    copy.push(Box::new(Spacer(1)));
    println!("original widgets: {}", toolbar.len());
    println!("copy widgets:     {}", copy.len());

    // dyn Widget is Hash, so a borrowed trait object (&dyn Widget) can be
    // hashed directly. Hashing it matches hashing the concrete value.
    for w in &toolbar {
        println!("{:016x} {}", fingerprint(&**w), w.render());
    }
    assert_eq!(
        fingerprint(&**toolbar.first().unwrap()),
        fingerprint(&Button { label: "ok".into() }),
    );
}
