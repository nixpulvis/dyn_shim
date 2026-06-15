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

// `trait_object` mounts a carrier onto a trait that already inherits it. It
// rejects a missing carrier supertrait, an argument list naming no carrier, and
// the old spelling that named a capability (`Clone`/`Hash`) instead of a
// carrier trait. All three errors come from the macro itself, so the snapshots
// are stable across toolchains.
#[test]
fn trait_object_misuse_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/trait_object_missing_carrier.rs");
    t.compile_fail("tests/ui/trait_object_no_recognized.rs");
    t.compile_fail("tests/ui/trait_object_capability_name.rs");
}
