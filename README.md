# dyn_shim

[![crates.io](https://img.shields.io/crates/v/dyn_shim.svg)](https://crates.io/crates/dyn_shim)
[![docs.rs](https://img.shields.io/docsrs/dyn_shim)](https://docs.rs/dyn_shim)
[![CI](https://github.com/nixpulvis/dyn_shim/actions/workflows/rust.yml/badge.svg)](https://github.com/nixpulvis/dyn_shim/actions/workflows/rust.yml)
[![license](https://img.shields.io/crates/l/dyn_shim.svg)](LICENSE)

Generate a dyn-compatible shim trait and blanket impl from a source trait that
is not dyn-compatible.

Some traits are not dyn-compatible, so you cannot hold a mixed set of
implementors behind one `Box<dyn Trait>`. The `#[dyn_shim(Name)]` attribute
reads the trait it is applied to, builds a second trait containing only the
dyn-compatible subset, and forwards each call to the original. Every implementor
of the source trait then works as a `dyn` shim.

## Usage

Add the dependency:

```toml
[dependencies]
dyn_shim = "0.2"
```

Annotate the trait with `#[dyn_shim(Name)]`, where `Name` is the shim trait to
generate:

```rust
use dyn_shim::dyn_shim;

#[dyn_shim(DynSink)]
trait Sink {
    // ...
}
```

Bounds after the shim's name become its supertraits. A `Clone` or `Hash` in
the list is recognized and handled specially: it makes the shim's trait
objects themselves cloneable (including `ToOwned`) or hashable, covering the
marker combinations of any auto traits listed alongside:

```rust
use dyn_shim::dyn_shim;

#[dyn_shim(DynShape: Clone + Send)]
trait Shape {
    fn area(&self) -> f64;
    fn scale(&mut self, factor: f64);
}

// Box<dyn DynShape> and Box<dyn DynShape + Send> implement Clone.
```

## Reflexive impls

By default the shim is a separate trait, so a `Box<dyn DynFoo>` is not a `Foo`.
Adding `reflexive = boxed` also generates `impl Foo for Box<dyn DynFoo>`, so the
boxed trait object satisfies the source trait itself and can be passed to code
written against `Foo`. Methods that cannot be dispatched through the shim (a
constructor, a generic method) are opted into a panicking stub with
`#[dyn_shim(panic)]`:

```rust
use dyn_shim::dyn_shim;

#[dyn_shim(DynMunch, reflexive = boxed)]
trait Munch {
    fn crunch(self) -> u32;
    #[dyn_shim(panic)]
    fn fresh() -> Self; // not dispatchable: panics if called on the box
}

fn eat(m: impl Munch) -> u32 {
    m.crunch()
}

// Box<dyn DynMunch> is a Munch, so it can be passed to `eat`.
```

`reflexive = bare` instead generates `impl Foo for dyn DynFoo`, so a `&dyn
DynFoo` satisfies `Foo` by reference. It cannot express a by-value `self` or a
`-> Self`, since `dyn DynFoo` is unsized; use `reflexive = boxed` for those.

## Foreign traits

`#[dyn_shim]` has to sit on the trait's own definition, so it cannot target a
trait from a dependency. `#[dyn_shim_foreign(path)]` does: the annotated trait
*is* the shim, restating the foreign methods to forward, and the macro fills in
the forwarding machinery plus a blanket impl pointing at the foreign path. Its
name, visibility, and supertrait list work just like `#[dyn_shim]`'s. A proc
macro cannot see another crate's trait body, so the signatures must be restated
by hand; a mismatch is caught when the generated forwarding call fails to
compile.

```rust
use dyn_shim::dyn_shim_foreign;

#[dyn_shim_foreign(other_crate::Sink)]
trait DynSink: Clone {
    fn write(&mut self, line: &str);
    fn finish(self) -> usize;
}

// Box<dyn DynSink> holds any Clone implementor of other_crate::Sink.
```

The `reflexive` option and `#[dyn_shim(panic)]` work on the foreign form too.

## Features

`Clone` and `Hash` cannot be supertraits of a dyn-compatible trait, so this
crate ships their shims directly, each behind a feature:

```toml
[dependencies]
dyn_shim = { version = "0.2", features = ["dyn_clone", "dyn_hash"] }
```

- `dyn_clone` provides `DynClone`: `Box<dyn DynClone>` implements `Clone` and `dyn
  DynClone` implements `ToOwned`. It is a drop-in for the `dyn-clone` crate's
  `DynClone`.
- `dyn_hash` provides `DynHash`: `dyn DynHash` implements `Hash` (covering `Box<dyn
  DynHash>` through the standard library's forwarding impl). It mirrors the
  `dyn-hash` crate.

With a feature on, a recognized `Clone`/`Hash` bound also makes the shim a
subtrait of `DynClone`/`DynHash`, so `Box<dyn DynFoo>` (or `&dyn DynFoo`)
upcasts to `Box<dyn DynClone>` (or `&dyn DynHash`) and flows into APIs typed
against those.

## Mounting a trait onto an existing trait object

A reflexive impl and `#[trait_object]` are the same operation: *mount* a trait
onto a trait object — emit `impl Target for <object>` so an erased value
satisfies a trait it cannot carry as a supertrait. The mounting machinery lives
in a generated macro on a **carrier**; both forms invoke it. A reflexive impl
mounts the source trait onto its own generated shim's object; `#[trait_object]`
mounts a carrier onto a trait you already own.

`#[trait_object(Carrier)]` re-emits the annotated trait untouched and mounts each
listed carrier — a trait the annotated trait inherits as a supertrait — onto its
`dyn` objects. Two kinds of carrier, reached the same way:

The shipped `DynClone`/`DynHash` (whose targets, `Clone`/`Hash`, cannot be
supertraits of a dyn-compatible trait):

```rust
use dyn_shim::{trait_object, DynClone, DynHash};

#[trait_object(DynHash + DynClone)]
trait Shape: DynHash + DynClone {
    fn area(&self) -> u32;
}

// dyn Shape implements Hash, and Box<dyn Shape> implements Clone.
```

Or any `#[dyn_shim]` shim, which mounts its *source* trait. This is the general
case: a non-dyn-compatible `Foo` cannot be a supertrait of your dyn-compatible
`Bar`, so instead inherit the shim `DynFoo` and mount `Foo` back onto `dyn Bar`:

```rust
use dyn_shim::{dyn_shim, trait_object};

#[dyn_shim(DynFoo)]
trait Foo {
    fn weight(&self) -> u32;
    #[dyn_shim(panic)]
    fn build<T: From<u32>>(&self) -> T; // generic: not dyn-compatible
}

#[trait_object(DynFoo)]
trait Bar: DynFoo {
    fn name(&self) -> &str;
}

// &dyn Bar and Box<dyn Bar> satisfy Foo, forwarding through DynFoo.
fn weigh(f: &impl Foo) -> u32 { f.weight() }
```

Several carriers may be combined (`#[trait_object(DynFoo + DynClone)]`), and
auto-trait markers (`#[trait_object(DynClone + Send)]`) select the covered `dyn`
variants, like a recognized bound. The difference from `#[dyn_shim(DynShape:
Hash)]` is the contract: the carrier is a supertrait, so *every* implementor must
satisfy it, whereas the shim form only filters which implementors become the
shim. Reach for `#[trait_object]` when `dyn Bar` is the type you use directly.
`DynClone` requires the `dyn_clone` feature and `DynHash` the `dyn_hash` feature,
since those define those carriers; a shim carrier needs no feature.

See the [API documentation](https://docs.rs/dyn_shim) for details.

## Testing

```sh
cargo test
```

The suite includes [`trybuild`](https://crates.io/crates/trybuild) UI tests
under `tests/ui/` that assert the compile errors for rejected traits and methods.

## License

Licensed under the MIT license. See [LICENSE](LICENSE) for details.
