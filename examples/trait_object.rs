//! Adding `Hash` and `Clone` to the objects of an existing dyn-compatible
//! trait with `#[trait_object]`.
//!
//! `Widget` below is already dyn-compatible: it has only `&self` methods, so
//! `dyn Widget` works on its own. What it lacks is `Hash` and `Clone` on the
//! trait object. `#[dyn_shim]` would answer by generating a separate `DynWidget`
//! shim, but here `dyn Widget` is the type used directly, so generating a second
//! trait is not wanted.
//!
//! `#[trait_object(DynHash + DynClone)]` adds the impls in place instead. The trait
//! carries `DynHash` and `DynClone` as supertraits to hold the machinery, and
//! the attribute makes `dyn Widget` implement `Hash` and `Box<dyn Widget>`
//! implement `Clone`. Because the carriers are supertraits, every `Widget`
//! implementor must be `Hash` and `Clone`.
//!
//! Run with: `cargo run --example trait_object --features "dyn_hash dyn_clone"`

use dyn_shim::{DynClone, DynHash, trait_object};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

#[trait_object(DynHash + DynClone)]
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
