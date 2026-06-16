// These cases shell out to cargo through trybuild, which Miri cannot run under
// its filesystem isolation. Under Miri the whole target compiles to no tests,
// so `cargo +nightly miri test` runs only the behavioral suite in `it.rs`.
#![cfg(not(miri))]

// Each case applies `#[dyn_shim(Dyn)]` to a trait containing one method that is
// not dyn-compatible, then tries to reach that method through `dyn Dyn`. The
// method must be absent from the generated shim, so each program must fail to
// compile. This is the inverse check of the behavioral tests in `it.rs`.
#[test]
fn skipped_methods_absent_from_shim() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/skip_receiverless.rs");
    t.compile_fail("tests/ui/skip_generic.rs");
    t.compile_fail("tests/ui/skip_const_generic.rs");
    t.compile_fail("tests/ui/skip_async.rs");
    t.compile_fail("tests/ui/skip_self_return.rs");
    t.compile_fail("tests/ui/skip_self_arg.rs");
    t.compile_fail("tests/ui/skip_impl_trait_arg.rs");
    t.compile_fail("tests/ui/skip_impl_trait_ret.rs");
    t.compile_fail("tests/ui/skip_self_sized.rs");
    t.compile_fail("tests/ui/skip_attr.rs");
}

// An invalid `#[dyn_shim(...)]` helper attribute is rejected with a direct
// error: on a non-method trait item the attribute is unsupported entirely, and
// on a method the only recognized argument is `skip`. Both errors come from
// the macro itself, not rustc, so the snapshots are stable across toolchains.
#[test]
fn invalid_helper_attrs_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/attr_non_method.rs");
    t.compile_fail("tests/ui/attr_unknown_arg.rs");
    t.compile_fail("tests/ui/foreign_missing_path.rs");
    t.compile_fail("tests/ui/foreign_default_not_dyn_compatible.rs");
}

// A recognized bound (`Clone`, `Hash`) constrains the blanket impl, so an
// implementor that does not satisfy it never receives the shim (a rustc
// error; the pinned toolchain keeps the snapshot stable).
#[test]
fn recognized_bounds_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/bound_clone_unsatisfied.rs");
    t.compile_fail("tests/ui/bound_hash_unsatisfied.rs");
}

// Bounds the macro recognizes only to reject: each would otherwise pass
// through as a supertrait and break the shim with a confusing error far from
// the cause. An arbitrary non-dyn-compatible trait cannot be recognized by
// name, so it passes through and rustc rejects the shim at its first `dyn`
// use site (a rustc error; the pinned toolchain keeps it stable).
#[test]
fn impossible_bounds_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/bound_copy.rs");
    t.compile_fail("tests/ui/bound_sized.rs");
    t.compile_fail("tests/ui/bound_default.rs");
    t.compile_fail("tests/ui/bound_eq.rs");
    t.compile_fail("tests/ui/bound_partial_eq.rs");
    t.compile_fail("tests/ui/bound_ord.rs");
    t.compile_fail("tests/ui/bound_partial_ord.rs");
    t.compile_fail("tests/ui/bound_not_dyn_compatible.rs");
    t.compile_fail("tests/ui/bound_path_form.rs");
    t.compile_fail("tests/ui/bound_maybe_sized.rs");
}

// The marker coverage of a recognized bound is opt-in: combinations exist
// only for auto traits actually listed in the bounds.
#[test]
fn unlisted_marker_not_covered() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/bound_clone_unlisted_marker.rs");
}

// A `reflexive` impl must account for every source method. `reflexive = bare`
// cannot express a by-value `self` receiver (the `dyn` shim is unsized), and a
// non-dyn-compatible method cannot forward through the shim unless it is opted
// into a panicking stub with `#[dyn_shim(panic)]`. Both errors come from the
// macro itself, so the snapshots are stable across toolchains.
#[test]
fn reflexive_impl_must_be_complete() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/reflexive_bare_by_value.rs");
    t.compile_fail("tests/ui/reflexive_both_bare_by_value.rs");
    t.compile_fail("tests/ui/reflexive_unstubbed_method.rs");
    t.compile_fail("tests/ui/reflexive_unstubbed_multiple.rs");
    t.compile_fail("tests/ui/reflexive_bare_self_sized_uncallable.rs");
}

// `#[dyn_shim(erase)]` lowers a method's generic parameters to trait objects,
// but only those used behind a reference. A parameter in the return type (no
// reference to lower, no single concrete type to pick) and a by-value parameter
// (no reference to lower) are both rejected directly by the macro, so those
// snapshots are stable across toolchains. A parameter that *is* behind a
// reference but whose bound does not forward through references is accepted by
// the macro and instead fails to compile on the generated forwarding call; that
// snapshot is a rustc trait error, so it is toolchain-dependent.
#[test]
fn erase_rejects_inexpressible() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/erase_return_generic.rs");
    t.compile_fail("tests/ui/erase_by_value.rs");
    t.compile_fail("tests/ui/erase_bound_not_forwarding.rs");
}

// `#[dyn_shim(boxed)]` boxes a `-> Self` builder into `Box<dyn Shim>`. That box
// is marker-free, so combining the helper with an auto-trait marker on a
// reflexive shim is rejected by the macro (a stable snapshot).
#[test]
fn boxed_rejects_markers() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/boxed_with_markers.rs");
}

// `dyn_shim_recognized` only knows `Clone` and `Hash`, and generates the shim's
// contents itself, so an unrecognized trait and a shim that declares its own
// items are both rejected by the macro.
#[test]
fn recognized_misuse_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/recognized_unknown_trait.rs");
    t.compile_fail("tests/ui/recognized_has_items.rs");
}

// `dyn_shim_bind` binds a carrier onto a trait that already inherits it. It
// rejects a missing carrier supertrait, an argument list naming no carrier, and
// the old spelling that named a capability (`Clone`/`Hash`) instead of a
// carrier trait. All three errors come from the macro itself, so the snapshots
// are stable across toolchains.
#[test]
fn dyn_shim_bind_misuse_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/dyn_shim_bind_missing_carrier.rs");
    t.compile_fail("tests/ui/dyn_shim_bind_no_recognized.rs");
    t.compile_fail("tests/ui/dyn_shim_bind_capability_name.rs");
}
