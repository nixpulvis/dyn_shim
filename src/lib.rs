//! Generate a dyn-compatible shim trait and blanket impl from a source trait
//! that is not dyn-compatible.
//!
//! Some traits are not dyn-compatible, so you cannot hold a mixed set of
//! implementors behind one `Box<dyn Trait>`. The [`macro@dyn_shim`] attribute
//! reads the trait it is applied to, builds a second trait containing only the
//! dyn-compatible subset, and forwards each call to the original. Every
//! implementor of the source trait then works as a `dyn` shim.
//!
//! See [`macro@dyn_shim`] for the local form, [`macro@dyn_shim_foreign`] for
//! shimming a trait defined in another crate, and the method/bounds rules.
//!
//! By default the shim is a separate trait. A [reflexive
//! impl](macro@dyn_shim#reflexive-impl) (`reflexive = bare | boxed`) also makes
//! the shim's trait object satisfy the source trait itself, so it can be passed
//! to code written against the original. [`macro@trait_object`] does the same
//! for a *different* trait object: it mounts the shim's source trait (or the
//! shipped `Clone`/`Hash` carriers) onto a dyn-compatible trait you own, so
//! `dyn YourTrait` satisfies a trait it could not list as a supertrait.
//!
//! # Dyn `Clone` and `Hash`
//!
//! `Clone` and `Hash` cannot be supertraits of a dyn-compatible trait, so they
//! cannot be shimmed by restating them. This crate ships their shims directly,
//! each behind a feature, as drop-in equivalents of the `dyn_clone` and
//! `dyn_hash` crates:
//!
//! - With the `dyn_clone` feature, [`DynClone`]: `Box<dyn DynClone>` implements
//!   [`Clone`] and `dyn DynClone` implements [`ToOwned`].
//! - With the `dyn_hash` feature, [`DynHash`]: `dyn DynHash` implements
//!   [`Hash`], covering `Box<dyn DynHash>` through the standard library's
//!   forwarding impl.
//!
//! Both cover the `+ Send` and `+ Sync` marker variants.
//!
//! To give a trait of your own these capabilities, list `Clone`/`Hash` as
//! [bounds](macro@dyn_shim#recognized-bounds) on a [`macro@dyn_shim`] shim, or,
//! when the trait is already dyn-compatible, mount the `DynClone`/`DynHash`
//! carriers onto its trait objects with [`macro@trait_object`].

pub use dyn_shim_macros::{dyn_shim, dyn_shim_foreign};

// `trait_object` mounts any carrier onto a trait's objects — a `#[dyn_shim]`
// shim, or the shipped `DynClone`/`DynHash` (those two behind their features).
// A shim carrier needs no feature, so the attribute is always available; the
// `DynClone`/`DynHash` carriers it can name simply do not exist without them.
pub use dyn_shim_macros::trait_object;

// The machinery behind the ready-made shims below. It is not part of this
// crate's public API (the shims are), so it is re-exported only to define them
// and is hidden from the docs.
#[doc(hidden)]
pub use dyn_shim_macros::dyn_shim_recognized;

/// A dyn-compatible shim for [`Clone`].
///
/// Every `T: Clone` is a `DynClone`, and `Box<dyn DynClone>` is itself `Clone`,
/// cloning the underlying concrete value into a fresh box. `dyn DynClone` also
/// implements [`ToOwned`], so a borrowed `&dyn DynClone` can be promoted to an
/// owned box. The `+ Send` and `+ Sync` marker variants are covered too, so
/// `Box<dyn DynClone + Send>` is cloneable and stays `Send`.
///
/// Cloning requires `'static` contents, so `Box<dyn DynClone + 'a>` is not
/// cloneable for a borrowed `'a`.
///
/// ```
/// use dyn_shim::DynClone;
///
/// #[derive(Clone)]
/// struct Widget(u32);
///
/// let a: Box<dyn DynClone> = Box::new(Widget(7));
/// let _b = a.clone();
/// ```
#[cfg(feature = "dyn_clone")]
#[dyn_shim_recognized(Clone + Send + Sync)]
pub trait DynClone {}

/// Clone the concrete value behind a `dyn` trait object that carries the
/// [`DynClone`] machinery, into a fresh `Box` of that same trait object type.
/// This backs the [`trait_object`](macro@trait_object) attribute's `Clone`
/// support: a `Box<dyn Foo>` where `Foo: DynClone` is cloned by duplicating the
/// underlying concrete value and re-boxing it as `dyn Foo`.
///
/// It is not part of the public API; the attribute references it by absolute
/// path in its generated `Clone` impl.
#[cfg(feature = "dyn_clone")]
#[doc(hidden)]
pub fn __clone_box<T: ?Sized + DynClone + 'static>(value: &T) -> Box<T> {
    // The carrier clones the concrete value into a `Box<dyn DynClone>`: a fresh
    // allocation laid out exactly as the concrete type behind `value`. We want
    // that allocation typed as `Box<T>` (e.g. `Box<dyn Foo>`), which holds the
    // same data but carries `T`'s metadata. `DynClone`'s vtable cannot produce
    // `T`'s, so take the fresh data pointer and splice in `T`'s metadata, read
    // from `value`.
    let cloned: Box<dyn DynClone> = value.__dyn_shim_clone_box();
    let data: *mut () = Box::into_raw(cloned) as *mut ();
    let mut fat: *const T = value;
    // SAFETY: `fat` points to `T`, and its metadata describes the concrete type
    // behind `value`, which is exactly the type `cloned` holds, so the metadata
    // is valid for the freshly cloned value. We overwrite only the data half of
    // the pointer with the clone's address (the `assert_eq!` confirms that half
    // is the leading pointer-sized field), then reconstruct an owned `Box<T>`
    // over the clone's allocation. Dropping it uses `T`'s size and align from
    // the metadata, matching the allocation the carrier made for that type.
    unsafe {
        let slot = &mut fat as *mut *const T as *mut *mut ();
        assert_eq!(*slot as *const (), value as *const T as *const ());
        *slot = data;
        Box::from_raw(fat as *mut T)
    }
}

/// A dyn-compatible shim for [`Hash`].
///
/// Every `T: Hash` is a `DynHash`, and `dyn DynHash` implements [`Hash`],
/// hashing like the underlying concrete value. Through the standard library's
/// `impl<T: ?Sized + Hash> Hash for Box<T>`, `Box<dyn DynHash>` and `&dyn
/// DynHash` hash the same way. The `+ Send` and `+ Sync` marker variants are
/// covered too.
///
/// ```
/// use dyn_shim::DynHash;
/// use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
///
/// let bh = BuildHasherDefault::<DefaultHasher>::default();
/// let boxed: Box<dyn DynHash> = Box::new(42u32);
/// assert_eq!(bh.hash_one(&*boxed), bh.hash_one(42u32));
/// ```
#[cfg(feature = "dyn_hash")]
#[dyn_shim_recognized(Hash + Send + Sync)]
pub trait DynHash {}
