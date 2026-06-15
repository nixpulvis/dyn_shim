# dyn_shim

[![crates.io](https://img.shields.io/crates/v/dyn_shim.svg)](https://crates.io/crates/dyn_shim)
[![docs.rs](https://img.shields.io/docsrs/dyn_shim)](https://docs.rs/dyn_shim)
[![CI](https://github.com/nixpulvis/dyn_shim/actions/workflows/rust.yml/badge.svg)](https://github.com/nixpulvis/dyn_shim/actions/workflows/rust.yml)
[![license](https://img.shields.io/crates/l/dyn_shim.svg)](LICENSE)

Make non-dyn-compatible traits usable behind `dyn`, and give trait objects
capabilities they cannot carry as supertraits — `Clone`, `Hash`, or a trait of
your own.

```toml
[dependencies]
dyn_shim = "0.2"
```

## Hold a mixed collection behind one `Box<dyn>`

A trait with a by-value `self`, a `-> Self`, or a generic method is not
dyn-compatible, so `Box<dyn Trait>` is impossible. `#[dyn_shim(Name)]` generates
a dyn-compatible shim over the dispatchable methods, plus a blanket impl so every
implementor already is one:

```rust
use dyn_shim::dyn_shim;

#[dyn_shim(DynParser)]
trait Parser {
    fn parse(&self, input: &str) -> usize;
    fn build() -> Self; // skipped: not dispatchable through a trait object
}

// A heterogeneous set of parsers behind one type.
let parsers: Vec<Box<dyn DynParser>> = Vec::new();
```

A method that is non-dyn-compatible only because of a generic argument can be
kept instead of skipped with `#[dyn_shim(erase)]`, which lowers a parameter
bounded by one trait and used behind a reference to a trait object (`&mut impl
Write` becomes `&mut dyn Write`). It is the same erasure the recognized `Hash`
bound applies to `Hash::hash`'s generic hasher.

## Make trait objects `Clone` or `Hash`

`Clone` and `Hash` cannot be supertraits of a dyn-compatible trait. List them as
recognized bounds on a shim and its trait objects gain them — a drop-in for the
`dyn-clone` and `dyn-hash` crates:

```rust
use dyn_shim::dyn_shim;

#[dyn_shim(DynShape: Clone + Hash)]
trait Shape {
    fn area(&self) -> f64;
}

// Box<dyn DynShape> is Clone; dyn DynShape is Hash.
```

Already own a dyn-compatible trait? `#[trait_object]` mounts the same capabilities
onto `dyn YourTrait` in place, generating no shim (the `DynClone`/`DynHash`
carriers are behind the `dyn_clone`/`dyn_hash` features):

```rust
use dyn_shim::{trait_object, DynClone};

#[trait_object(DynClone)]
trait Widget: DynClone {
    fn render(&self) -> String;
}

// Box<dyn Widget> is Clone.
```

## Pass an erased value back into trait-generic code

`Rule` below is not dyn-compatible — its generic `threshold` rules out `dyn Rule`
— so a mixed set lives behind `Box<dyn DynRule>`. But a `Box<dyn DynRule>` is not
a `Rule`. The `reflexive` option makes it one, so the erased value still flows
into functions written against the original trait:

```rust
use dyn_shim::dyn_shim;

#[dyn_shim(DynRule, reflexive = bare + boxed)]
trait Rule {
    fn check(&self, n: i32) -> bool;
    #[dyn_shim(panic)]
    fn threshold<T: From<i32>>(&self) -> T; // generic: rules out `dyn Rule`
}

fn passes(rule: &(impl Rule + ?Sized), n: i32) -> bool {
    rule.check(n)
}

// A &dyn DynRule satisfies Rule, so an erased rule can be passed to `passes`.
```

A method that cannot forward through the shim is opted into a panicking stub
with `#[dyn_shim(panic)]`, as `threshold` is above. A `-> Self` builder is the
exception: `#[dyn_shim(boxed)]` makes the shim method return `Box<dyn DynRule>`,
so a `reflexive = boxed` impl keeps the builder working on the erased value
(the general form of `Clone`'s boxing) instead of stubbing it.

The same machinery mounts a non-dyn-compatible trait onto a *different* trait
object you own: have it inherit the shim, and `#[trait_object(DynRule)]` gives
`dyn YourTrait` the `Rule` it could never list as a supertrait.

## Shim a trait from another crate

`#[dyn_shim]` needs the trait's own definition. `#[dyn_shim_foreign(path)]` shims
one you do not own by restating its dispatchable methods to forward:

```rust
use dyn_shim::dyn_shim_foreign;

#[dyn_shim_foreign(other_crate::Sink)]
trait DynSink: Clone {
    fn write(&mut self, line: &str);
}

// Box<dyn DynSink> holds any Clone implementor of other_crate::Sink.
```

## Documentation

The [API documentation](https://docs.rs/dyn_shim) covers the full
method-selection rules, the `reflexive`, recognized-bound, and `#[trait_object]`
carrier mechanics, and the `dyn_clone`/`dyn_hash` features. A runnable program
for each lives in [`examples/`](examples/).

## Testing

```sh
cargo test
```

The suite includes [`trybuild`](https://crates.io/crates/trybuild) UI tests under
`tests/ui/` that assert the compile errors for rejected traits and methods.

## License

Licensed under the MIT license. See [LICENSE](LICENSE) for details.
