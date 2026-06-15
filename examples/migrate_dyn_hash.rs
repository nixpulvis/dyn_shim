//! Migrating the `dyn-hash` README example to `#[trait_object]`.
//!
//! `dyn-hash` makes a `Box<dyn MyTrait>` hashable in two pieces: a `DynHash`
//! supertrait that carries the erased hashing, and a `hash_trait_object!` macro
//! call that implements `Hash` for `dyn MyTrait`. The original program:
//!
//! ```ignore
//! use dyn_hash::DynHash;
//!
//! trait MyTrait: DynHash {
//!     fn recite(&self);
//! }
//!
//! // Implement std::hash::Hash for dyn MyTrait
//! dyn_hash::hash_trait_object!(MyTrait);
//!
//! // Now data structures containing Box<dyn MyTrait> can derive Hash:
//! #[derive(Hash)]
//! struct Container {
//!     trait_object: Box<dyn MyTrait>,
//! }
//! ```
//!
//! The `dyn_shim` version keeps the trait, its `DynHash` supertrait, and the
//! `dyn MyTrait` type unchanged. Two things move: the import points at
//! `dyn_shim`, and the separate `hash_trait_object!(MyTrait)` call becomes a
//! `#[trait_object(DynHash)]` attribute on the trait. `dyn MyTrait` then implements
//! `Hash`, covering `Box<dyn MyTrait>` through the standard library's forwarding
//! impl, so the `Container` derive works as before.
//!
//! Run with: `cargo run --example migrate_dyn_hash --features dyn_hash`

use dyn_shim::{DynHash, trait_object};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

#[trait_object(DynHash)]
trait MyTrait: DynHash {
    fn recite(&self);
}

impl MyTrait for String {
    fn recite(&self) {
        println!("{} reporting in", self);
    }
}

#[derive(Hash)]
struct Container {
    trait_object: Box<dyn MyTrait>,
}

fn main() {
    let line = "The slithy structs did gyre and gimble the namespace";
    let container = Container {
        trait_object: Box::new(String::from(line)),
    };
    container.trait_object.recite();

    let bh = BuildHasherDefault::<DefaultHasher>::default();
    println!("container hash: {:016x}", bh.hash_one(&container));
    assert_eq!(
        bh.hash_one(&*container.trait_object),
        bh.hash_one(&String::from(line))
    );
}
