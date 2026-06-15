//! Migrating the `dyn-clone` README example to `#[dyn_shim_bind]`.
//!
//! `dyn-clone` makes a `Box<dyn MyTrait>` cloneable in two pieces: a `DynClone`
//! supertrait that carries the erased clone, and either the `clone_box` free
//! function or a `clone_trait_object!` macro call. The original program:
//!
//! ```ignore
//! use dyn_clone::DynClone;
//!
//! trait MyTrait: DynClone {
//!     fn recite(&self);
//! }
//!
//! impl MyTrait for String {
//!     fn recite(&self) {
//!         println!("{} reporting in", self);
//!     }
//! }
//!
//! fn main() {
//!     let line = "The slithy structs did gyre and gimble the namespace";
//!     let x: Box<dyn MyTrait> = Box::new(String::from(line));
//!     x.recite();
//!     let x2 = dyn_clone::clone_box(&*x);
//!     x2.recite();
//! }
//! ```
//!
//! The `dyn_shim` version keeps the trait, its `DynClone` supertrait, and the
//! `dyn MyTrait` type unchanged. Two things move: the import points at
//! `dyn_shim`, and `#[dyn_shim_bind(DynClone)]` on the trait stands in for the
//! `clone_trait_object!` call. The attribute makes `Box<dyn MyTrait>` implement
//! `Clone`, so `dyn_clone::clone_box(&*x)` becomes `x.clone()`; it also gives
//! `dyn MyTrait` a `ToOwned` impl, so `(&*x).to_owned()` is the direct
//! counterpart when you hold only a borrow.
//!
//! Run with: `cargo run --example migrate_dyn_clone --features dyn_clone`

use dyn_shim::{DynClone, dyn_shim_bind};

#[dyn_shim_bind(DynClone)]
trait MyTrait: DynClone {
    fn recite(&self);
}

impl MyTrait for String {
    fn recite(&self) {
        println!("{} reporting in", self);
    }
}

fn main() {
    let line = "The slithy structs did gyre and gimble the namespace";

    let x: Box<dyn MyTrait> = Box::new(String::from(line));
    x.recite();

    // Using #[dyn_shim_bind(DynClone)] let's us call .clone here directly.
    let x2 = x.clone();
    x2.recite();
}
