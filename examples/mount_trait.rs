//! Mounting a non-dyn-compatible trait onto a trait object you own, using a
//! `#[dyn_shim]` shim as the carrier for `#[trait_object]`.
//!
//! `Priced` is not dyn-compatible: `quote` is generic over its return type, so
//! it cannot enter a vtable, and therefore `Priced` cannot be a supertrait of a
//! dyn-compatible trait. We still want a `&dyn Product` to satisfy `Priced` so it
//! can flow into `Priced`-generic code.
//!
//! The bridge is two steps that share one mechanism:
//!
//! - `#[dyn_shim(DynPriced)]` builds a dyn-compatible shim of `Priced`'s
//!   dispatchable methods. `Product` inherits `DynPriced`, so `dyn Product`
//!   carries them.
//! - `#[trait_object(DynPriced)]` mounts `Priced` back onto `dyn Product` and
//!   `Box<dyn Product>`, forwarding through the shim. `quote` is generic, so it
//!   gets a panicking stub there (`#[dyn_shim(panic)]`); call it on a concrete
//!   product, before erasing.
//!
//! This is the same mounting operation `reflexive` performs, aimed at a trait
//! object other than the shim's own. The shim's mount macro is what carries
//! `Priced`'s method shapes to the `#[trait_object]` site, so it works even when
//! `Product` lives in a different module or crate from `Priced`.
//!
//! Run with: `cargo run --example mount_trait`

use dyn_shim::{dyn_shim, trait_object};

#[dyn_shim(DynPriced)]
trait Priced {
    fn price(&self) -> u32;
    fn discounted(&self, percent: u32) -> u32;
    // Generic over the return type, so not dyn-compatible: it cannot forward
    // through the shim, so the mounted impl gives it a panicking stub.
    #[dyn_shim(panic)]
    fn quote<T: From<u32>>(&self) -> T;
}

// `Product` is dyn-compatible and is the type held behind `dyn`. It inherits the
// `DynPriced` carrier, and `#[trait_object(DynPriced)]` mounts `Priced` onto its
// objects.
#[trait_object(DynPriced)]
trait Product: DynPriced {
    fn sku(&self) -> &str;
}

struct Book {
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
        "BOOK-1"
    }
}

// Generic over `Priced` by reference: a `&dyn Product` satisfies it through the
// mounted bare impl, no allocation.
fn shelf_price(p: &(impl Priced + ?Sized)) -> u32 {
    p.price()
}

// Generic over `Priced` by value: a `Box<dyn Product>` satisfies it through the
// mounted boxed impl.
fn checkout(p: impl Priced) -> u32 {
    p.discounted(10)
}

fn main() {
    let product: Box<dyn Product> = Box::new(Book { cents: 1200 });

    // `dyn Product`'s own method.
    println!("sku:       {}", product.sku());

    // `&dyn Product` is accepted as a `&impl Priced` (bare mount).
    println!("price:     {}", shelf_price(&*product));

    // `quote` is generic, so it lives only on the concrete type; on the erased
    // product it is the panicking stub. Call it before erasing.
    let q: u64 = Book { cents: 1200 }.quote();
    println!("quote u64: {q}");

    // `Box<dyn Product>` is consumed as an owned `impl Priced` (boxed mount).
    println!("checkout:  {}", checkout(product));
}
