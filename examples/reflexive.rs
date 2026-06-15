//! Letting erased shim objects flow back into `Rule`-generic code, and the
//! ladder of remediations for the methods that cannot forward through the shim.
//!
//! `Rule` is not dyn-compatible — its generic methods rule out a vtable — so a
//! mixed `Vec<Box<dyn DynRule>>` holds the generated `DynRule` shim. On its own
//! a `dyn DynRule` is not a `Rule`, so `Rule`-generic functions cannot take one.
//! `reflexive = bare + boxed` bridges that gap from both directions:
//!
//! - `bare` emits `impl Rule for dyn DynRule`, so a borrow (`&dyn DynRule`)
//!   satisfies `Rule` by reference.
//! - `boxed` emits `impl Rule for Box<dyn DynRule>`, so an owned box satisfies
//!   `Rule` by value.
//!
//! Each method that cannot forward through the shim picks a remediation, best to
//! last resort:
//!
//! - `explain` is generic over a `Write` *argument*. `#[dyn_shim(erase)]` lowers
//!   `&mut W` to `&mut dyn Write`, so it forwards for real, not as a stub.
//! - `parsed` is generic over its *return*, so it cannot forward — but the return
//!   can express absence, so `#[dyn_shim(stub = None)]` degrades to `None` on an
//!   erased value instead of aborting.
//! - `threshold` is generic over its return with no such fallback, so
//!   `#[dyn_shim(panic)]` is the last resort. Call it on a concrete rule.
//!
//! A `-> Self` builder is the other reflexive remediation, `#[dyn_shim(boxed)]`;
//! it needs `reflexive = boxed` (a bare `dyn` cannot be a returned `Self`), so it
//! lives in the `builder` example.
//!
//! Run with: `cargo run --example reflexive`

use dyn_shim::dyn_shim;
use std::io::Write;
use std::str::FromStr;

#[dyn_shim(DynRule, reflexive = bare + boxed)]
trait Rule {
    fn name(&self) -> &str;
    fn check(&self, value: i32) -> bool;

    // Generic over a `Write` argument, used behind `&mut`: `erase` lowers it to
    // `&mut dyn Write`, so the method enters the shim's vtable and forwards.
    #[dyn_shim(erase)]
    fn explain<W: Write>(&self, out: &mut W);

    // Generic over the return type, so it cannot forward. The return can express
    // "no value", so degrade to `None` on an erased value rather than abort.
    #[dyn_shim(stub = None)]
    fn parsed<T: FromStr>(&self, text: &str) -> Option<T>;

    // Generic over the return type with no graceful fallback: panic if reached on
    // an erased value. Call it on a concrete rule, before erasing.
    #[dyn_shim(panic)]
    fn threshold<T: From<i32>>(&self) -> T;
}

struct AtLeast {
    floor: i32,
}
impl Rule for AtLeast {
    fn name(&self) -> &str {
        "at_least"
    }
    fn check(&self, value: i32) -> bool {
        value >= self.floor
    }
    fn explain<W: Write>(&self, out: &mut W) {
        write!(out, "value >= {}", self.floor).unwrap();
    }
    fn parsed<T: FromStr>(&self, text: &str) -> Option<T> {
        text.parse().ok()
    }
    fn threshold<T: From<i32>>(&self) -> T {
        T::from(self.floor)
    }
}

struct Even;
impl Rule for Even {
    fn name(&self) -> &str {
        "even"
    }
    fn check(&self, value: i32) -> bool {
        value % 2 == 0
    }
    fn explain<W: Write>(&self, out: &mut W) {
        write!(out, "value % 2 == 0").unwrap();
    }
    fn parsed<T: FromStr>(&self, text: &str) -> Option<T> {
        text.parse().ok()
    }
    fn threshold<T: From<i32>>(&self) -> T {
        T::from(0)
    }
}

// Generic over `Rule` by reference. A `&dyn DynRule` satisfies the bound through
// the bare reflexive impl, so it forwards without an allocation.
fn passes<R: Rule + ?Sized>(rule: &R, value: i32) -> bool {
    rule.check(value)
}

// Generic over `Rule` by value. A `Box<dyn DynRule>` satisfies the bound through
// the boxed reflexive impl, so the owned object can be consumed here.
fn into_name(rule: impl Rule) -> String {
    rule.name().to_string()
}

fn main() {
    let rules: Vec<Box<dyn DynRule>> = vec![Box::new(AtLeast { floor: 10 }), Box::new(Even)];

    let value = 12;
    for rule in &rules {
        // With both reflexive impls in scope a shim object is both a `DynRule`
        // and a `Rule`, so `name` is qualified to the shim.
        let name = DynRule::name(&**rule);

        // `explain` forwards through the shim — its `W: Write` was erased to
        // `&mut dyn Write`, so `&mut Vec<u8>` coerces straight in.
        let mut why = Vec::new();
        DynRule::explain(&**rule, &mut why);
        let why = String::from_utf8(why).unwrap();

        // bare: `&**rule` is a `&dyn DynRule`, accepted as a `&impl Rule`.
        println!("{name}: {value} passes = {} ({why})", passes(&**rule, value));

        // `parsed` is generic, so it is not on the shim; on the erased value it
        // reaches the `None` stub rather than panicking.
        let parsed: Option<i32> = Rule::parsed(&**rule, "41");
        println!("  parsed(\"41\") on the erased rule = {parsed:?}");
    }

    // boxed: each `Box<dyn DynRule>` is an owned `impl Rule`, consumed by value.
    for rule in rules {
        println!("consumed rule named {}", into_name(rule));
    }

    // `threshold` has only a panicking stub once erased, so call it on a concrete
    // rule. (`AtLeast { floor: 11 }.threshold()` would also reach the stub via the
    // reflexive impl if called on a `Box<dyn DynRule>`.)
    let floor: i64 = AtLeast { floor: 11 }.threshold();
    println!("concrete threshold: {floor}");
}
