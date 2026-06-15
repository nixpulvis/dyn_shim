//! Cloneable and hashable boxed trait objects, with the recognized `Clone` and
//! `Hash` bounds.
//!
//! A `Shape` trait wants cloneable, hashable implementors, but neither `Clone`
//! nor `Hash` can be a supertrait of a dyn-compatible trait (`clone` returns
//! `Self`, `hash` is generic over the hasher). Listing them in the shim's bounds
//! is recognized and handled specially instead of passed through: the generated
//! `DynShape` shim carries hidden machinery, and only `Clone + Hash` implementors
//! of `Shape` receive the shim.
//!
//! The machinery lands on the shim's trait objects:
//!
//! - `Clone` is implemented for `Box<dyn DynShape>`, so a whole `Vec` of them
//!   clones.
//! - `ToOwned` is implemented for `dyn DynShape`, so a borrowed `&dyn DynShape`
//!   can be promoted to an owned box without copying the reference by mistake.
//! - `Hash` is implemented for `dyn DynShape`, which also covers `&dyn DynShape`
//!   and, through the standard library's forwarding impl, `Box<dyn DynShape>`.
//!
//! These recognized bounds are self-contained and need no crate feature. The
//! `dyn_clone` / `dyn_hash` features only add an upcast into the standalone
//! `DynClone` / `DynHash` carriers (see the `bind` example).
//!
//! Run with: `cargo run --example clone_and_hash`

use dyn_shim::dyn_shim;
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hash};

#[dyn_shim(DynShape: Clone + Hash)]
trait Shape {
    fn name(&self) -> &'static str;
    fn area(&self) -> u32;
    fn scale(&mut self, factor: u32);
}

#[derive(Clone, Hash)]
struct Rect {
    w: u32,
    h: u32,
}
impl Shape for Rect {
    fn name(&self) -> &'static str {
        "rect"
    }
    fn area(&self) -> u32 {
        self.w * self.h
    }
    fn scale(&mut self, factor: u32) {
        self.w *= factor;
        self.h *= factor;
    }
}

#[derive(Clone, Hash)]
struct Square(u32);
impl Shape for Square {
    fn name(&self) -> &'static str {
        "square"
    }
    fn area(&self) -> u32 {
        self.0 * self.0
    }
    fn scale(&mut self, factor: u32) {
        self.0 *= factor;
    }
}

fn fingerprint<T: Hash + ?Sized>(value: &T) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

fn main() {
    let shapes: Vec<Box<dyn DynShape>> = vec![Box::new(Rect { w: 2, h: 3 }), Box::new(Square(4))];

    // Box<dyn DynShape> is Clone, so the whole Vec clones. Scaling the copies
    // leaves the originals untouched.
    let mut grown = shapes.clone();
    for shape in grown.iter_mut() {
        shape.scale(2);
    }
    for (original, copy) in shapes.iter().zip(&grown) {
        println!(
            "{}: area {}, scaled clone area {}",
            original.name(),
            original.area(),
            copy.area()
        );
    }

    // dyn DynShape is Hash, so a borrowed trait object hashes like the concrete
    // value behind it.
    let first = shapes.first().unwrap();
    println!("\n{:016x} {}", fingerprint(&**first), first.name());
    assert_eq!(fingerprint(&**first), fingerprint(&Rect { w: 2, h: 3 }));

    // dyn DynShape is ToOwned, so a bare borrow can escape into an owned box.
    // (Calling .clone() on the &dyn would clone the reference, not the value.)
    let borrowed: &dyn DynShape = &Square(5);
    let owned: Box<dyn DynShape> = borrowed.to_owned();
    println!("owned from borrow: {} area {}", owned.name(), owned.area());
}
