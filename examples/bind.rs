//! Binding capabilities onto a trait that is *already* dyn-compatible, with
//! `#[dyn_shim_bind]`.
//!
//! `Product` below is already dyn-compatible: it has only `&self` methods, so
//! `dyn Product` works on its own. What it lacks is `Clone` and `Hash` on the
//! boxed object, and the ability to satisfy `Priced` (which is not
//! dyn-compatible). `#[dyn_shim]` would answer by generating a separate shim
//! trait, but here `dyn Product` is the type used directly, so a second trait is
//! not wanted.
//!
//! `#[dyn_shim_bind(..)]` generates no new trait. It re-emits `Product` untouched
//! and stamps `impl Target for dyn Product` blocks, one per *carrier* it is
//! given. A carrier is a supertrait whose bind machinery knows how to implement a
//! target on a `dyn` type. Two kinds exist:
//!
//! - **Shipped carriers** `DynClone` / `DynHash` (behind the `dyn_clone` /
//!   `dyn_hash` features). Their targets are `Clone` / `Hash`, neither of which
//!   can be a supertrait of a dyn-compatible trait. Binding them makes `Box<dyn
//!   Product>` cloneable and `dyn Product` hashable. This is the drop-in
//!   replacement for `dyn-clone`'s `clone_trait_object!(Product)` and
//!   `dyn-hash`'s `hash_trait_object!(Product)`.
//! - **A `#[dyn_shim]` shim** `DynPriced`. Its target is the source trait
//!   `Priced`, which is not dyn-compatible. Binding it makes `dyn Product` and
//!   `Box<dyn Product>` satisfy `Priced`, forwarding the dispatchable methods
//!   through the shim. This is how a non-dyn-compatible `Priced` reaches a trait
//!   object that could never list it as a supertrait.
//!
//! Each carrier is written as an explicit supertrait, so every `Product`
//! implementor must satisfy all three.
//!
//! Run with: `cargo run --example bind --features "dyn_clone dyn_hash"`

use dyn_shim::{DynClone, DynHash, dyn_shim, dyn_shim_bind};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hash};

// `Priced` is not dyn-compatible: `quote` is generic over its return type, so it
// cannot ride a vtable. `#[dyn_shim(DynPriced)]` builds a dyn-compatible shim of
// the dispatchable methods, which `Product` will inherit and bind back.
#[dyn_shim(DynPriced)]
trait Priced {
    fn price(&self) -> u32;
    fn discounted(&self, percent: u32) -> u32;
    // Generic over the return, so it cannot forward through the shim; the bind
    // gives it a panicking stub on the erased value.
    #[dyn_shim(panic)]
    fn quote<T: From<u32>>(&self) -> T;
}

// `Product` is dyn-compatible and is the type held behind `dyn`. The three
// carriers bind `Priced`, `Clone`, and `Hash` onto its objects.
#[dyn_shim_bind(DynPriced + DynClone + DynHash)]
trait Product: DynPriced + DynClone + DynHash {
    fn sku(&self) -> &str;
}

#[derive(Clone, Hash)]
struct Book {
    sku: String,
    cents: u32,
}
impl Priced for Book {
    fn price(&self) -> u32 {
        self.cents
    }
    fn discounted(&self, percent: u32) -> u32 {
        self.cents - self.cents * percent / 100
    }
    fn quote<T: From<u32>>(&self) -> T {
        T::from(self.cents)
    }
}
impl Product for Book {
    fn sku(&self) -> &str {
        &self.sku
    }
}

#[derive(Clone, Hash)]
struct Sticker {
    sku: String,
    cents: u32,
}
impl Priced for Sticker {
    fn price(&self) -> u32 {
        self.cents
    }
    fn discounted(&self, percent: u32) -> u32 {
        self.cents - self.cents * percent / 100
    }
    fn quote<T: From<u32>>(&self) -> T {
        T::from(self.cents)
    }
}
impl Product for Sticker {
    fn sku(&self) -> &str {
        &self.sku
    }
}

// Generic over `Priced` by reference: a `&dyn Product` satisfies it, no allocation.
fn shelf_price(p: &(impl Priced + ?Sized)) -> u32 {
    p.price()
}

// Generic over `Priced` by value: a `Box<dyn Product>` satisfies it.
fn checkout(p: impl Priced) -> u32 {
    p.discounted(10)
}

fn fingerprint<T: Hash + ?Sized>(value: &T) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

fn main() {
    let catalog: Vec<Box<dyn Product>> = vec![
        Box::new(Book {
            sku: "BOOK-1".into(),
            cents: 1200,
        }),
        Box::new(Sticker {
            sku: "STK-9".into(),
            cents: 150,
        }),
    ];

    // DynClone carrier: Box<dyn Product> is Clone, so the whole catalog duplicates.
    let copy = catalog.clone();
    println!(
        "catalog: {} items, copy: {} items",
        catalog.len(),
        copy.len()
    );

    for product in &catalog {
        // `dyn Product`'s own method.
        println!("\nsku:        {}", product.sku());

        // DynPriced shim carrier: &dyn Product is accepted as a &impl Priced.
        println!("shelf price: {}", shelf_price(&**product));

        // DynHash carrier: dyn Product is Hash, hashing like the concrete value.
        println!("fingerprint: {:016x}", fingerprint(&**product));
    }

    // `quote` is generic, so on an erased product it is the panicking stub. Call
    // it on the concrete type, before erasing.
    let quote: u64 = Book {
        sku: "BOOK-1".into(),
        cents: 1200,
    }
    .quote();
    println!("\nquote u64:  {quote}");

    // Each Box<dyn Product> is consumed as an owned impl Priced.
    for product in catalog {
        println!("checkout:   {}", checkout(product));
    }
}
