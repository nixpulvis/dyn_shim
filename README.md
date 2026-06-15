# dyn_shim

[![crates.io](https://img.shields.io/crates/v/dyn_shim.svg)](https://crates.io/crates/dyn_shim)
[![docs.rs](https://img.shields.io/docsrs/dyn_shim)](https://docs.rs/dyn_shim)
[![CI](https://github.com/nixpulvis/dyn_shim/actions/workflows/rust.yml/badge.svg)](https://github.com/nixpulvis/dyn_shim/actions/workflows/rust.yml)
[![license](https://img.shields.io/crates/l/dyn_shim.svg)](LICENSE)

Generate a dyn-compatible shim trait from one that is not, so a mixed set of
implementors can live behind a single `Box<dyn _>`.

```toml
[dependencies]
dyn_shim = "0.2"
```

```rust
use dyn_shim::dyn_shim;

// Not dyn-compatible: `build` returns `Self`, so `Box<dyn Parser>` is impossible.
#[dyn_shim(DynParser)]
trait Parser {
    fn parse(&self, input: &str) -> usize;
    fn build() -> Self;
}

struct Words;
impl Parser for Words {
    fn parse(&self, input: &str) -> usize { input.split_whitespace().count() }
    fn build() -> Self { Words }
}

struct Bytes;
impl Parser for Bytes {
    fn parse(&self, input: &str) -> usize { input.len() }
    fn build() -> Self { Bytes }
}

// `DynParser` is dyn-compatible, so a heterogeneous set lives behind one type.
let parsers: Vec<Box<dyn DynParser>> = vec![Box::new(Words), Box::new(Bytes)];
let total: usize = parsers.iter().map(|p| p.parse("a b c")).sum();
assert_eq!(total, 3 + 5);
```

## As a `dyn-clone` / `dyn-hash` Replacement

With the `dyn_clone` / `dyn_hash` feature, `DynClone` and `DynHash` are near
drop-in replacements for the [`dyn-clone`](https://crates.io/crates/dyn-clone)
and [`dyn-hash`](https://crates.io/crates/dyn-hash) crates. You keep the
`DynClone` / `DynHash` supertrait and the `clone_trait_object!(MyTrait)` or
`hash_trait_object!(MyTrait)` macro calls become a `#[dyn_shim_bind(DynClone)]` or
`#[dyn_shim_bind(DynHash)]` attribute on the trait.

```toml
dyn_shim = { version = "0.2", features = ["dyn_clone", "dyn_hash"] }
```

```rust
use dyn_shim::{dyn_shim_bind, DynClone, DynHash};

#[dyn_shim_bind(DynClone + DynHash)]
trait MyTrait: DynClone + DynHash { /* ... */ }
```

See the [API documentation](https://docs.rs/dyn_shim) for the rest.

## Examples

The [`examples/`](examples) directory has one runnable program per feature:

- [`shim.rs`](examples/shim.rs) - the core: turn a non-dyn-compatible trait into
  a shim so a mixed set of implementors lives behind one `Box<dyn _>`.
- [`clone_and_hash.rs`](examples/clone_and_hash.rs) - the recognized `Clone` and
  `Hash` bounds, making boxed shim objects cloneable and hashable.
- [`bind.rs`](examples/bind.rs) - `#[dyn_shim_bind]`, binding the
  `DynClone` / `DynHash` carriers and a `#[dyn_shim]` shim onto a trait that is
  already dyn-compatible. This is also the migration path from the
  [`dyn-clone`](https://crates.io/crates/dyn-clone) and
  [`dyn-hash`](https://crates.io/crates/dyn-hash) crates, replacing their
  `clone_trait_object!` / `hash_trait_object!` calls. Needs
  `--features "dyn_clone dyn_hash"`.
- [`reflexive.rs`](examples/reflexive.rs) - `reflexive` impls that let erased
  objects flow back into source-trait-generic code, plus the `erase` / `stub` /
  `panic` / `boxed` remediations for methods that cannot forward.
- [`foreign.rs`](examples/foreign.rs) - shimming a trait defined in another
  crate with `#[dyn_shim_foreign]`.

```sh
cargo run --example shim
```

## Testing

```sh
cargo test
```

The suite includes [`trybuild`](https://crates.io/crates/trybuild) UI tests under
`tests/ui/` that assert the compile errors for rejected traits and methods.

## License

Licensed under the MIT license. See [LICENSE](LICENSE) for details.
