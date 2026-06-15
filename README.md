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

## `dyn-hash` support

With the `dyn_hash` feature, `DynHash` is a drop-in for the
[`dyn-hash`](https://crates.io/crates/dyn-hash) crate:

```toml
dyn_shim = { version = "0.2", features = ["dyn_hash"] }
```

```rust
use dyn_shim::DynHash;

// `dyn DynHash` (and so `Box<dyn DynHash>`) implements `Hash`.
let boxed: Box<dyn DynHash> = Box::new(42u32);
```

See the [API documentation](https://docs.rs/dyn_shim) for the rest.

## Testing

```sh
cargo test
```

The suite includes [`trybuild`](https://crates.io/crates/trybuild) UI tests under
`tests/ui/` that assert the compile errors for rejected traits and methods.

## License

Licensed under the MIT license. See [LICENSE](LICENSE) for details.
