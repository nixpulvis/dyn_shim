//! Proc-macro implementation of the [`dyn_shim`](https://docs.rs/dyn_shim)
//! crate. Depend on `dyn_shim`, not this crate directly: it re-exports these
//! macros and adds the feature-gated `DynClone`/`DynHash` traits.
//!
//! [`macro@dyn_shim`] generates a dyn-compatible shim trait and blanket impl
//! from a source trait that is not dyn-compatible; [`macro@dyn_shim_foreign`]
//! does the same for a trait defined in another crate. See [`macro@dyn_shim`]
//! for the method-forwarding and bounds rules, the reflexive impl, and the
//! limitations.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Expr, FnArg, GenericParam, Ident, ItemTrait, Pat, Path, Receiver, ReturnType,
    Signature, Token, TraitItem, TraitItemFn, Type, TypeParamBound, parse_macro_input, parse_quote,
};

/// Which trait-object form an impl lands on: the bare unsized `dyn X` or the
/// sized `Box<dyn X>`. Both the `reflexive` impls and the recognized
/// `Clone`/`Hash` bridges are the same operation, `impl Target for <object
/// form>`, so they share this axis. A reflexive impl picks a set of these
/// (`reflexive = bare`, `boxed`, or `bare + boxed`); each recognized bridge
/// picks the form its capability needs (`Hash` on bare, `Clone` on boxed).
#[derive(Clone, Copy, PartialEq)]
enum ObjectForm {
    /// `dyn X`. As a reflexive `impl Target for dyn X`, `Self` is the unsized
    /// `dyn` type, so a by-value `self` receiver or a by-value `Self` in the
    /// signature cannot be expressed.
    Bare,
    /// `Box<dyn X>`. As a reflexive `impl Target for Box<dyn X>`, `Self` is the
    /// sized boxed type, so by-value `self` and `-> Self` become `Box<dyn X>`
    /// and work.
    Boxed,
}

impl ObjectForm {
    /// The trait-object type this form names for `principal` carrying `markers`:
    /// `dyn principal markers` (bare) or `Box<dyn principal markers>` (boxed).
    /// `Box` is named by absolute path so the expansion does not depend on what
    /// `Box` resolves to at the call site.
    fn ty(self, principal: impl ToTokens, markers: impl ToTokens) -> TokenStream2 {
        match self {
            ObjectForm::Bare => quote! { dyn #principal #markers },
            ObjectForm::Boxed => quote! { ::std::boxed::Box<dyn #principal #markers> },
        }
    }
}

/// Parse an optional trailing `, reflexive = <kinds>` from an attribute's
/// argument list, where `<kinds>` is one or more of `bare` and `boxed` joined
/// with `+`; `reflexive = bare + boxed` emits both impls. Returns the requested
/// kinds, deduplicated, and empty when no `reflexive` argument is present. The
/// argument comes after whatever each attribute reads first (the shim name and
/// bounds for [`macro@dyn_shim`], the source path for [`macro@dyn_shim_foreign`]).
fn parse_reflexive(input: ParseStream) -> syn::Result<Vec<ObjectForm>> {
    if !input.peek(Token![,]) {
        return Ok(Vec::new());
    }
    input.parse::<Token![,]>()?;
    let key: Ident = input.parse()?;
    if key != "reflexive" {
        return Err(syn::Error::new_spanned(key, "expected `reflexive`"));
    }
    input.parse::<Token![=]>()?;
    let mut kinds = Vec::new();
    for ident in Punctuated::<Ident, Token![+]>::parse_separated_nonempty(input)? {
        let kind = match ident.to_string().as_str() {
            "bare" => ObjectForm::Bare,
            "boxed" => ObjectForm::Boxed,
            _ => {
                return Err(syn::Error::new_spanned(
                    ident,
                    "unsupported reflexive kind, expected `bare` or `boxed`",
                ));
            }
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    Ok(kinds)
}

/// Arguments to [`macro@dyn_shim`]: the shim trait's name, optionally followed
/// by supertraits to put on it, written like a trait's supertrait list
/// (`DynFoo: Send + Sync`), and an optional `, reflexive = bare | boxed | bare + boxed`.
struct Args {
    shim_name: Ident,
    bounds: Punctuated<TypeParamBound, Token![+]>,
    reflexive: Vec<ObjectForm>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let shim_name = input.parse()?;
        let mut bounds = Punctuated::new();
        if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            bounds = Punctuated::parse_separated_nonempty(input)?;
        }
        let reflexive = parse_reflexive(input)?;
        Ok(Args {
            shim_name,
            bounds,
            reflexive,
        })
    }
}

/// Arguments to [`macro@dyn_shim_foreign`]: the path to the foreign source
/// trait, and an optional `, reflexive = bare | boxed | bare + boxed`.
struct ForeignArgs {
    source: Path,
    reflexive: Vec<ObjectForm>,
}

impl Parse for ForeignArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let source = input.parse()?;
        let reflexive = parse_reflexive(input)?;
        Ok(ForeignArgs { source, reflexive })
    }
}

/// A bare `+`-joined bound list, the whole attribute argument for both
/// [`macro@dyn_shim_recognized`] (a recognized trait to expose as a shim, plus
/// auto-trait markers) and [`macro@trait_object`] (recognized traits to
/// implement for a trait's `dyn` objects, plus markers). Each validates the
/// contents itself; this only parses the syntax (`Clone + Send`).
struct BoundList {
    bounds: Punctuated<TypeParamBound, Token![+]>,
}

impl Parse for BoundList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(BoundList {
            bounds: Punctuated::parse_separated_nonempty(input)?,
        })
    }
}

/// The bound's trait path, if the bound is a plain trait name with no
/// modifier or binder. Recognition applies only to bounds written bare:
/// a modified or higher-ranked bound (`?Sized`, `for<'a> Fn(&'a str)`)
/// passes through for rustc to judge.
fn plain_trait_bound(bound: &TypeParamBound) -> Option<&syn::Path> {
    match bound {
        TypeParamBound::Trait(t)
            if matches!(t.modifier, syn::TraitBoundModifier::None) && t.lifetimes.is_none() =>
        {
            Some(&t.path)
        }
        _ => None,
    }
}

/// A std trait recognized in the bounds list. Such a trait cannot be a
/// supertrait of a dyn-compatible trait, so instead of passing it through,
/// the macro generates proxy machinery that implements it for the shim's
/// trait objects.
#[derive(Clone, Copy, PartialEq)]
enum RecognizedBound {
    Clone,
    Hash,
}

impl RecognizedBound {
    /// The absolute path this bound adds to the blanket impl: the proxy
    /// methods need the implementor to actually be `Clone`/`Hash`.
    fn impl_bound(self) -> TokenStream2 {
        match self {
            RecognizedBound::Clone => quote! { ::std::clone::Clone },
            RecognizedBound::Hash => quote! { ::std::hash::Hash },
        }
    }

    /// The line added to the generated shim's docs naming the capability, so
    /// readers of a downstream crate's docs learn it without visiting this
    /// crate's.
    fn doc_line(self, shim: &Ident) -> String {
        match self {
            RecognizedBound::Clone => format!(
                "`Box<dyn {shim}>` implements [`Clone`], and `dyn {shim}` implements \
                 [`ToOwned`], both cloning the underlying concrete value."
            ),
            RecognizedBound::Hash => format!(
                "`dyn {shim}` implements [`Hash`], hashing like the underlying \
                 concrete value."
            ),
        }
    }

    /// The standalone shim trait (`::dyn_shim::DynClone` / `::dyn_shim::DynHash`)
    /// to add as a supertrait of a recognized-*bound* shim, so that shim's `dyn`
    /// type upcasts into the standalone one. Only when the matching crate
    /// feature is enabled (which is what defines the standalone trait); `None`
    /// otherwise, keeping the bound self-contained. Not applied to the standalone
    /// shims themselves, which are built through `expand_recognized`.
    fn dyn_supertrait(self) -> Option<TokenStream2> {
        match self {
            RecognizedBound::Clone if cfg!(feature = "dyn_clone") => {
                Some(quote! { ::dyn_shim::DynClone })
            }
            RecognizedBound::Hash if cfg!(feature = "dyn_hash") => {
                Some(quote! { ::dyn_shim::DynHash })
            }
            _ => None,
        }
    }

    /// The body of the `@mount` arm of this carrier's mount macro: the bridge
    /// impls that put the capability on `dyn $principal $($marker)*`, forwarding
    /// through the carrier the way `#[trait_object]` does. `$principal` and
    /// `$marker` are emitted as literal metavariables for the surrounding
    /// `macro_rules!` to bind; `carrier` is the carrier trait whose inherited
    /// method does the erased work (the shim this macro is generated for).
    ///
    /// `Clone` reconstructs `Box<dyn $principal>` from the concrete value with
    /// the fat-pointer splice in `__clone_box` (there is no blanket impl to clone
    /// through here, unlike a recognized bound). `Hash` forwards through the
    /// carrier's `__dyn_shim_hash`, which erases the generic hasher.
    fn mount_arm(self, carrier: &Ident) -> TokenStream2 {
        match self {
            RecognizedBound::Clone => quote! {
                impl ::std::clone::Clone for ::std::boxed::Box<dyn $principal $($marker)*> {
                    fn clone(&self) -> Self {
                        $crate::__clone_box(&**self)
                    }
                }
                impl ::std::borrow::ToOwned for dyn $principal $($marker)* {
                    type Owned = ::std::boxed::Box<dyn $principal $($marker)*>;
                    fn to_owned(&self) -> Self::Owned {
                        $crate::__clone_box(self)
                    }
                }
            },
            RecognizedBound::Hash => quote! {
                impl ::std::hash::Hash for dyn $principal $($marker)* {
                    fn hash<__H: ::std::hash::Hasher>(&self, state: &mut __H) {
                        <Self as #carrier>::__dyn_shim_hash(self, state)
                    }
                }
            },
        }
    }

    /// Generate the machinery for one recognized bound: hidden method
    /// signatures for the shim trait, their bodies for the blanket impl, and
    /// the standalone trait impls emitted after both.
    fn expand(
        self,
        shim: &Ident,
        combos: &[MarkerCombo],
    ) -> (TokenStream2, TokenStream2, TokenStream2) {
        match self {
            RecognizedBound::Clone => expand_clone(shim, combos),
            RecognizedBound::Hash => expand_hash(shim, combos),
        }
    }
}

/// A std auto trait recognized in the bounds list by its bare ident. A listed
/// auto trait passes through as a supertrait like any other bound; in
/// addition it selects which `dyn Shim + markers` types receive the machinery
/// for recognized bounds such as `Clone`, since only auto traits can follow
/// the principal trait in a trait object type.
#[derive(Clone, Copy, PartialEq)]
enum AutoTrait {
    Send,
    Sync,
    Unpin,
    UnwindSafe,
    RefUnwindSafe,
}

impl AutoTrait {
    /// The method-name suffix for a marker combination containing this trait.
    fn suffix(self) -> &'static str {
        match self {
            AutoTrait::Send => "send",
            AutoTrait::Sync => "sync",
            AutoTrait::Unpin => "unpin",
            AutoTrait::UnwindSafe => "unwind_safe",
            AutoTrait::RefUnwindSafe => "ref_unwind_safe",
        }
    }

    fn path(self) -> TokenStream2 {
        match self {
            AutoTrait::Send => quote! { ::std::marker::Send },
            AutoTrait::Sync => quote! { ::std::marker::Sync },
            AutoTrait::Unpin => quote! { ::std::marker::Unpin },
            AutoTrait::UnwindSafe => quote! { ::std::panic::UnwindSafe },
            AutoTrait::RefUnwindSafe => quote! { ::std::panic::RefUnwindSafe },
        }
    }
}

/// How one bound in the list is treated, decided by a single token match on
/// its bare name.
enum Classified {
    /// A std trait the macro implements for the shim's trait objects instead
    /// of passing it through as a supertrait (`Clone`, `Hash`).
    Recognized(RecognizedBound),
    /// An auto trait: passes through as a supertrait and additionally selects
    /// which `dyn Shim + markers` types get the recognized-bound machinery.
    Auto(AutoTrait),
    /// A std name recognized only to be rejected, carrying its targeted error
    /// message. Each would otherwise pass through as a supertrait and silently
    /// make the shim non-dyn-compatible (or is simply impossible), surfacing
    /// as a confusing rustc error far from the cause.
    Rejected(&'static str),
    /// Anything else: passed through to rustc as a supertrait.
    PassThrough,
}

impl Classified {
    /// Classify one bound by its bare name. Every recognized, auto, and rejected
    /// name lives in exactly one arm here, so the categories cannot overlap and no
    /// caller has to check them in a particular order. Like the literal `where
    /// Self: Sized` check on methods, this is a token match: trait resolution is
    /// unavailable during expansion, so a path-form bound (`std::clone::Clone`)
    /// is not classified (it falls through to `PassThrough`), and a user-defined
    /// trait that happens to share one of these names is treated as the std one.
    fn of(bound: &TypeParamBound) -> Classified {
        let Some(ident) = plain_trait_bound(bound).and_then(syn::Path::get_ident) else {
            return Classified::PassThrough;
        };
        match ident.to_string().as_str() {
            "Clone" => Classified::Recognized(RecognizedBound::Clone),
            "Hash" => Classified::Recognized(RecognizedBound::Hash),
            "Send" => Classified::Auto(AutoTrait::Send),
            "Sync" => Classified::Auto(AutoTrait::Sync),
            "Unpin" => Classified::Auto(AutoTrait::Unpin),
            "UnwindSafe" => Classified::Auto(AutoTrait::UnwindSafe),
            "RefUnwindSafe" => Classified::Auto(AutoTrait::RefUnwindSafe),
            "Copy" => Classified::Rejected(
                "trait objects are unsized and can never be `Copy` (use a `Clone` bound to make the shim's boxes cloneable)",
            ),
            "Sized" => Classified::Rejected(
                "trait objects are unsized, so the shim's `dyn` type can never be `Sized`",
            ),
            "Default" => Classified::Rejected(
                "`Default` has no `self` receiver and cannot be dispatched through a trait object (construct values as concrete types and box them)",
            ),
            "PartialEq" => Classified::Rejected(
                "`PartialEq` is not yet a recognized bound (cross-type equality on trait objects needs an `Any` downcast the macro does not generate)",
            ),
            "Eq" => Classified::Rejected(
                "`Eq` is not yet a recognized bound (cross-type equality on trait objects needs an `Any` downcast the macro does not generate)",
            ),
            "PartialOrd" => Classified::Rejected(
                "`PartialOrd` is not supported: the macro cannot define an order between different implementor types (sort with `sort_by_key` or implement the comparison traits for the shim's `dyn` type by hand)",
            ),
            "Ord" => Classified::Rejected(
                "`Ord` is not supported: the macro cannot define a total order between different implementor types (sort with `sort_by_key` or implement the comparison traits for the shim's `dyn` type by hand)",
            ),
            _ => Classified::PassThrough,
        }
    }
}

/// One `dyn Shim + markers` variant the recognized-bound machinery covers: a
/// method-name suffix and the `+ ...` tokens appended to the `dyn` type.
struct MarkerCombo {
    suffix: String,
    markers: TokenStream2,
}

impl MarkerCombo {
    /// Every subset of the listed auto traits, the plain (empty) combination
    /// first. The order markers are written in a `dyn` type does not affect type
    /// identity, so one combo per subset, each written in the order the auto
    /// traits were listed, covers every spelling at the use site. The count is
    /// `2^n` in the number of listed auto traits.
    fn all(autos: &[AutoTrait]) -> Vec<MarkerCombo> {
        (0..1usize << autos.len())
            .map(|mask| {
                let mut suffix = String::new();
                let mut markers = TokenStream2::new();
                for (i, auto) in autos.iter().enumerate() {
                    if mask & (1 << i) == 0 {
                        continue;
                    }
                    suffix.push('_');
                    suffix.push_str(auto.suffix());
                    let path = auto.path();
                    markers.extend(quote! { + #path });
                }
                MarkerCombo { suffix, markers }
            })
            .collect()
    }
}

/// The bridge impls that put `Clone` on a trait object: `Clone` for `Box<dyn
/// principal markers>` (the sized owner) and `ToOwned` for `dyn principal
/// markers` (for callers holding only a borrow), both cloning the concrete
/// value into a fresh `Box<dyn principal markers>`. `call` builds the cloning
/// expression from the receiver it is handed (`&**self` for `Clone`, `self` for
/// `ToOwned`). The two paths that emit these differ only in `call`: a recognized
/// bound forwards through a generated carrier method, while `trait_object` calls
/// `__clone_box`.
fn clone_bridge(
    principal: &Ident,
    markers: &TokenStream2,
    call: impl Fn(TokenStream2) -> TokenStream2,
) -> TokenStream2 {
    let boxed = ObjectForm::Boxed.ty(principal, markers);
    let bare = ObjectForm::Bare.ty(principal, markers);
    let clone_call = call(quote! { &**self });
    let to_owned_call = call(quote! { self });
    quote! {
        impl ::std::clone::Clone for #boxed {
            fn clone(&self) -> Self {
                #clone_call
            }
        }

        impl ::std::borrow::ToOwned for #bare {
            type Owned = #boxed;
            fn to_owned(&self) -> Self::Owned {
                #to_owned_call
            }
        }
    }
}

/// The bridge impl that puts `Hash` on a trait object: `Hash` for `dyn
/// principal markers`, which also covers `&dyn principal` and, through std's
/// `impl<T: ?Sized + Hash> Hash for Box<T>`, `Box<dyn principal>`. `carrier`
/// names the trait whose `__dyn_shim_hash` does the erased hashing: the shim
/// itself for a recognized bound (where that method is generated), or `DynHash`
/// for `trait_object` (inherited as a supertrait). It is named in a qualified
/// call so it stays unambiguous when the principal also inherits a same-named
/// method.
fn hash_bridge(principal: &Ident, markers: &TokenStream2, carrier: &TokenStream2) -> TokenStream2 {
    let bare = ObjectForm::Bare.ty(principal, markers);
    quote! {
        impl ::std::hash::Hash for #bare {
            fn hash<__H: ::std::hash::Hasher>(&self, state: &mut __H) {
                <Self as #carrier>::__dyn_shim_hash(self, state)
            }
        }
    }
}

/// Build one *boxing method*: a shim method that returns `Box<dyn shim
/// markers>` by boxing `call` (an expression of the implementor's own `Self`
/// type), bounded by `where Self: 'static markers`. `sig` carries the method's
/// name, receiver, and arguments; its return type and the `'static` bound are
/// filled in here. The `'static` bound licenses the `Box<__T>` to `Box<dyn
/// shim>` coercion in the blanket impl without restricting the shim's
/// implementors (it holds at every call site, since `Box<dyn shim>` is `+
/// 'static` by default), and unlike `Self: Sized` it does not exclude the method
/// from the vtable. The marker has to be re-attached here, where the concrete
/// type is still known: a value boxed as a plain `Box<dyn shim>` could never be
/// coerced back to `Box<dyn shim + Send>`.
///
/// This is the shared core of two boxings: the recognized `Clone` carrier's
/// `__dyn_shim_clone_box` (boxing `Clone::clone(self)`) and a
/// `#[dyn_shim(boxed)]` `-> Self` builder (boxing the forwarded source call),
/// the same way [`erase_generic`] is shared by the `Hash` carrier and
/// `#[dyn_shim(erase)]`.
fn build_boxed_method(
    shim: &Ident,
    markers: &TokenStream2,
    mut sig: Signature,
    call: TokenStream2,
) -> (Signature, TokenStream2) {
    sig.output = parse_quote! { -> ::std::boxed::Box<dyn #shim #markers> };
    sig.generics
        .make_where_clause()
        .predicates
        .push(parse_quote! { Self: 'static #markers });
    let body = quote! { ::std::boxed::Box::new(#call) };
    (sig, body)
}

/// Machinery for a recognized `Clone` bound: per marker combination, a hidden
/// boxing method cloning into a fresh box, and a `Clone` impl for that box
/// calling it. The boxing method is built through [`build_boxed_method`], the
/// same primitive a `#[dyn_shim(boxed)]` builder uses — `Clone` is just the
/// case whose boxed expression is `Clone::clone(self)`.
fn expand_clone(
    shim: &Ident,
    combos: &[MarkerCombo],
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let mut sigs = TokenStream2::new();
    let mut impls = TokenStream2::new();
    let mut after = TokenStream2::new();
    for MarkerCombo { suffix, markers } in combos {
        let method = format_ident!("__dyn_shim_clone_box{suffix}");
        let (sig, body) = build_boxed_method(
            shim,
            markers,
            parse_quote! { fn #method(&self) },
            quote! { ::std::clone::Clone::clone(self) },
        );
        sigs.extend(quote! {
            #[doc(hidden)]
            #sig ;
        });
        impls.extend(quote! {
            #sig { #body }
        });
        // `ToOwned` rides along with `Clone` (handled by `clone_bridge`), one
        // facade for callers who own a box and one for callers holding only
        // `&dyn Shim` (where `.clone()` would silently copy the reference).
        // Forward through `#shim`'s own hidden method, named in a qualified call:
        // when the shim also gains a `DynClone` supertrait (under the `dyn_clone`
        // feature) it inherits a same-named method, so a bare `self.#method()`
        // would be ambiguous.
        let on = ObjectForm::Bare.ty(shim, markers);
        after.extend(clone_bridge(shim, markers, |recv| {
            quote! { <#on as #shim>::#method(#recv) }
        }));
    }
    (sigs, impls, after)
}

/// Machinery for a recognized `Hash` bound: one hidden method erasing the
/// generic `H: Hasher` parameter to `&mut dyn Hasher` (lossless, since std
/// implements `Hasher` for `&mut H` where `H: Hasher + ?Sized`), and a
/// `Hash` impl on each `dyn` type calling it. Implementing on the `dyn`
/// types directly means std's `impl<T: ?Sized + Hash> Hash for Box<T>`
/// forwards for free and `&dyn Shim` is covered too. Hashing only reads, so
/// one hidden method serves every marker combination.
///
/// The hidden method's body *is* a generic-argument erasure: `Hash::hash`'s
/// `<H: Hasher>` used as `&mut H` lowers to `&mut dyn Hasher`. Rather than
/// spelling that out, the method is generated by running a synthetic
/// `fn(&self, state: &mut impl Hasher)` through [`erase_generic`], the same
/// transform `#[dyn_shim(erase)]` applies to a source method — so this carrier
/// is the first consumer of that shared substrate.
fn expand_hash(shim: &Ident, combos: &[MarkerCombo]) -> (TokenStream2, TokenStream2, TokenStream2) {
    let synthetic: TraitItemFn =
        parse_quote! { fn __dyn_shim_hash(&self, state: &mut impl ::std::hash::Hasher); };
    let erased = erase_generic(&synthetic.sig).expect("`Hash::hash` erases by construction");
    let sig = &erased.sig;
    let preamble = &erased.preamble;
    let args = &erased.args;
    let sigs = quote! {
        #[doc(hidden)]
        #sig ;
    };
    // Forward to `Hash::hash` rather than re-dispatching this hidden method:
    // the erased signature and reborrowed arguments are shared, only the call
    // target differs.
    let impls = quote! {
        #sig {
            #preamble
            <__T as ::std::hash::Hash>::hash(self #(, #args)*)
        }
    };
    // The carrier method `__dyn_shim_hash` is generated on the shim itself, so
    // the bridge forwards through `#shim`.
    let carrier = quote! { #shim };
    let mut after = TokenStream2::new();
    for MarkerCombo { markers, .. } in combos {
        after.extend(hash_bridge(shim, markers, &carrier));
    }
    (sigs, impls, after)
}

/// Generate a dyn-compatible shim for the annotated trait.
///
/// # Usage
///
/// ```
/// use dyn_shim::dyn_shim;
///
/// #[dyn_shim(DynFoo)]
/// trait Foo {
///     fn describe(&self) -> String;
///
///     fn make() -> Self;        // skipped: receiverless, not dyn-compatible
///
///     #[dyn_shim(skip)]
///     fn debug_only(&self) {}   // skipped: opted out
/// }
/// ```
///
/// The original trait is left in place. A new trait `DynFoo` is generated
/// alongside it, together with `impl<T: Foo> DynFoo for T`, so every
/// implementor of `Foo` is automatically a `DynFoo` and can be used as `dyn
/// DynFoo`. `DynFoo` inherits the source trait's visibility.
///
/// # Method Selection
///
/// A method is forwarded into the shim only if it can be dispatched through a
/// trait object. The criteria below approximate the language's [Dyn
/// Compatibility] rules per method. They catch the common reasons a method is
/// not callable on a `dyn` type, but do not reproduce the full rule set. A
/// method is **skipped** when any of the following holds:
///
/// - It has no `self` receiver (an associated function such as `fn new() -> Self`).
/// - It is `async`.
/// - It has a generic type or const parameter (lifetime parameters are fine).
/// - Its return type or any argument type mentions `Self`, or uses `impl Trait`.
/// - It requires `Self: Sized` (such a method is excluded from the vtable).
/// - It is annotated with `#[dyn_shim(skip)]`.
///
/// Some of these are opt-in remediable rather than skipped: a generic parameter
/// or `impl Trait` argument can be lowered with
/// [`#[dyn_shim(erase)]`](#erasing-a-generic-argument), and a `-> Self` builder
/// can be boxed with [`#[dyn_shim(boxed)]`](#reflexive-impl).
///
/// Skipped methods stay on the source trait and are reached on the concrete
/// type. A forwarded method keeps its entire signature — lifetimes, `where`
/// clause, parameter names, `unsafe`, and any explicit ABI — as well as its
/// attributes, so `#[doc]`, `#[must_use]`, `#[deprecated]`, and `#[cfg]`
/// behave the same on the shim as on the source trait. A by-value
/// `self` receiver is rewritten to `self: Box<Self>` and forwarded by
/// dereferencing the box. A dispatchable receiver (`&self`, `&mut self`, or an
/// explicit `self: Box<Self>`, `Rc<Self>`, `Arc<Self>`, or `Pin<_>`) is
/// forwarded unchanged.
///
/// ## Erasing a generic argument
///
/// A method that is non-dyn-compatible *only* because of a generic parameter or
/// argument-position `impl Trait` can be kept rather than skipped, by annotating
/// it `#[dyn_shim(erase)]`. Each such parameter must be bounded by a single
/// trait and used only behind a `&` or `&mut`; the shim then lowers it to a
/// trait object (`&mut impl Write` becomes `&mut dyn Write`), and forwarding
/// reborrows the argument so the source method's parameter re-infers to the
/// sized reference type. This is the same erasure the recognized `Hash` bound
/// applies to `Hash::hash`'s generic hasher.
///
/// ```
/// use dyn_shim::dyn_shim;
/// use std::io::Write;
///
/// #[dyn_shim(DynLog)]
/// trait Log {
///     #[dyn_shim(erase)]
///     fn write_to(&self, out: &mut impl Write); // shim: `out: &mut dyn Write`
/// }
///
/// // `Box<dyn DynLog>` can call `write_to`, dispatched through the vtable.
/// ```
///
/// It is sound exactly when the bound provides `impl Bound for &mut (dyn Bound)`
/// (or `&(dyn Bound)`), as std does for `Hasher`, `Write`, `Read`, and others;
/// a bound without that impl fails to compile at the forwarding call. A
/// parameter used by value, in the return type, or in more than one argument
/// cannot be erased, and the macro reports it at the method.
///
/// # Reflexive Impl
///
/// By default the shim is a distinct trait, so a `Box<dyn DynFoo>` is not a
/// `Foo`. The optional `reflexive` argument additionally emits an impl of the
/// source trait for the shim's trait object, so the erased value satisfies the
/// source trait itself and can be passed to code written against `Foo`:
///
/// - `reflexive = boxed` emits `impl Foo for Box<dyn DynFoo>`. `Self` is the
///   sized `Box<dyn DynFoo>`, so by-value `self` and `-> Self` methods work.
/// - `reflexive = bare` emits `impl Foo for dyn DynFoo`, letting a `&dyn DynFoo`
///   stand in for an `&impl Foo`. `Self` is the unsized `dyn DynFoo`, so a
///   by-value `self` receiver, a `-> Self`, or a by-value `Self` argument cannot
///   be expressed; use `reflexive = boxed` for a trait that has those.
/// - `reflexive = bare + boxed` emits both impls, so a borrow *and* an owned box
///   satisfy `Foo`. It is only available when the bare impl is (no by-value
///   `Self`); otherwise the macro rejects it toward `boxed`.
///
/// The two kinds are complementary: a `bare` impl gives `&dyn DynFoo: Foo`, and
/// a `boxed` impl gives `Box<dyn DynFoo>: Foo`. Neither implies the other for a
/// general trait, so request both when the erased value is consumed both ways.
///
/// The impl must account for every method of the source trait. A dyn-compatible
/// method forwards through the shim. A method that is not dyn-compatible (see
/// [Method Selection](#method-selection)) cannot forward; the remediations, from
/// most to least preferable, are:
///
/// - **Make it dispatch** — `#[dyn_shim(erase)]` (generic argument) or
///   `#[dyn_shim(boxed)]` (`-> Self` builder, see below) turn it into a real
///   forwarding rather than a stub.
/// - **Inherit a default body** on the source trait, which the impl reuses.
/// - **`where Self: Sized`** — on a `reflexive = bare` impl such a method is left
///   out entirely (it is not part of the unsized object's surface), so calling it
///   on a `&dyn DynFoo` is a compile error rather than a runtime panic. (The
///   boxed object is `Sized`, so a `boxed` impl still needs one of the others.)
/// - **A fallback body** for the erased impl: `#[dyn_shim(stub = <expr>)]`
///   evaluates `<expr>` (e.g. `None`, `Default::default()`), letting the method
///   degrade to a value; `#[dyn_shim(panic)]` is the special case that panics
///   with a generated message. Unlike a default body, a stub leaves the source
///   trait's method required of concrete implementors.
///
/// A method with none of these is a compile error naming it.
///
/// The `boxed` remediation, in detail: the shim method returns `Box<dyn DynFoo>`
/// (the concrete result, boxed and unsized to the shim object), and the
/// `reflexive = boxed` impl forwards through it, since there `Self` *is* `Box<dyn
/// DynFoo>`. So a consuming or chaining builder keeps working on an erased value
/// instead of panicking. This is the general form of the boxing the recognized
/// `Clone` bound applies to `clone`; it requires `reflexive = boxed` (a `bare`
/// impl has an unsized, unconstructible `Self`) and does not yet support
/// auto-trait markers on the shim.
///
/// ```
/// use dyn_shim::dyn_shim;
///
/// #[dyn_shim(DynMunch, reflexive = boxed)]
/// trait Munch {
///     fn crunch(self) -> u32;   // by-value self: forwarded
///     #[dyn_shim(panic)]
///     fn fresh() -> Self;       // receiverless: panicking stub
/// }
///
/// struct Apple(u32);
/// impl Munch for Apple {
///     fn crunch(self) -> u32 { self.0 }
///     fn fresh() -> Self { Apple(1) }
/// }
///
/// fn eat(m: impl Munch) -> u32 { m.crunch() }
///
/// // Box<dyn DynMunch> is a Munch, so it can be passed to code expecting one.
/// let m: Box<dyn DynMunch> = Box::new(Apple(7));
/// assert_eq!(eat(m), 7);
/// ```
///
/// The `reflexive` argument and the `#[dyn_shim(panic)]` / `#[dyn_shim(boxed)]`
/// helpers work the same on [`macro@dyn_shim_foreign`].
///
/// # Bounds
///
/// The generated shim has no supertraits by default — not even the source
/// trait's. Optional bounds after the shim's name, written like a trait's
/// supertrait list, are added as supertraits of the shim and as bounds on the
/// blanket impl:
///
/// ```
/// use dyn_shim::dyn_shim;
///
/// #[dyn_shim(DynJob: Send + Sync)]
/// trait Job {
///     fn run(&self) -> u32;
/// }
///
/// struct Sleep;
/// impl Job for Sleep {
///     fn run(&self) -> u32 { 1 }
/// }
///
/// let job: Box<dyn DynJob> = Box::new(Sleep);
/// assert_eq!(std::thread::spawn(move || job.run()).join().unwrap(), 1);
/// ```
///
/// This is also the way to re-add a supertrait of the source trait, making its
/// methods callable on the shim's `dyn` type (`DynShim: std::fmt::Display`,
/// for example).
///
/// The shim's bounds should generally mirror the source trait's supertraits,
/// keeping the shim a faithful dyn-compatible view of the source. Auto
/// traits are the common exception: a `Send` or `Sync` bound describes
/// implementors rather than the trait's contract, so it usually appears only
/// on the shim, as above.
///
/// A bound the source trait does not require deserves a warning, because it
/// does not behave like the supertrait it is spelled as:
///
/// ```
/// use dyn_shim::dyn_shim;
///
/// #[dyn_shim(DynFoo: Iterator)]
/// trait Foo {
///     fn describe(&self) -> String;
/// }
///
/// // Implements Foo, but not Iterator.
/// struct Bar;
/// impl Foo for Bar {
///     fn describe(&self) -> String { "bar".into() }
/// }
///
/// // Compiles: implementing Foo carries no Iterator obligation. Bar just
/// // silently never receives the DynFoo blanket impl, so this would fail:
/// // let b: Box<dyn DynFoo<Item = u8>> = Box::new(Bar);
/// ```
///
/// Had `trait DynFoo: Iterator` been written by hand, each
/// `impl DynFoo for ...` would be checked against the supertrait, making an
/// implementor without `Iterator` an error at its `impl`. But nobody writes
/// impls of the shim. There is only the blanket impl, and a bound there is a
/// filter, not an obligation: `Bar` implements `Foo` fine, never becomes a
/// `DynFoo`, and errors only where it is used as `Box<dyn DynFoo>`, which
/// may be far from the mistake, or nowhere. Mirroring the bound as a
/// supertrait of the source (`trait Foo: Iterator`) restores the immediate
/// per-impl check.
///
/// **The macro cannot classify the listed bounds.** Trait resolution is
/// unavailable during macro expansion, so whether a named trait is
/// dyn-compatible cannot be decided there. Auto traits such as `Send` and
/// `Sync`, lifetimes such as `'static`, and dyn-compatible traits pass
/// through and work as supertraits. A few std names are handled specially by
/// token match: [recognized](#recognized-bounds) ones get generated
/// machinery, and [known-impossible](#rejected-bounds) ones get a targeted
/// error. Anything else that breaks the shim (a non-dyn-compatible user
/// trait, a path-form `std::clone::Clone`) makes the shim non-dyn-compatible
/// too; rustc reports that at the first place `dyn Shim` is written, not at
/// the attribute. An implementor of the source trait that does not satisfy
/// the bounds does not receive the shim impl.
///
/// ## Recognized Bounds
///
/// `Clone` and `Hash` are exceptions to the rule above. Neither can be a
/// supertrait of a dyn-compatible trait, so a literal `Clone` or `Hash` in
/// the bounds list is intercepted instead of passed through. It still bounds
/// the blanket impl (an implementor that is not `Clone` does not receive the
/// shim), and instead of a supertrait the macro generates the machinery that
/// implements the trait for the shim's trait objects:
///
/// ```
/// use dyn_shim::dyn_shim;
/// use std::hash::{DefaultHasher, Hash, Hasher};
///
/// #[dyn_shim(DynShape: Clone + Hash)]
/// trait Shape {
///     fn area(&self) -> u32;
/// }
///
/// #[derive(Clone, Hash)]
/// struct Rect(u32, u32);
/// impl Shape for Rect {
///     fn area(&self) -> u32 { self.0 * self.1 }
/// }
///
/// let shapes: Vec<Box<dyn DynShape>> = vec![Box::new(Rect(2, 3))];
/// let copy = shapes.clone(); // Box<dyn DynShape>: Clone
/// assert_eq!(copy[0].area(), 6);
///
/// let mut hasher = DefaultHasher::new();
/// shapes[0].hash(&mut hasher); // Box<dyn DynShape>: Hash
///
/// let borrowed: &dyn DynShape = &Rect(2, 3);
/// let owned: Box<dyn DynShape> = borrowed.to_owned(); // dyn DynShape: ToOwned
/// assert_eq!(owned.area(), 6);
/// ```
///
/// `Clone` is implemented for `Box<dyn Shim>`. `Hash` is implemented for
/// `dyn Shim` itself, which also covers `&dyn Shim` and, through std's
/// forwarding impl, `Box<dyn Shim>`. Cloning requires `'static` contents:
/// `Box<dyn Shim + 'a>` does not get `Clone`.
///
/// With the crate's `dyn_clone` (or `dyn_hash`) feature enabled, a recognized `Clone`
/// (or `Hash`) bound additionally makes the shim a subtrait of the standalone
/// `dyn_shim::DynClone` (or `dyn_shim::DynHash`). The shim's `dyn` type then
/// upcasts into the standalone one, so a `Box<dyn Shim>` flows into an API typed
/// against `Box<dyn DynClone>` (and a `&dyn Shim` into `&dyn DynHash`). Without
/// the feature the bound is self-contained and adds no such supertrait.
///
/// A recognized `Clone` also implements `ToOwned` for the `dyn` type. The
/// two are facades over the same machinery serving different callers:
/// `Clone` duplicates a box you already own, while `to_owned` lets a caller
/// holding only a borrowed `&dyn Shim` escape the borrow with an owned copy.
/// That borrowed edge is otherwise a footgun, since `&T` is itself `Clone`:
/// `shape_ref.clone()` compiles and silently copies the reference, not the
/// value. `ToOwned` is also what `Cow<'_, dyn Shim>` requires, enabling APIs
/// that pass borrowed values through untouched and allocate only on the
/// owning path.
///
/// Auto traits listed in the bounds select which marker types are covered as
/// well. For each subset of the listed auto traits (`Send`, `Sync`, `Unpin`,
/// `UnwindSafe`, and `RefUnwindSafe` are recognized), the trait is also
/// implemented for `Box<dyn Shim + markers>` (`dyn Shim + markers` for
/// `Hash`): `Clone + Send` covers `Box<dyn Shim>` and `Box<dyn Shim + Send>`.
/// A listed auto trait otherwise behaves as before, becoming a supertrait of
/// the shim and a bound on the blanket impl. Position in the bounds list
/// never matters, nor does marker order at the use site (`Box<dyn Send +
/// Shim>` is the same type). The number of generated impls doubles with each
/// listed auto trait.
///
/// Like the literal `where Self: Sized` check on methods, recognition is a
/// token match on the bare name: a path-form `std::clone::Clone` is passed
/// through as a supertrait (breaking the shim's dyn-compatibility), and a
/// user-defined trait named `Clone` is intercepted. Trait resolution is
/// unavailable during expansion, so the macro cannot see what a name is
/// imported as; the bare ident is all it has.
///
/// The same applies to the auto traits that select marker combinations. Only
/// a bare `Send` (or `Sync`, `Unpin`, `UnwindSafe`, `RefUnwindSafe`) is added
/// to the covered subsets. A path-form `std::marker::Send` still passes
/// through as a supertrait, so the bound itself compiles, but it is left out
/// of the marker machinery, so `Box<dyn Shim + Send>` never receives the
/// `Clone` or `Hash` impl. The miss surfaces as a trait-bound error at the
/// `Box<dyn Shim + Send>` use site, not at the attribute. Write auto traits
/// bare when they should drive the marker combinations.
///
/// The generated machinery names the shim's `dyn` type bare (`Box<dyn
/// Shim>`), which requires every associated type of every supertrait to be
/// fixed. So a recognized bound combines with a bound whose trait has
/// associated types (such as `Iterator`) only when the bounds list binds
/// them: `Clone + Iterator<Item = u8>` works, while `Clone + Iterator` does
/// not, since an unbound `Item` could only be supplied at a use site, which
/// the generated impls never see.
///
/// ## Rejected Bounds
///
/// Some std names are recognized only to be rejected with a targeted error,
/// because no machinery could make them work: `Copy` and `Sized` contradict
/// being a trait object, and `Default` has no receiver for a vtable to
/// dispatch on. The comparison traits are rejected too: `PartialEq` and `Eq`
/// need an `Any` downcast the macro does not generate yet, and `PartialOrd`
/// and `Ord` would make the macro invent an order between unrelated concrete
/// types, which is not its call. Sort with `sort_by_key`, or implement the
/// comparison traits on the `dyn` type by hand: the crate invoking the macro
/// owns the shim trait, so coherence permits `impl Ord for dyn Shim` there
/// (std's forwarding impls carry it onto the boxes), and the generated
/// machinery stays out of the way.
///
/// Rejection is a bare-name token match, with the same blind spot as
/// recognition: the macro cannot see what a name resolves to. A user-defined
/// trait that happens to be named `Ord` (or any of the rejected names) is
/// caught too, and reported with the message written for the std trait, even
/// when that trait is dyn-compatible and would work as a supertrait. Write it
/// path-qualified (`self::Ord`, `crate::cmp::Ord`) to pass it through: a
/// multi-segment path is not a bare ident, so it skips the rejection list and
/// becomes an ordinary supertrait.
///
/// ## Bounds That Need No Entry
///
/// A dyn-compatible trait works as a plain pass-through bound; nothing needs
/// recognizing. This covers, among many others, `Debug`, `Display`,
/// `std::error::Error`, the auto traits, `AsRef<T>` and `Borrow<T>`,
/// `std::io::Read`/`Write`/`Seek`, `Iterator`, and `Future`. The last two
/// carry associated types, which need one extra step; see
/// [Bounds With Associated Types](#bounds-with-associated-types).
///
/// `Any` is worth singling out: trait object upcasting is built into the
/// language, so an `Any` bound already enables downcasting with no generated
/// machinery:
///
/// ```
/// use dyn_shim::dyn_shim;
/// use std::any::Any;
///
/// #[dyn_shim(DynShape: Any)]
/// trait Shape {
///     fn area(&self) -> u32;
/// }
///
/// struct Rect(u32, u32);
/// impl Shape for Rect {
///     fn area(&self) -> u32 { self.0 * self.1 }
/// }
///
/// let shape: Box<dyn DynShape> = Box::new(Rect(2, 3));
/// let any: &dyn Any = &*shape; // upcasting coercion
/// assert!(any.downcast_ref::<Rect>().is_some());
/// ```
///
/// ## Bounds With Associated Types
///
/// A bound whose trait has associated types, such as `Iterator` or
/// `Future`, passes through like any other dyn-compatible trait, but the
/// associated types must be bound before the shim's `dyn` type can be
/// written. There are two places to bind them, and they trade against each
/// other:
///
/// - **At the use site.** The bounds list names the bare trait
///   (`DynSamples: Iterator`), and every spot that writes the `dyn` type
///   supplies the bindings: `Box<dyn DynSamples<Item = u8>>`. One shim
///   serves every item type, each collection picking its own binding, but
///   the bare `dyn DynSamples` is not a nameable type (forgetting the
///   binding is a "must be specified" error at that spot).
/// - **In the bounds list.** `DynSamples: Iterator<Item = u8>` fixes the
///   associated type for every implementor, so the bare `dyn DynSamples`
///   is nameable. Only implementors with exactly that item type receive
///   the shim, and this is the only form that combines with
///   [recognized bounds](#recognized-bounds), whose generated machinery
///   must name the `dyn` type bare.
///
/// ```
/// use dyn_shim::dyn_shim;
///
/// #[dyn_shim(DynSamples: Iterator)]
/// trait Samples: Iterator {
///     fn label(&self) -> String;
/// }
///
/// struct Ramp(u8);
/// impl Iterator for Ramp {
///     type Item = u8;
///     fn next(&mut self) -> Option<u8> {
///         self.0 += 1;
///         Some(self.0)
///     }
/// }
/// impl Samples for Ramp {
///     fn label(&self) -> String { "ramp".into() }
/// }
///
/// let mut source: Box<dyn DynSamples<Item = u8>> = Box::new(Ramp(0));
/// assert_eq!(source.label(), "ramp");
/// let head: Vec<u8> = source.by_ref().take(3).collect();
/// assert_eq!(head, [1, 2, 3]);
/// ```
///
/// # Limitations
///
/// A skipped method (see [Method Selection](#method-selection)) is not a
/// limitation of this macro: it cannot be dispatched through any trait object,
/// so no shim could forward it. The limitations specific to this macro are:
///
/// - **The source trait may not be generic.** A trait with type, const, or
///   lifetime parameters is rejected with a compile error. Such a trait can
///   still be dyn-compatible on its own (`dyn Trait<i32>`); the macro just does
///   not generate a parameterized shim for it.
/// - **Supertraits are not inherited.** The macro cannot tell whether a
///   supertrait is dyn-compatible, so it carries none of them onto the shim and
///   their methods are not callable on the shim's `dyn` type. Re-add the ones
///   you need — and know to be dyn-compatible — as [bounds on the shim's
///   name](#bounds).
/// - **Only a literal `where Self: Sized` bound is recognized.** Classifying any
///   other `Self:` bound would need trait resolution, which is unavailable during
///   macro expansion, so such a method is forwarded as written. This is correct
///   for an auto-trait bound like `Self: Send` (call it through `&(dyn Shim +
///   Send)`), but a `Self: Clone` bound produces a method that cannot be called
///   on the shim's `dyn` type, and a `Self: Debug` bound produces a shim that
///   does not compile. Annotate such a method with `#[dyn_shim(skip)]`.
///
/// [Dyn Compatibility]: https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility
///
/// # Example
///
/// ```
/// use dyn_shim::dyn_shim;
///
/// #[dyn_shim(DynSink)]
/// trait Sink {
///     fn connect() -> Self;                 // skipped: receiverless
///     fn write(&mut self, line: &str);
///     fn total(&self) -> usize;
///     fn finish(self) -> usize;             // by-value -> self: Box<Self>
///     #[dyn_shim(skip)]
///     fn debug_only(&self) {}               // skipped: opted out
/// }
///
/// #[derive(Default)]
/// struct Buf { lines: usize }
/// impl Sink for Buf {
///     fn connect() -> Self { Buf::default() }
///     fn write(&mut self, _line: &str) { self.lines += 1; }
///     fn total(&self) -> usize { self.lines }
///     fn finish(self) -> usize { self.lines }
/// }
///
/// let mut s: Box<dyn DynSink> = Box::new(Buf::connect());
/// s.write("a");
/// s.write("b");
/// assert_eq!(s.total(), 2);
/// assert_eq!(s.finish(), 2);
/// ```
#[proc_macro_attribute]
pub fn dyn_shim(attr: TokenStream, item: TokenStream) -> TokenStream {
    let Args {
        shim_name,
        bounds,
        reflexive,
    } = parse_macro_input!(attr as Args);
    let input = parse_macro_input!(item as ItemTrait);
    // The source trait is local: refer to it by its own name, and re-emit it.
    let source_ref = input.ident.to_token_stream();
    let source_doc = input.ident.to_string();
    expand(
        shim_name,
        bounds,
        &input,
        &source_ref,
        &source_doc,
        true,
        reflexive,
    )
}

/// Generate a dyn-compatible shim for a trait defined in another crate.
///
/// `#[dyn_shim]` must sit on the trait's own definition, so it cannot target a
/// trait you do not own. This attribute fills that gap. Its sole argument is the
/// path to the foreign source trait, and the trait it is written on *is* the
/// shim: its name names the shim, its supertrait list supplies the bounds, and
/// its body restates the methods to forward.
///
/// ```
/// use dyn_shim::dyn_shim_foreign;
///
/// // Stands in for a trait defined in a dependency.
/// mod other_crate {
///     pub trait Sink {
///         fn write(&mut self, line: &str);
///         fn total(&self) -> usize;
///         fn finish(self) -> usize;
///     }
/// }
///
/// #[dyn_shim_foreign(other_crate::Sink)]
/// trait DynSink {
///     fn write(&mut self, line: &str);
///     fn total(&self) -> usize;
///     fn finish(self) -> usize; // by-value -> self: Box<Self>
/// }
///
/// struct Buf(usize);
/// impl other_crate::Sink for Buf {
///     fn write(&mut self, _line: &str) { self.0 += 1; }
///     fn total(&self) -> usize { self.0 }
///     fn finish(self) -> usize { self.0 }
/// }
///
/// let mut s: Box<dyn DynSink> = Box::new(Buf(0));
/// s.write("a");
/// assert_eq!(s.total(), 1);
/// assert_eq!(s.finish(), 1);
/// ```
///
/// # How It Differs From [`macro@dyn_shim`]
///
/// `#[dyn_shim]` reads a source trait and emits a *second*, shim trait beside
/// it. `#[dyn_shim_foreign]` has no source trait to read — it lives in another
/// crate — so the annotated trait is the shim directly: it is consumed and
/// re-emitted with the forwarding machinery filled in, rather than copied. The
/// blanket impl forwards to the foreign path
/// (`impl<T: other_crate::Sink> DynSink for T`), which coherence permits: the
/// shim trait is local, so a blanket impl of it is allowed however foreign the
/// source trait is, and the recognized-bound machinery lands on the local
/// `dyn` types. The shim's name, visibility, supertrait list, and method
/// selection all read off the annotated trait, so [method selection], [bounds],
/// and [recognized bounds] work exactly as for [`macro@dyn_shim`] — a `Clone`
/// or `Hash` in the supertrait list is recognized, auto traits pass through and
/// select marker combinations, and so on.
///
/// One thing follows from the source trait being foreign: **the signatures must
/// be restated by hand.** A proc macro sees only its own input tokens, never
/// another crate's AST, so it cannot read the foreign trait's methods. List the
/// dyn-compatible ones you want forwarded; omit the rest (a receiverless
/// `fn build() -> Self` simply has no place in the shim anyway). A restated
/// signature that does not match the real one is caught when the generated
/// `<T as other_crate::Sink>::method(..)` call fails to compile.
///
/// # Provided methods
///
/// A restated method with a **default body** is not forwarded: it is provided by
/// the shim itself, emitted verbatim, with its body calling the shim's forwarded
/// methods. Use it to add a convenience method that the foreign trait does not
/// declare — the macro generates no `<T as Source>::method` call for it, so the
/// foreign trait needs no counterpart:
///
/// ```
/// use dyn_shim::dyn_shim_foreign;
///
/// mod other_crate {
///     pub trait Sink {
///         fn total(&self) -> usize;
///     }
/// }
///
/// #[dyn_shim_foreign(other_crate::Sink)]
/// trait DynSink {
///     fn total(&self) -> usize;       // forwarded to `other_crate::Sink::total`
///     fn doubled(&self) -> usize {    // provided: not on `Sink`, computed here
///         self.total() * 2
///     }
/// }
///
/// struct Buf(usize);
/// impl other_crate::Sink for Buf {
///     fn total(&self) -> usize { self.0 }
/// }
///
/// let s: Box<dyn DynSink> = Box::new(Buf(4));
/// assert_eq!(s.doubled(), 8);
/// ```
///
/// (In [`macro@dyn_shim`] a default body instead lives on the re-emitted source
/// trait and is forwarded like any other method, so an implementor's override is
/// still honored; only the foreign form, which has no source trait to hold it,
/// provides the body on the shim.)
///
/// The trailing `reflexive = bare | boxed` argument
/// (`#[dyn_shim_foreign(other_crate::Sink, reflexive = boxed)]`) and the method
/// helpers (`#[dyn_shim(panic)]`, `#[dyn_shim(stub = ...)]`, `erase`, `boxed`)
/// work here too, emitting `impl other_crate::Sink for Box<dyn DynSink>` so the
/// boxed shim satisfies the foreign trait; a provided method is left out of that
/// impl, since it is not a method of the foreign trait. See [reflexive
/// impl](macro@dyn_shim#reflexive-impl).
///
/// [method selection]: macro@dyn_shim#method-selection
/// [bounds]: macro@dyn_shim#bounds
/// [recognized bounds]: macro@dyn_shim#recognized-bounds
#[proc_macro_attribute]
pub fn dyn_shim_foreign(attr: TokenStream, item: TokenStream) -> TokenStream {
    // The first argument is the foreign source trait's path; everything else is
    // read off the annotated trait, which is itself the shim.
    let ForeignArgs { source, reflexive } = parse_macro_input!(attr as ForeignArgs);
    let input = parse_macro_input!(item as ItemTrait);
    let source_ref = source.to_token_stream();
    let source_doc = path_doc_string(&source);
    // No source trait to re-emit: the annotated trait is the shim, regenerated
    // from its own name, supertraits, and restated signatures.
    let shim_name = input.ident.clone();
    let bounds = input.supertraits.clone();
    expand(
        shim_name,
        bounds,
        &input,
        &source_ref,
        &source_doc,
        false,
        reflexive,
    )
}

/// Expose a recognized std trait as a standalone dyn-compatible shim.
///
/// `Clone` and `Hash` cannot be supertraits of a dyn-compatible trait, so they
/// cannot be shimmed by restating them through [`macro@dyn_shim_foreign`]: the
/// dyn-compatible form is not a subset of their methods but a transform of them
/// (erasing `Clone::clone`'s `-> Self` into a boxing clone, and `Hash::hash`'s
/// generic `H: Hasher` into `&mut dyn Hasher`). That transform is built into the
/// macro, so this attribute needs only the trait name; the shim it is written
/// on supplies the shim's name and visibility and must have an empty body.
///
/// ```
/// use dyn_shim::dyn_shim_recognized;
///
/// #[dyn_shim_recognized(Clone)]
/// trait DynClone {}
///
/// #[derive(Clone)]
/// struct Widget(u32);
///
/// // No impl of `DynClone` is written: `impl<T: Clone> DynClone for T` is
/// // generated, and `Box<dyn DynClone>` is itself `Clone`.
/// let a: Box<dyn DynClone> = Box::new(Widget(7));
/// let _b = a.clone();
/// ```
///
/// The result mirrors the `dyn_clone` and `dyn_hash` crates: `Box<dyn DynClone>`
/// implements `Clone` (and `dyn DynClone` implements `ToOwned`), and `dyn
/// DynHash` implements `Hash` (covering `Box<dyn DynHash>` through std's
/// forwarding impl). It is the same machinery a recognized [bound] generates on
/// a host shim, with the recognized trait as the principal instead.
///
/// Auto-trait markers listed after the trait select which `dyn` variants are
/// covered, exactly as in the bound form: `#[dyn_shim_recognized(Clone + Send)]`
/// makes both `Box<dyn DynClone>` and `Box<dyn DynClone + Send>` cloneable. The
/// markers are not supertraits of the shim, so they do not constrain its
/// implementors; only the marked `dyn` variant's machinery requires them.
///
/// [bound]: macro@dyn_shim#recognized-bounds
#[proc_macro_attribute]
pub fn dyn_shim_recognized(attr: TokenStream, item: TokenStream) -> TokenStream {
    let BoundList { bounds } = parse_macro_input!(attr as BoundList);
    let input = parse_macro_input!(item as ItemTrait);
    expand_recognized(&input, bounds)
}

/// Mount a *carrier* onto the trait objects of a dyn-compatible trait you
/// already own, so `dyn YourTrait` satisfies a trait it cannot carry as a
/// supertrait — without generating a shim.
///
/// [`macro@dyn_shim`] and [`macro@dyn_shim_foreign`] build a *new*
/// dyn-compatible trait from one that is not. This attribute is the other half:
/// the trait is already dyn-compatible, and you want its `dyn` objects to also
/// satisfy some target trait. It generates no trait and no blanket impl — it
/// re-emits the annotated trait untouched and invokes each listed carrier's
/// mount macro, which stamps the `impl Target for dyn YourTrait` blocks.
///
/// A **carrier** is a trait the annotated trait inherits whose mount macro knows
/// how to implement a target on a `dyn` type. Two kinds exist, reached the same
/// way:
///
/// - The shipped [`DynClone`] / [`DynHash`] (behind the `dyn_clone` / `dyn_hash`
///   features). Their target is `Clone` / `Hash`, which cannot be a supertrait
///   of a dyn-compatible trait. `#[trait_object(DynClone)]` makes `Box<dyn
///   YourTrait>` cloneable (and `dyn YourTrait` `ToOwned`); `#[trait_object(
///   DynHash)]` makes `dyn YourTrait` (and so `&dyn` and `Box<dyn>`) hashable.
/// - Any [`macro@dyn_shim`] shim `DynFoo`. Its target is the source trait `Foo`,
///   which is not dyn-compatible. `#[trait_object(DynFoo)]` makes `dyn YourTrait`
///   and `Box<dyn YourTrait>` satisfy `Foo`, forwarding the dyn-compatible
///   methods through the shim. This is how a non-dyn-compatible `Foo` reaches a
///   trait object that could never list it as a supertrait: inherit `DynFoo`
///   instead, then mount `Foo` back on.
///
/// The carrier is written as an explicit supertrait, so a reader sees that every
/// implementor must satisfy it:
///
/// ```
/// use dyn_shim::{DynHash, trait_object};
/// use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
///
/// #[trait_object(DynHash)]
/// trait Event: DynHash {
///     fn name(&self) -> &str;
/// }
///
/// #[derive(Hash)]
/// struct Tick(u64);
/// impl Event for Tick {
///     fn name(&self) -> &str { "tick" }
/// }
///
/// // `dyn Event` (and so `&dyn Event` and `Box<dyn Event>`) implements `Hash`.
/// let bh = BuildHasherDefault::<DefaultHasher>::default();
/// let boxed: Box<dyn Event> = Box::new(Tick(7));
/// assert_eq!(bh.hash_one(&*boxed), bh.hash_one(&Tick(7)));
/// ```
///
/// Several carriers may be combined (`#[trait_object(DynHash + DynClone)]`,
/// `#[trait_object(DynFoo + DynClone)]`), and auto-trait markers select the
/// covered `dyn` variants exactly as for a [recognized
/// bound](macro@dyn_shim#recognized-bounds): `#[trait_object(DynClone + Send)]`
/// makes both `Box<dyn YourTrait>` and `Box<dyn YourTrait + Send>` cloneable.
///
/// # The carrier is a supertrait, so the contract is strict
///
/// `#[dyn_shim(DynFoo: Hash)]` generates a separate `DynFoo` shim and bounds its
/// blanket impl by `Hash`, so a non-`Hash` implementor of `Foo` simply never
/// becomes a `DynFoo`. `#[trait_object(...)]` adds no shim and instead requires
/// the carrier as a supertrait of the annotated trait itself, so the contract is
/// stricter: *every* implementor must satisfy the carrier. Reach for this
/// attribute when the annotated trait is the `dyn` type you use directly; reach
/// for a recognized bound on a shim when only some implementors qualify.
///
/// # Relation to [`reflexive`](macro@dyn_shim#reflexive-impl)
///
/// Mounting a shim carrier (`#[trait_object(DynFoo)]`) and a `reflexive` impl are
/// the same operation through the same generated macro: both stamp `impl Foo for
/// <object>`. `reflexive` mounts onto the shim's *own* objects (`dyn DynFoo`,
/// `Box<dyn DynFoo>`) at the shim's definition; `#[trait_object(DynFoo)]` mounts
/// onto a *different* principal that inherits the shim, anywhere `DynFoo` is in
/// scope (including another crate).
///
/// The carrier is matched by the last segment of its path (`DynClone`,
/// `DynHash`, `DynFoo`), the same token-match convention as the rest of the
/// crate: a missing carrier supertrait is reported at the attribute. The carrier
/// must be in scope as a macro at the use site, which its own import provides
/// (`use krate::DynFoo` brings the trait and its mount macro together).
#[proc_macro_attribute]
pub fn trait_object(attr: TokenStream, item: TokenStream) -> TokenStream {
    let BoundList { bounds } = parse_macro_input!(attr as BoundList);
    let input = parse_macro_input!(item as ItemTrait);
    expand_trait_object(&input, bounds)
}

/// If the annotated trait has any generic parameters, return the compile error
/// to emit. Every entry point builds non-parameterized output (one shim or one
/// set of `dyn` impls), so a type, const, or lifetime parameter has nowhere to
/// go. `message` is the full error text, naming the macro and what it cannot be
/// generic over.
fn reject_generics(input: &ItemTrait, message: &str) -> Option<TokenStream> {
    input.generics.params.first().map(|param| {
        syn::Error::new_spanned(param, message)
            .to_compile_error()
            .into()
    })
}

/// The attribute's bound list split by [`Classified::of`], with each caller left to
/// apply its own policy. `recognized` and `autos` are deduplicated in listing
/// order. `supertraits` is the auto-trait and pass-through bounds in listing
/// order, ready to re-emit as a shim's supertraits. `passthrough` is just the
/// plain pass-through bounds, kept for their spans: `dyn_shim` keeps them as
/// supertraits, while `trait_object` and `dyn_shim_recognized` reject the first
/// one. `rejected` pairs each known-impossible bound (`Copy`, `Ord`, ...) with
/// its targeted message, in listing order. Duplicates are dropped silently,
/// matching the language's own tolerance of `trait Foo: A + A`.
struct ClassifiedBounds {
    recognized: Vec<RecognizedBound>,
    autos: Vec<AutoTrait>,
    supertraits: Punctuated<TypeParamBound, Token![+]>,
    passthrough: Vec<TypeParamBound>,
    rejected: Vec<(TypeParamBound, &'static str)>,
}

impl ClassifiedBounds {
    fn classify(bounds: &Punctuated<TypeParamBound, Token![+]>) -> ClassifiedBounds {
        let mut recognized = Vec::new();
        let mut autos = Vec::new();
        let mut supertraits = Punctuated::new();
        let mut passthrough = Vec::new();
        let mut rejected = Vec::new();
        for bound in bounds {
            match Classified::of(bound) {
                Classified::Recognized(k) => {
                    if !recognized.contains(&k) {
                        recognized.push(k);
                    }
                }
                Classified::Auto(auto) => {
                    if !autos.contains(&auto) {
                        autos.push(auto);
                    }
                    supertraits.push(bound.clone());
                }
                Classified::Rejected(msg) => rejected.push((bound.clone(), msg)),
                Classified::PassThrough => {
                    passthrough.push(bound.clone());
                    supertraits.push(bound.clone());
                }
            }
        }
        ClassifiedBounds {
            recognized,
            autos,
            supertraits,
            passthrough,
            rejected,
        }
    }
}

/// Expansion for [`macro@trait_object`]: re-emit the annotated trait unchanged
/// and mount each listed carrier onto its `dyn` objects by invoking that
/// carrier's generated mount macro. A carrier is any trait the annotated trait
/// inherits whose mount macro stamps `impl Target for dyn Trait` — the shipped
/// `DynClone`/`DynHash`, or a `#[dyn_shim]` shim. Auto traits in the list are
/// markers selecting the covered `dyn` variants, exactly as for a recognized
/// bound. This emits no impls itself; the linking lives entirely in the
/// carriers' macros, shared with the `reflexive` option.
fn expand_trait_object(
    input: &ItemTrait,
    bounds: Punctuated<TypeParamBound, Token![+]>,
) -> TokenStream {
    if let Some(err) = reject_generics(input, "trait_object does not support generic traits") {
        return err;
    }

    // Split the list: auto traits are markers, every other bound is a carrier to
    // mount (named by its mount macro). A bare `Clone`/`Hash` names a capability
    // rather than a carrier trait, so it is rejected toward the carrier to name.
    let mut carriers: Vec<Path> = Vec::new();
    let mut autos: Vec<AutoTrait> = Vec::new();
    for bound in &bounds {
        match Classified::of(bound) {
            Classified::Auto(auto) => {
                if !autos.contains(&auto) {
                    autos.push(auto);
                }
            }
            Classified::Recognized(_) => {
                return syn::Error::new_spanned(
                    bound,
                    "name the carrier trait, not the capability: write `DynClone` / `DynHash` \
                     (gated on the `dyn_clone` / `dyn_hash` feature) and inherit it as a supertrait",
                )
                .to_compile_error()
                .into();
            }
            _ => {
                let Some(path) = plain_trait_bound(bound) else {
                    return syn::Error::new_spanned(
                        bound,
                        "a carrier must be a plain trait path (no `?`, no higher-ranked binder)",
                    )
                    .to_compile_error()
                    .into();
                };
                carriers.push(path.clone());
            }
        }
    }
    if carriers.is_empty() {
        return syn::Error::new_spanned(
            &bounds,
            "trait_object expects at least one carrier trait to mount (`DynClone`, `DynHash`, \
             or a `#[dyn_shim]` shim), optionally followed by auto-trait markers",
        )
        .to_compile_error()
        .into();
    }

    // Each carrier must be inherited as a supertrait, so `dyn Trait: Carrier`
    // holds and the mounted impl's forwarding body type-checks. Report a missing
    // one here, at the attribute, rather than as a macro-not-found or
    // trait-bound error on generated code.
    for carrier in &carriers {
        if let Err(err) = require_carrier(input, carrier) {
            return err.to_compile_error().into();
        }
    }

    let trait_ident = &input.ident;
    let combos = MarkerCombo::all(&autos);
    let mut mounts = TokenStream2::new();
    for carrier in &carriers {
        for MarkerCombo { markers, .. } in &combos {
            mounts.extend(quote! {
                #carrier! { @mount (#trait_ident) ( #markers ) }
            });
        }
    }

    quote! {
        #input
        #mounts
    }
    .into()
}

/// Check that the annotated trait inherits `carrier` as a supertrait, matched by
/// the carrier path's last segment (a bare-name token match, like
/// [`Classified::of`]). Without it, `dyn Trait: Carrier` would not hold and the
/// mounted impl's forwarding call would fail to compile on generated code.
fn require_carrier(input: &ItemTrait, carrier: &Path) -> syn::Result<()> {
    let name = &carrier.segments.last().unwrap().ident;
    let present = input.supertraits.iter().any(|bound| {
        plain_trait_bound(bound)
            .and_then(|path| path.segments.last())
            .is_some_and(|seg| &seg.ident == name)
    });
    if present {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            &input.ident,
            format!(
                "trait_object needs `{name}` as a supertrait; write `trait {}: {name}`",
                input.ident
            ),
        ))
    }
}

/// Render a path as `a::b::C` for doc links, dropping any generic arguments.
fn path_doc_string(path: &Path) -> String {
    let mut out = String::new();
    if path.leading_colon.is_some() {
        out.push_str("::");
    }
    for (i, segment) in path.segments.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        out.push_str(&segment.ident.to_string());
    }
    out
}

/// Shared expansion for both attributes. `input` is the annotated trait, read
/// for the shim's visibility, generics, and method signatures (and, for the
/// local form, re-emitted). `source_ref` is how the source trait is named in
/// the blanket impl and the forwarding calls (an ident for the local form, a
/// path for the foreign one); `source_doc` is its `::`-joined spelling for doc
/// links. `reemit` is `true` for the local form, which owns the source trait
/// and re-emits it with a dyn-compat doc note, and `false` for the foreign
/// form, whose annotated trait is the shim itself. `reflexive`, when set, also
/// emits an `impl SourceTrait for <shim object>` so the shim's trait object
/// satisfies the source trait.
fn expand(
    shim_name: Ident,
    bounds: Punctuated<TypeParamBound, Token![+]>,
    input: &ItemTrait,
    source_ref: &TokenStream2,
    source_doc: &str,
    reemit: bool,
    reflexive: Vec<ObjectForm>,
) -> TokenStream {
    if let Some(err) = reject_generics(input, "dyn_shim does not support generic source traits") {
        return err;
    }
    let vis = &input.vis;
    let items = &input.items;

    // Partition the bounds list. A recognized std trait (`Clone`, `Hash`) is
    // drained: as a supertrait it would break dyn-compatibility, so it
    // instead becomes a bound on the blanket impl plus proxy machinery on the
    // shim's trait objects. A recognized auto trait passes through like any
    // other bound and additionally selects which `dyn Shim + markers` types
    // get that machinery. A plain bound passes through as a supertrait.
    // Position in the list never matters.
    let ClassifiedBounds {
        recognized,
        autos,
        supertraits: passthrough,
        rejected,
        ..
    } = ClassifiedBounds::classify(&bounds);
    if let Some((bound, msg)) = rejected.into_iter().next() {
        return syn::Error::new_spanned(bound, msg)
            .to_compile_error()
            .into();
    }
    // The marker combinations feed the recognized-bound machinery and the
    // reflexive impl (one per `dyn Shim + markers` variant), so there is
    // nothing to compute when neither is present.
    let combos = if recognized.is_empty() && reflexive.is_empty() {
        Vec::new()
    } else {
        MarkerCombo::all(&autos)
    };

    // A `#[dyn_shim(boxed)]` builder's shim method returns the marker-free
    // `Box<dyn Shim>`, so a reflexive impl on a `+ marker` object form cannot be
    // satisfied by it. Reject that combination up front rather than emit a type
    // error on generated code; the boxed builder is still usable without markers.
    //
    // TODO: lift this gate. `build_boxed_method` already emits per-combo
    // `Box<dyn Shim + markers>` (that is how the `Clone` carrier supports
    // markers), so the blocker is only the forwarding side: a builder's reflexive
    // impl forwards through the shared, combo-independent `@mount` entries, which
    // cannot pick a per-combo method. Lifting it means emitting suffixed shim
    // methods (`add`, `add_send`, ...) like `Clone`'s `__dyn_shim_clone_box`, and
    // teaching the mount layer to route each combo's impl to the matching suffix
    // — which `macro_rules!` cannot do today (it cannot derive a suffix ident
    // from the `$($marker)*` tokens). That is a change to the mount machinery, not
    // more sharing; see `build_mount_macro`.
    if !reflexive.is_empty()
        && !autos.is_empty()
        && let Some(method) = items.iter().find_map(|item| match item {
            TraitItem::Fn(m) if matches!(Helper::of(m), Some(Helper::Boxed)) => Some(m),
            _ => None,
        })
    {
        return syn::Error::new_spanned(
            &method.sig.ident,
            "#[dyn_shim(boxed)] does not yet support auto-trait markers on a reflexive shim: the \
             boxed return `Box<dyn Shim>` cannot carry `+ Send` and similar",
        )
        .to_compile_error()
        .into();
    }

    // Validate the `#[dyn_shim(...)]` helper attributes: on a method the only
    // supported argument is `skip`; on any other trait item the attribute is
    // rejected outright. Only methods are stripped of it before the trait is
    // re-emitted, so left in place rustc would re-expand it as this attribute
    // macro and fail with an unrelated parse error pointing at the item.
    for item in items {
        let attrs = match item {
            TraitItem::Fn(item) => {
                for attr in item.attrs.iter().filter(|a| a.path().is_ident("dyn_shim")) {
                    if let Err(err) = Helper::parse(attr) {
                        return err.to_compile_error().into();
                    }
                }
                continue;
            }
            TraitItem::Const(item) => &item.attrs,
            TraitItem::Type(item) => &item.attrs,
            TraitItem::Macro(item) => &item.attrs,
            _ => continue,
        };
        if let Some(attr) = attrs.iter().find(|a| a.path().is_ident("dyn_shim")) {
            return syn::Error::new_spanned(
                attr,
                "#[dyn_shim] attributes are only supported on methods",
            )
            .to_compile_error()
            .into();
        }
    }

    let mut sigs = Vec::new();
    let mut impls = Vec::new();
    let mut skipped: Vec<(String, &str)> = Vec::new();
    for item in items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        match skip(method) {
            Some(reason) => skipped.push((method.sig.ident.to_string(), reason)),
            // A foreign-shim method with a default body is shim-local: emit it
            // verbatim (minus our helper attrs) so the shim trait carries it,
            // and add no forwarding impl — the blanket impl inherits the default.
            None if is_provided(method, reemit) => {
                let mut provided = method.clone();
                provided.attrs.retain(|a| !a.path().is_ident("dyn_shim"));
                sigs.push(quote! { #provided });
            }
            None => match forward(method, &shim_name, source_ref) {
                Ok((sig, body)) => {
                    sigs.push(sig);
                    impls.push(body);
                }
                Err(err) => return err.to_compile_error().into(),
            },
        }
    }

    // Re-emit the source trait (local form only) without our
    // `#[dyn_shim(skip)]` helper attributes, and point its docs at the
    // generated shim. The foreign form reads only the signatures above and
    // emits nothing for the annotated trait.
    let clean = reemit.then(|| {
        let mut clean = input.clone();
        for item in &mut clean.items {
            if let TraitItem::Fn(method) = item {
                method.attrs.retain(|a| !a.path().is_ident("dyn_shim"));
            }
        }
        for line in source_note(&shim_name) {
            clean.attrs.push(syn::parse_quote! { #[doc = #line] });
        }
        clean
    });

    let doc_attrs = shim_doc(source_doc, &shim_name, &recognized, &skipped)
        .into_iter()
        .map(|line| quote! { #[doc = #line] });

    // The passed-through bounds become the shim's supertraits, so the blanket
    // impl must require them of the implementor as well. A recognized bound
    // requires its trait of the implementor too, but only on the impl. Under
    // the `dyn_clone`/`dyn_hash` features a recognized `Clone`/`Hash` also adds the
    // standalone `DynClone`/`DynHash` as a supertrait, so the shim's `dyn` type
    // upcasts into that standalone shim. The blanket impl needs no extra bound
    // for it: its `Clone`/`Hash` bound already implies `DynClone`/`DynHash`.
    let mut shim_supers: Vec<TokenStream2> = passthrough.iter().map(|b| quote! { #b }).collect();
    for k in &recognized {
        if let Some(path) = k.dyn_supertrait() {
            shim_supers.push(path);
        }
    }
    let supertraits = (!shim_supers.is_empty()).then(|| quote! { : #(#shim_supers)+* });
    let impl_bounds = (!passthrough.is_empty()).then(|| quote! { + #passthrough });
    let recognized_bounds: TokenStream2 = recognized
        .iter()
        .map(|k| {
            let path = k.impl_bound();
            quote! { + #path }
        })
        .collect();

    let mut recognized_sigs = TokenStream2::new();
    let mut recognized_impls = TokenStream2::new();
    let mut recognized_extra = TokenStream2::new();
    for k in &recognized {
        let (sigs, impls, extra) = k.expand(&shim_name, &combos);
        recognized_sigs.extend(sigs);
        recognized_impls.extend(impls);
        recognized_extra.extend(extra);
    }

    // The forwarding bodies for each object form, computed once and shared by
    // the mount macro and the `reflexive` invocations below. A form whose bodies
    // do not type-check (a by-value `self` under `bare`, an unsupported receiver)
    // is an `Err`, deferred: it surfaces only if `reflexive` actually requests
    // that form, and is otherwise simply left out of the macro.
    let bare_entries = reflexive_entries(ObjectForm::Bare, &shim_name, items, reemit);
    let boxed_entries = reflexive_entries(ObjectForm::Boxed, &shim_name, items, reemit);

    // Always emit the shim's mount macro (when any form is expressible), so a
    // downstream `#[trait_object(Shim)]` can mount the source trait onto its own
    // principal even when this shim requested no reflexive impl of its own.
    let mount_macro = build_mount_macro(&shim_name, source_ref, &bare_entries, &boxed_entries);

    // When requested, mount the source trait onto the shim's own trait objects
    // by invoking that macro, one form per requested kind and one call per marker
    // combination, so the shim's trait object satisfies the source trait. A
    // requested form that does not type-check reports its methods all at once
    // (and the reflexive impls are omitted), rather than cascading into an
    // "unimplemented trait items" error on generated code.
    let reflexive_impl = {
        let mut tokens = TokenStream2::new();
        let mut errors: Option<syn::Error> = None;
        for kind in &reflexive {
            let (entries, tag) = match kind {
                ObjectForm::Bare => (&bare_entries, quote! { @bare }),
                ObjectForm::Boxed => (&boxed_entries, quote! { @boxed }),
            };
            match entries {
                Ok(_) => {
                    for MarkerCombo { markers, .. } in &combos {
                        let self_ty = kind.ty(&shim_name, markers);
                        tokens.extend(quote! { #shim_name! { #tag #self_ty } });
                    }
                }
                Err(err) => match &mut errors {
                    Some(acc) => acc.combine(err.clone()),
                    None => errors = Some(err.clone()),
                },
            }
        }
        match errors {
            Some(err) => err.to_compile_error(),
            None => tokens,
        }
    };

    quote! {
        #clean

        #(#doc_attrs)*
        #vis trait #shim_name #supertraits {
            #(#sigs)*
            #recognized_sigs
        }

        impl<__T: #source_ref #impl_bounds #recognized_bounds> #shim_name for __T {
            #(#impls)*
            #recognized_impls
        }

        #recognized_extra

        #mount_macro

        #reflexive_impl
    }
    .into()
}

/// Expansion for [`macro@dyn_shim_recognized`]: emit a standalone shim whose
/// only contents are a recognized std trait's generated machinery. The
/// annotated trait supplies the shim's name and visibility and must be a
/// non-generic trait with no methods or supertraits of its own; the recognized
/// trait and its auto-trait markers come from the attribute.
fn expand_recognized(
    input: &ItemTrait,
    bounds: Punctuated<TypeParamBound, Token![+]>,
) -> TokenStream {
    if let Some(err) = reject_generics(input, "dyn_shim_recognized does not support generic shims")
    {
        return err;
    }
    if let Some(item) = input.items.first() {
        return syn::Error::new_spanned(
            item,
            "a dyn_shim_recognized shim has no items of its own; its contents are generated \
             from the recognized trait",
        )
        .to_compile_error()
        .into();
    }
    if let Some(supertrait) = input.supertraits.first() {
        return syn::Error::new_spanned(
            supertrait,
            "list auto-trait markers in the attribute (`dyn_shim_recognized(Clone + Send)`), \
             not as supertraits of the shim",
        )
        .to_compile_error()
        .into();
    }

    // Exactly one recognized trait is the principal; the rest must be auto
    // traits, which select the covered marker combinations. A rejected or
    // pass-through bound is neither, and gets one uniform message.
    let ClassifiedBounds {
        recognized,
        autos,
        passthrough,
        rejected,
        ..
    } = ClassifiedBounds::classify(&bounds);
    if let Some(bound) = rejected
        .first()
        .map(|(bound, _)| bound)
        .or_else(|| passthrough.first())
    {
        return syn::Error::new_spanned(
            bound,
            "dyn_shim_recognized expects a recognized trait (`Clone` or `Hash`), \
             optionally followed by auto-trait markers",
        )
        .to_compile_error()
        .into();
    }
    if recognized.len() > 1 {
        return syn::Error::new_spanned(
            &bounds,
            "expected a single recognized trait (`Clone` or `Hash`)",
        )
        .to_compile_error()
        .into();
    }
    let Some(recognized) = recognized.into_iter().next() else {
        return syn::Error::new_spanned(
            &bounds,
            "dyn_shim_recognized expects a recognized trait (`Clone` or `Hash`)",
        )
        .to_compile_error()
        .into();
    };

    let shim = &input.ident;
    let vis = &input.vis;
    let attrs = &input.attrs;
    let combos = MarkerCombo::all(&autos);
    let (sigs, impls, extra) = recognized.expand(shim, &combos);
    let impl_bound = recognized.impl_bound();
    let doc = recognized.doc_line(shim);
    // The mount macro that backs `#[trait_object(Carrier)]`: stamps this
    // capability's bridge impls onto an arbitrary principal that inherits the
    // carrier. Named after the carrier so the carrier's import carries it.
    let mount_arm = recognized.mount_arm(shim);

    quote! {
        #(#attrs)*
        #[doc = ""]
        #[doc = #doc]
        #vis trait #shim {
            #sigs
        }

        impl<__T: #impl_bound> #shim for __T {
            #impls
        }

        #extra

        #[allow(non_local_definitions)]
        #[macro_export]
        #[doc(hidden)]
        macro_rules! #shim {
            (@mount ($principal:path) ($($marker:tt)*)) => {
                #mount_arm
            };
        }
    }
    .into()
}

/// Rename a method's value arguments (everything after the receiver) in place to
/// their declared idents, giving a synthetic `__a{i}` to any non-trivial pattern
/// (only legal on a defaulted method). Returns the names in order so a generated
/// body can forward them.
fn rename_args(sig: &mut Signature) -> Vec<Ident> {
    let mut names = Vec::new();
    for (i, arg) in sig.inputs.iter_mut().skip(1).enumerate() {
        let FnArg::Typed(pat) = arg else { continue };
        let id = match &*pat.pat {
            Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => p.ident.clone(),
            _ => format_ident!("__a{i}"),
        };
        *pat.pat = syn::parse_quote! { #id };
        names.push(id);
    }
    names
}

/// A method's `#[cfg]` gates, the only attributes that carry onto a generated
/// forwarding method. A `#[cfg]`-gated method must stay gated everywhere it is
/// emitted, while attributes like `#[must_use]` and `#[deprecated]` (and the
/// `cfg_attr` that can expand to them) are rejected on trait methods in impl
/// blocks.
fn cfg_gates(method: &TraitItemFn) -> Vec<&Attribute> {
    method
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .collect()
}

/// A method whose generic parameters have been lowered to trait objects, ready
/// to drop into a vtable. Produced by [`erase_generic`] and consumed both by
/// `#[dyn_shim(erase)]` source methods (via [`forward_erased`]) and by the
/// `Hash` carrier's hidden hashing method (via [`expand_hash`]).
struct ErasedFn {
    /// The dyn-compatible signature: every erased type parameter removed from
    /// the generics and its `&P` / `&mut P` argument retyped to `&dyn Bound` /
    /// `&mut dyn Bound`, with arguments renamed for forwarding.
    sig: Signature,
    /// `let mut <arg> = <arg>;` rebindings for each erased mutable argument, so
    /// the reborrow below has a mutable place to point at.
    preamble: TokenStream2,
    /// One forwarding expression per argument after the receiver: a reborrow
    /// (`&<arg>` / `&mut <arg>`) for an erased argument so the callee's type
    /// parameter re-infers to the sized reference type, the plain name
    /// otherwise.
    args: Vec<TokenStream2>,
}

/// How an erased argument is reborrowed when forwarding.
#[derive(Clone, Copy)]
enum Erased {
    /// `&P` was lowered to `&dyn Bound`; forward `&arg`.
    Shared,
    /// `&mut P` was lowered to `&mut dyn Bound`; forward `&mut arg` (needs a
    /// `let mut` rebinding).
    Mut,
}

/// Lower a method's generic parameters, each bounded by a single trait and used
/// only behind a reference, to `&dyn Bound` / `&mut dyn Bound`. This is the
/// transform behind `#[dyn_shim(erase)]`: a generic argument such as `w: &mut
/// impl Write` (or a named `<W: Write>` used as `&mut W`) cannot enter a vtable,
/// but `w: &mut dyn Write` can. Forwarding reborrows the argument (`&mut w`) so
/// the source method's type parameter re-infers to the sized reference type,
/// which is sound exactly when std (or the bound's author) provides `impl Bound
/// for &mut (dyn Bound)` — true for `Hasher`, `Write`, `Read`, and friends.
///
/// The recognized [`Hash`](RecognizedBound::Hash) carrier is the proof case: its
/// hidden hashing method is this transform applied to `fn(&self, &mut impl
/// Hasher)`.
///
/// Returns `Err` for a parameter that cannot be erased — a const generic, more
/// than one trait bound, a use by value or in the return type, or a use in more
/// than one argument (which would force one inferred reference type onto two
/// independent trait objects) — so the caller reports it where the user opted in.
fn erase_generic(sig: &Signature) -> syn::Result<ErasedFn> {
    let mut sig = sig.clone();

    // Each erasable type parameter and the single trait path it is bounded by,
    // the only shape that lowers to one `dyn Bound`. A `?Sized` relaxation is
    // ignored (the parameter is dropped anyway); lifetime bounds are kept out of
    // the count.
    let mut params: Vec<(Ident, Path)> = Vec::new();
    for param in &sig.generics.params {
        match param {
            GenericParam::Lifetime(_) => {}
            GenericParam::Const(c) => {
                return Err(syn::Error::new_spanned(
                    c,
                    "#[dyn_shim(erase)] cannot erase a const generic parameter to a trait object",
                ));
            }
            GenericParam::Type(t) => {
                let traits: Vec<&Path> = t
                    .bounds
                    .iter()
                    .filter_map(plain_trait_bound)
                    .collect();
                let [path] = traits.as_slice() else {
                    return Err(syn::Error::new_spanned(
                        t,
                        format!(
                            "#[dyn_shim(erase)] needs `{}` to have exactly one trait bound to \
                             lower it to a single `dyn Bound`",
                            t.ident
                        ),
                    ));
                };
                params.push((t.ident.clone(), (*path).clone()));
            }
        }
    }

    // Walk the arguments, retyping each `&[mut] P` / `&[mut] impl Bound` referent
    // to `dyn Bound` and recording how to reborrow it. A parameter must appear in
    // exactly one argument; track uses to reject zero (nothing to erase) or many
    // (one inferred type cannot serve two objects).
    let mut uses: Vec<usize> = vec![0; params.len()];
    let mut erased: Vec<Option<Erased>> = Vec::new();
    for arg in sig.inputs.iter_mut().skip(1) {
        let FnArg::Typed(pat) = arg else { continue };
        erased.push(erase_arg_ty(&mut pat.ty, &params, &mut uses)?);
    }

    // A parameter used outside a single reference argument — in the return type,
    // by value, or more than once — cannot be erased.
    if let ReturnType::Type(_, ty) = &sig.output
        && let Some((ident, _)) = params.iter().find(|(id, _)| type_mentions_ident(ty, id))
    {
        return Err(syn::Error::new_spanned(
            ty,
            format!("#[dyn_shim(erase)] cannot erase `{ident}`: it appears in the return type"),
        ));
    }
    for ((ident, _), count) in params.iter().zip(&uses) {
        if *count == 0 {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "#[dyn_shim(erase)] cannot erase `{ident}`: it is not used behind a `&` or \
                     `&mut` argument (only such uses lower to a trait object)"
                ),
            ));
        }
        if *count > 1 {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "#[dyn_shim(erase)] cannot erase `{ident}`: it is used in more than one \
                     argument, which would force one reference type onto independent objects"
                ),
            ));
        }
    }

    // Drop the erased parameters (now all type parameters) and any `where`
    // predicate bounding them, leaving only lifetimes.
    let erased_idents: Vec<&Ident> = params.iter().map(|(id, _)| id).collect();
    sig.generics.params = std::mem::take(&mut sig.generics.params)
        .into_iter()
        .filter(|p| !matches!(p, GenericParam::Type(t) if erased_idents.contains(&&t.ident)))
        .collect();
    if sig.generics.params.is_empty() {
        sig.generics.lt_token = None;
        sig.generics.gt_token = None;
    }
    if let Some(where_clause) = &mut sig.generics.where_clause {
        where_clause.predicates = std::mem::take(&mut where_clause.predicates)
            .into_iter()
            .filter(|pred| !predicate_bounds_any(pred, &erased_idents))
            .collect();
        if where_clause.predicates.is_empty() {
            sig.generics.where_clause = None;
        }
    }

    // Rename arguments and build the reborrow forwarding expressions.
    let mut preamble = TokenStream2::new();
    let mut args = Vec::new();
    for (i, (arg, kind)) in sig.inputs.iter_mut().skip(1).zip(&erased).enumerate() {
        let FnArg::Typed(pat) = arg else { continue };
        let name = match &*pat.pat {
            Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => p.ident.clone(),
            _ => format_ident!("__a{i}"),
        };
        *pat.pat = parse_quote! { #name };
        match kind {
            Some(Erased::Mut) => {
                preamble.extend(quote! { let mut #name = #name; });
                args.push(quote! { &mut #name });
            }
            Some(Erased::Shared) => args.push(quote! { & #name }),
            None => args.push(quote! { #name }),
        }
    }

    Ok(ErasedFn {
        sig,
        preamble,
        args,
    })
}

/// Retype one argument if it is a `&[mut] P` / `&[mut] impl Bound` that can be
/// erased, returning how to reborrow it. `params` maps a named parameter to its
/// bound; `uses` counts named-parameter uses for the single-use check. A named
/// parameter mentioned anywhere but as a bare reference referent is left for the
/// caller's by-value / return-type checks to reject.
fn erase_arg_ty(
    ty: &mut Type,
    params: &[(Ident, Path)],
    uses: &mut [usize],
) -> syn::Result<Option<Erased>> {
    let Type::Reference(reference) = ty else {
        return Ok(None);
    };
    let kind = if reference.mutability.is_some() {
        Erased::Mut
    } else {
        Erased::Shared
    };
    let bound: Path = match &*reference.elem {
        // `&[mut] P` for a named parameter P.
        Type::Path(p) if p.qself.is_none() => {
            let Some(idx) = p
                .path
                .get_ident()
                .and_then(|id| params.iter().position(|(pid, _)| pid == id))
            else {
                return Ok(None);
            };
            uses[idx] += 1;
            params[idx].1.clone()
        }
        // `&[mut] impl Bound` (argument-position impl Trait), an anonymous
        // single-bounded parameter.
        Type::ImplTrait(it) => {
            let traits: Vec<&Path> = it.bounds.iter().filter_map(plain_trait_bound).collect();
            let [path] = traits.as_slice() else {
                return Err(syn::Error::new_spanned(
                    it,
                    "#[dyn_shim(erase)] needs the `impl Trait` argument to have exactly one \
                     trait bound to lower it to a single `dyn Bound`",
                ));
            };
            (*path).clone()
        }
        _ => return Ok(None),
    };
    *reference.elem = parse_quote! { dyn #bound };
    Ok(Some(kind))
}

/// True if a type mentions the given identifier as a path segment.
fn type_mentions_ident(ty: &Type, ident: &Ident) -> bool {
    struct Finder<'a> {
        ident: &'a Ident,
        hit: bool,
    }
    impl<'ast, 'a> Visit<'ast> for Finder<'a> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if path.is_ident(self.ident) {
                self.hit = true;
            }
            visit::visit_path(self, path);
        }
    }
    let mut finder = Finder { ident, hit: false };
    finder.visit_type(ty);
    finder.hit
}

/// True if a `where` predicate bounds one of the given (erased) parameters, so
/// it can be dropped alongside them.
fn predicate_bounds_any(pred: &syn::WherePredicate, idents: &[&Ident]) -> bool {
    let syn::WherePredicate::Type(pred) = pred else {
        return false;
    };
    let Type::Path(bounded) = &pred.bounded_ty else {
        return false;
    };
    bounded
        .path
        .get_ident()
        .is_some_and(|id| idents.contains(&id))
}

/// Build the shim signature and the forwarding impl body for one method.
///
/// The shim method reuses the source method's entire signature (`unsafe`, ABI,
/// generics, `where` clause, ...) and its attributes, rewriting only the
/// inputs: a by-value `self` becomes `self: Box<Self>`, and each argument
/// keeps its declared name where it has one. Copying the attributes keeps
/// `#[doc]`, `#[must_use]`, and `#[deprecated]` working on the shim, and keeps
/// a `#[cfg]`-gated method gated consistently across the source trait, the
/// shim trait, and the blanket impl.
///
/// `src` is how the source trait is named in the forwarding call: its own ident
/// for a local source trait, or a path for a foreign one.
///
/// A method marked `#[dyn_shim(erase)]` is routed through [`forward_erased`],
/// which lowers its generic parameters to trait objects, and one marked
/// `#[dyn_shim(boxed)]` through [`forward_boxed`], which boxes a `-> Self`
/// return into `Box<dyn shim>`. Both can fail (a generic used by value, a
/// non-`Self` return, ...), so this returns a `Result`.
fn forward(
    method: &TraitItemFn,
    shim: &Ident,
    src: &TokenStream2,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    match Helper::of(method) {
        Some(Helper::Erase) => return forward_erased(method, src),
        Some(Helper::Boxed) => return forward_boxed(method, shim, src),
        _ => {}
    }

    let mut sig = method.sig.clone();

    let Some(FnArg::Receiver(recv)) = sig.inputs.first() else {
        unreachable!("skip guarantees a receiver")
    };
    // A by-value `self` (or its explicit `self: Self` spelling) is rewritten to
    // `self: Box<Self>` below; only a typed receiver with a real wrapper type
    // (Box, Rc, Arc, Pin, ...) is forwarded unchanged.
    let by_value = matches!(ReceiverKind::of(recv), ReceiverKind::Value);
    let self_expr = if by_value {
        // Absolute path: the expansion must not depend on what `Box` names at
        // the call site (a local shadow, or a missing prelude under no_std).
        sig.inputs[0] = syn::parse_quote! { self: ::std::boxed::Box<Self> };
        quote! { *self }
    } else {
        quote! { self }
    };

    let names = rename_args(&mut sig);

    // The shim's signature keeps every attribute except our `#[dyn_shim]`
    // helpers, so `#[doc]`, `#[must_use]`, and `#[deprecated]` carry over. The
    // impl method takes only the `#[cfg]` gates (see `cfg_gates`); `#[allow]`
    // keeps the generated forwarding call to a `#[deprecated]` method quiet.
    let attrs: Vec<&Attribute> = method
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("dyn_shim"))
        .collect();
    let cfg_attrs = cfg_gates(method);

    let name = &sig.ident;
    let shim_sig = quote! {
        #(#attrs)*
        #sig ;
    };
    let shim_impl = quote! {
        #(#cfg_attrs)*
        #[allow(deprecated)]
        #sig {
            <__T as #src>::#name(#self_expr #(, #names)*)
        }
    };
    Ok((shim_sig, shim_impl))
}

/// Build the shim signature and forwarding body for an `#[dyn_shim(erase)]`
/// method, lowering its generic parameters to trait objects via
/// [`erase_generic`]. The shim method is non-generic and so enters the vtable,
/// while the forwarding call reborrows each erased argument so the source
/// method's type parameter re-infers to the sized reference type.
///
/// Receiver handling matches [`forward`]: a by-value `self` becomes `self:
/// Box<Self>`, forwarded by dereference.
fn forward_erased(method: &TraitItemFn, src: &TokenStream2) -> syn::Result<(TokenStream2, TokenStream2)> {
    let ErasedFn {
        mut sig,
        preamble,
        args,
    } = erase_generic(&method.sig)?;

    let Some(FnArg::Receiver(recv)) = sig.inputs.first() else {
        unreachable!("skip guarantees a receiver")
    };
    let self_expr = if matches!(ReceiverKind::of(recv), ReceiverKind::Value) {
        sig.inputs[0] = parse_quote! { self: ::std::boxed::Box<Self> };
        quote! { *self }
    } else {
        quote! { self }
    };

    let attrs: Vec<&Attribute> = method
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("dyn_shim"))
        .collect();
    let cfg_attrs = cfg_gates(method);

    let name = &sig.ident;
    let shim_sig = quote! {
        #(#attrs)*
        #sig ;
    };
    let shim_impl = quote! {
        #(#cfg_attrs)*
        #[allow(deprecated)]
        #sig {
            #preamble
            <__T as #src>::#name(#self_expr #(, #args)*)
        }
    };
    Ok((shim_sig, shim_impl))
}

/// Build the shim signature and forwarding body for a `#[dyn_shim(boxed)]`
/// method: a `-> Self` builder made dyn-compatible by boxing its result into
/// `Box<dyn shim>`. `Self` would be unsized as the trait object, so it cannot
/// be returned directly; the shim returns the boxed object instead, and the
/// blanket impl boxes the concrete result (which unsizes to `Box<dyn shim>`
/// because the implementor is `shim` and the method requires `Self: 'static`).
/// A `reflexive = boxed` impl can then satisfy the source `-> Self`, since there
/// `Self` *is* `Box<dyn shim>`. This is the general form of the boxing the
/// recognized `Clone` bound applies to `clone`.
///
/// Errors (reported where the user opted in) when the shape cannot be boxed: a
/// return that is not exactly `Self`, a generic method, or `Self` used in an
/// argument (only the return is boxed).
fn forward_boxed(
    method: &TraitItemFn,
    shim: &Ident,
    src: &TokenStream2,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let ReturnType::Type(_, ret) = &method.sig.output else {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "#[dyn_shim(boxed)] expects a method returning `Self`, to box into the shim object",
        ));
    };
    if !is_bare_self(ret) {
        return Err(syn::Error::new_spanned(
            ret,
            "#[dyn_shim(boxed)] expects the return type to be exactly `Self`; only a bare `Self` \
             can be boxed into the shim's trait object",
        ));
    }
    // TODO: support `boxed` + `erase` together, for a generic `-> Self` builder
    // such as `fn with<T: Into<X>>(self, t: T) -> Self`. A method carries one
    // `#[dyn_shim(...)]` helper today, and the two transforms are applied by
    // separate forwarders (`forward_erased` / `forward_boxed`). Composing them is
    // mechanical — they touch disjoint parts of the signature (erase rewrites the
    // arguments and generics, boxing rewrites the return) — but needs a way to
    // request both on one method and a single forwarder that runs `erase_generic`
    // and then boxes the result.
    if has_type_or_const_generics(&method.sig) {
        return Err(syn::Error::new_spanned(
            &method.sig.ident,
            "#[dyn_shim(boxed)] cannot apply to a generic method (combining it with `erase` is \
             not yet supported)",
        ));
    }
    for arg in method.sig.inputs.iter().skip(1) {
        if let FnArg::Typed(pat) = arg
            && type_finds(&pat.ty, true, false)
        {
            return Err(syn::Error::new_spanned(
                &pat.ty,
                "#[dyn_shim(boxed)] cannot forward a method that mentions `Self` in an argument; \
                 only a `-> Self` return is boxed",
            ));
        }
    }

    let mut sig = method.sig.clone();
    let Some(FnArg::Receiver(recv)) = sig.inputs.first() else {
        unreachable!("skip guarantees a receiver")
    };
    let self_expr = if matches!(ReceiverKind::of(recv), ReceiverKind::Value) {
        sig.inputs[0] = parse_quote! { self: ::std::boxed::Box<Self> };
        quote! { *self }
    } else {
        quote! { self }
    };

    let name = method.sig.ident.clone();
    let names = rename_args(&mut sig);
    // Box the forwarded source call into the shim object. No markers: a boxed
    // builder's reflexive impl forwards through the shared, combo-independent
    // mount entries (unlike `Clone`'s dedicated per-combo bridge), so the
    // marker-free `Box<dyn shim>` is all that path can satisfy — the marker
    // combination is rejected up front in `expand`.
    let call = quote! { <__T as #src>::#name(#self_expr #(, #names)*) };
    let (sig, body) = build_boxed_method(shim, &TokenStream2::new(), sig, call);

    let attrs: Vec<&Attribute> = method
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("dyn_shim"))
        .collect();
    let cfg_attrs = cfg_gates(method);

    let shim_sig = quote! {
        #(#attrs)*
        #sig ;
    };
    let shim_impl = quote! {
        #(#cfg_attrs)*
        #[allow(deprecated)]
        #sig { #body }
    };
    Ok((shim_sig, shim_impl))
}

/// Build the forwarding method bodies for an `impl SourceTrait for <shim
/// object>` in one object form. Each source method either forwards to the shim,
/// gets a panicking stub (`#[dyn_shim(panic)]`), or is omitted to inherit a
/// trait default body. Every method that cannot be placed is collected, so the
/// caller reports them all in one pass. The entries are independent of the
/// principal (they reference `self`/`&**self` and the shim's methods, with
/// `Self` resolving to the impl's self type), so the same set serves the shim's
/// own objects (the `reflexive` option) and any downstream principal that
/// inherits the shim (`#[trait_object(Shim)]`).
///
/// A shim-provided method (see [`is_provided`]) is left out entirely: it is not
/// a method of the source trait, so it has no place in `impl SourceTrait`.
fn reflexive_entries(
    kind: ObjectForm,
    shim: &Ident,
    items: &[TraitItem],
    reemit: bool,
) -> syn::Result<Vec<TokenStream2>> {
    let mut entries = Vec::new();
    let mut errors: Option<syn::Error> = None;
    for item in items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        if is_provided(method, reemit) {
            continue;
        }
        match reflexive_method(kind, shim, method) {
            Ok(Some(entry)) => entries.push(entry),
            // A non-forwardable method with a default body is left off the
            // impl, so calls fall back to the source trait's default.
            Ok(None) => {}
            Err(err) => match &mut errors {
                Some(acc) => acc.combine(err),
                None => errors = Some(err),
            },
        }
    }
    match errors {
        Some(err) => Err(err),
        None => Ok(entries),
    }
}

/// Build the shim's mount macro: a `macro_rules!` named after the shim that
/// stamps `impl SourceTrait for <object>`, forwarding through the shim. It backs
/// both the `reflexive` option (mounting onto the shim's own `dyn`/`Box` types)
/// and a downstream `#[trait_object(Shim)]` (mounting onto any principal that
/// inherits the shim). The arms:
///
/// - `@bare`/`@boxed` take a fully formed self type and stamp one form's impl;
///   the `reflexive` invocations use these, since they choose the form.
/// - `@mount` takes a principal trait path plus the marker tokens of one
///   combination and stamps every *expressible* form; `#[trait_object]` uses
///   this single uniform entry, shared with the recognized carriers.
///
/// Only the forms whose forwarding bodies type-check are emitted. The macro is
/// exported (so a downstream crate can mount through it) and named after the
/// shim, so the shim's own import carries it (`use krate::Shim` brings the trait
/// and the macro, which live in different namespaces). It is `#[doc(hidden)]`,
/// and `#[allow(non_local_definitions)]` keeps it quiet when a shim is declared
/// inside a function body (as a doctest's implicit `main` does).
fn build_mount_macro(
    shim: &Ident,
    source_ref: &TokenStream2,
    bare: &syn::Result<Vec<TokenStream2>>,
    boxed: &syn::Result<Vec<TokenStream2>>,
) -> TokenStream2 {
    let mut arms = TokenStream2::new();
    let mut mount_impls = TokenStream2::new();
    if let Ok(entries) = bare {
        arms.extend(quote! {
            (@bare $self_ty:ty) => {
                impl #source_ref for $self_ty { #(#entries)* }
            };
        });
        mount_impls.extend(quote! {
            impl #source_ref for dyn $principal $($marker)* { #(#entries)* }
        });
    }
    if let Ok(entries) = boxed {
        arms.extend(quote! {
            (@boxed $self_ty:ty) => {
                impl #source_ref for $self_ty { #(#entries)* }
            };
        });
        mount_impls.extend(quote! {
            impl #source_ref for ::std::boxed::Box<dyn $principal $($marker)*> { #(#entries)* }
        });
    }
    if arms.is_empty() {
        // Neither form forwards (every method is non-dyn-compatible without a
        // stub or default), so there is nothing to mount and no macro to emit.
        return TokenStream2::new();
    }
    quote! {
        #[allow(non_local_definitions)]
        #[macro_export]
        #[doc(hidden)]
        macro_rules! #shim {
            #arms
            (@mount ($principal:path) ($($marker:tt)*)) => {
                #mount_impls
            };
        }
    }
}

/// Build one method of the reflexive impl. `Ok(Some(..))` is a method to emit,
/// `Ok(None)` omits it, and `Err` reports a method that cannot be placed in the
/// impl at all. A non-forwardable method is omitted when it has a trait default
/// body (inherited) or, on the bare form, when it requires `Self: Sized` (not
/// part of the unsized object's surface); otherwise it needs a stub helper
/// (`#[dyn_shim(panic)]` / `#[dyn_shim(stub = ...)]`).
fn reflexive_method(
    kind: ObjectForm,
    shim: &Ident,
    method: &TraitItemFn,
) -> syn::Result<Option<TokenStream2>> {
    let name = &method.sig.ident;

    let stub_body = if skip(method).is_none() {
        // Forwardable: dispatched through the shim below.
        None
    } else {
        // A `where Self: Sized` method is excluded from the unsized bare
        // object's surface, so `impl Foo for dyn Shim` need not provide it:
        // omit it. Calling it on a `&dyn Shim` is then a compile error rather
        // than a runtime panic. (The boxed object is `Sized`, so it still needs
        // a stub or default.)
        if kind == ObjectForm::Bare && requires_self_sized(&method.sig) {
            return Ok(None);
        }
        // A default body on the source trait is inherited by the impl.
        if method.default.is_some() {
            return Ok(None);
        }
        // Otherwise a stub helper must supply a fallback body.
        match Helper::of(method).and_then(|h| h.stub_body(shim, name)) {
            Some(body) => Some(body),
            None => {
                let reason = skip(method).unwrap_or("not dyn-compatible");
                return Err(syn::Error::new_spanned(
                    name,
                    format!(
                        "`{name}` is not dyn-compatible ({reason}), so the reflexive impl cannot \
                         forward it; provide a fallback with `#[dyn_shim(panic)]` or \
                         `#[dyn_shim(stub = <expr>)]`, or give it a default body"
                    ),
                ));
            }
        }
    };

    // `reflexive = bare` impls for the unsized `dyn` type, so an emitted method
    // must not place `Self` by value.
    if kind == ObjectForm::Bare
        && let Some(err) = bare_inexpressible(method)
    {
        return Err(err);
    }

    // The impl restates the source signature (with `Self` left intact: it
    // resolves to the impl's self type), renaming arguments so the body can
    // forward them. Only `#[cfg]` gates carry over, matching `forward`.
    let mut sig = method.sig.clone();
    let names = rename_args(&mut sig);
    let cfg_attrs = cfg_gates(method);

    // A stub body (`panic` / `stub = ...`) typically ignores the arguments, so
    // silence the unused-variable warnings the restated signature would draw.
    let stub_allow = stub_body
        .is_some()
        .then(|| quote! { #[allow(unused_variables)] });
    let body = match stub_body {
        Some(body) => body,
        None => {
            let recv = match sig.inputs.first() {
                Some(FnArg::Receiver(recv)) => recv,
                _ => unreachable!("a forwarded method has a receiver"),
            };
            let recv_expr = kind.reflexive_receiver(recv, name)?;
            // Dispatch through the shim trait by name, so `Self` infers to the
            // `dyn` type (vtable dispatch to the concrete implementor). Calling
            // the source method on `self` instead would resolve right back to
            // this impl and recurse.
            quote! { #shim::#name(#recv_expr #(, #names)*) }
        }
    };

    Ok(Some(quote! {
        #(#cfg_attrs)*
        #stub_allow
        #[allow(deprecated)]
        #sig { #body }
    }))
}

impl ObjectForm {
    /// The receiver expression passed to the shim method when forwarding through
    /// the reflexive impl. Adjusts for the impl's self type: `Box<dyn Shim>`
    /// (boxed) dereferences to reach the `dyn` type, while `dyn Shim` (bare) is
    /// already there.
    fn reflexive_receiver(self, recv: &Receiver, name: &Ident) -> syn::Result<TokenStream2> {
        let expr = match (self, ReceiverKind::of(recv)) {
            // By-value `self`: boxed's self is `Box<dyn Shim>`, which is exactly the
            // shim method's `self: Box<Self>`. (Bare never reaches here: a by-value
            // receiver is rejected earlier as inexpressible.)
            (_, ReceiverKind::Value) => quote! { self },
            // `Box<Self>` source receiver: boxed's self is `Box<Box<dyn Shim>>`, so
            // peel one box; bare's self is already `Box<dyn Shim>`.
            (ObjectForm::Boxed, ReceiverKind::Boxed) => quote! { *self },
            (ObjectForm::Bare, ReceiverKind::Boxed) => quote! { self },
            // `&self` / `&mut self`: boxed reborrows through the box to the `dyn`
            // type; bare's receiver already is `&dyn Shim`.
            (ObjectForm::Boxed, ReceiverKind::Ref) => quote! { &**self },
            (ObjectForm::Boxed, ReceiverKind::RefMut) => quote! { &mut **self },
            (ObjectForm::Bare, ReceiverKind::Ref | ReceiverKind::RefMut) => quote! { self },
            (_, ReceiverKind::Other) => {
                return Err(syn::Error::new_spanned(
                    recv,
                    format!(
                        "`{name}`'s `self` receiver is not yet supported in a reflexive impl \
                     (only `self`, `&self`, `&mut self`, and `self: Box<Self>` are)"
                    ),
                ));
            }
        };
        Ok(expr)
    }
}

/// How a forwarded method's receiver is shaped, for reflexive forwarding.
enum ReceiverKind {
    /// By-value `self` (or the explicit `self: Self`).
    Value,
    /// `&self`.
    Ref,
    /// `&mut self`.
    RefMut,
    /// `self: Box<Self>`.
    Boxed,
    /// Any other typed receiver (`Rc<Self>`, `Arc<Self>`, `Pin<_>`, ...).
    Other,
}

impl ReceiverKind {
    /// Classify a method's `self` receiver by its shape.
    fn of(recv: &Receiver) -> ReceiverKind {
        if recv.reference.is_some() {
            if recv.mutability.is_some() {
                ReceiverKind::RefMut
            } else {
                ReceiverKind::Ref
            }
        } else if recv.colon_token.is_none()
            || matches!(&*recv.ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("Self"))
        {
            ReceiverKind::Value
        } else if is_box_self(&recv.ty) {
            ReceiverKind::Boxed
        } else {
            ReceiverKind::Other
        }
    }
}

/// True if a type is `Box<Self>` (by any path spelling of `Box`).
fn is_box_self(ty: &Type) -> bool {
    let Type::Path(p) = ty else {
        return false;
    };
    let Some(seg) = p.path.segments.last() else {
        return false;
    };
    seg.ident == "Box"
        && matches!(&seg.arguments, syn::PathArguments::AngleBracketed(a)
            if a.args.iter().any(|arg|
                matches!(arg, syn::GenericArgument::Type(Type::Path(t)) if t.path.is_ident("Self"))))
}

/// If a method cannot be expressed in a `reflexive = bare` impl (where `Self`
/// is the unsized `dyn` shim), return the error. Such a method places `Self`
/// by value: a by-value `self` receiver, a bare `-> Self` return, or a bare
/// `Self` argument.
fn bare_inexpressible(method: &TraitItemFn) -> Option<syn::Error> {
    let sig = &method.sig;
    let name = &sig.ident;

    if let Some(FnArg::Receiver(recv)) = sig.inputs.first()
        && matches!(ReceiverKind::of(recv), ReceiverKind::Value)
    {
        return Some(syn::Error::new_spanned(
            recv,
            format!(
                "`reflexive = bare` cannot include `{name}`: its by-value `self` receiver \
                 would take the unsized `dyn` shim by value. Use `reflexive = boxed`."
            ),
        ));
    }

    if let ReturnType::Type(_, ty) = &sig.output
        && is_bare_self(ty)
    {
        return Some(syn::Error::new_spanned(
            ty,
            format!(
                "`reflexive = bare` cannot include `{name}`: it returns `Self` by value, \
                 which is unsized as the `dyn` shim. Use `reflexive = boxed`."
            ),
        ));
    }

    for arg in sig.inputs.iter().skip(1) {
        if let FnArg::Typed(pat) = arg
            && is_bare_self(&pat.ty)
        {
            return Some(syn::Error::new_spanned(
                &pat.ty,
                format!(
                    "`reflexive = bare` cannot include `{name}`: it takes `Self` by value, \
                     which is unsized as the `dyn` shim. Use `reflexive = boxed`."
                ),
            ));
        }
    }

    None
}

/// True if a type is exactly the bare `Self` path (not `&Self`, `Box<Self>`,
/// or another type that merely mentions it).
fn is_bare_self(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("Self"))
}

/// Build the doc-comment lines appended to the source trait, pointing readers
/// at the generated dyn-compatible shim.
fn source_note(shim_name: &Ident) -> Vec<String> {
    vec![
        String::new(),
        "# Dyn Compatibility".to_string(),
        String::new(),
        format!(
            "[`{shim_name}`] is a generated dyn-compatible shim for this trait. \
             Use `dyn {shim_name}` to hold implementors behind a trait object."
        ),
    ]
}

/// Build the doc-comment lines for the generated shim trait: the capabilities
/// added by recognized bounds, and any source methods that were skipped and
/// why.
fn shim_doc(
    src: &str,
    shim: &Ident,
    recognized: &[RecognizedBound],
    skipped: &[(String, &str)],
) -> Vec<String> {
    let mut lines = vec![format!("Dyn-compatible shim for [`{src}`].")];
    if !recognized.is_empty() {
        lines.push(String::new());
        for k in recognized {
            lines.push(k.doc_line(shim));
        }
    }
    if !skipped.is_empty() {
        lines.push(String::new());
        lines.push("These methods of the source trait are not dyn-compatible, so they".to_string());
        lines.push("are not part of this shim. Call them on the concrete type.".to_string());
        lines.push(String::new());
        for (name, reason) in skipped {
            lines.push(format!("- [`{src}::{name}`] ({reason})"));
        }
    }
    lines
}

/// A `#[dyn_shim(...)]` helper attribute on a method.
#[derive(Clone)]
enum Helper {
    /// `#[dyn_shim(skip)]`: leave the method off the shim entirely.
    Skip,
    /// `#[dyn_shim(erase)]`: lower the method's generic parameters (each bounded
    /// by a single trait and used only behind a reference) to `&dyn Bound` /
    /// `&mut dyn Bound` so the method enters the shim's vtable, instead of being
    /// skipped as non-dyn-compatible. See [`erase_generic`].
    Erase,
    /// `#[dyn_shim(boxed)]`: forward a `-> Self` builder by boxing its result
    /// into the shim's trait object, so the shim method returns `Box<dyn Shim>`
    /// and a `reflexive = boxed` impl can satisfy the source `-> Self` (where
    /// `Self` is the boxed object). See [`forward_boxed`]; this is the general
    /// case of the boxing the recognized `Clone` bound applies to `clone`.
    Boxed,
    /// A fallback body for a method that cannot forward through the shim, used in
    /// a reflexive impl. `#[dyn_shim(panic)]` is `Stub(None)` (panic with a
    /// generated message); `#[dyn_shim(stub = <expr>)]` is `Stub(Some(expr))`,
    /// letting the method degrade to a value (`None`, `Default::default()`, ...)
    /// instead of aborting. See [`Helper::stub_body`].
    Stub(Option<Expr>),
}

/// A shim method that carries its own body, so it is *provided* by the shim
/// rather than forwarded to the source trait. Only in the foreign form: a
/// [`macro@dyn_shim_foreign`] method with a default body is shim-local — the
/// foreign trait need not declare it — so it is emitted verbatim on the shim
/// trait and left out of the blanket impl, the reflexive impl, and the mount
/// macro. Its body calls the shim's other (forwarded) methods.
///
/// In the local form a default body lives on the re-emitted source trait, where
/// forwarding still honors an implementor's override, so a defaulted method is
/// forwarded like any other and this returns `false`.
fn is_provided(method: &TraitItemFn, reemit: bool) -> bool {
    !reemit && method.default.is_some()
}

/// If a method cannot be dispatched through a trait object, return a short
/// reason it is skipped. Return `None` when the method is forwarded.
fn skip(method: &TraitItemFn) -> Option<&'static str> {
    let sig = &method.sig;
    // `#[dyn_shim(erase)]` keeps a method that is non-dyn-compatible only because
    // of its generic parameters or argument-position `impl Trait`: `erase_generic`
    // lowers those to trait objects. `#[dyn_shim(boxed)]` keeps a `-> Self`
    // builder, boxing the return into the shim object. Neither can rescue a
    // method for any other reason (no receiver, async, ...), so those still skip;
    // `forward_erased` / `forward_boxed` then report any opt-in they cannot honor.
    let helper = Helper::of(method);
    let erasing = matches!(helper, Some(Helper::Erase));
    let boxing = matches!(helper, Some(Helper::Boxed));
    if matches!(helper, Some(Helper::Skip)) {
        Some("opted out with #[dyn_shim(skip)]")
    } else if sig.asyncness.is_some() {
        Some("async fn")
    } else if !has_self_receiver(sig) {
        Some("no self receiver")
    } else if has_type_or_const_generics(sig) && !erasing {
        Some("generic type or const parameter")
    } else if requires_self_sized(sig) {
        Some("requires Self: Sized")
    } else if signature_mentions_self(sig) && !boxing {
        Some("mentions Self")
    } else if signature_mentions_impl_trait(sig) && !erasing {
        Some("uses impl Trait")
    } else {
        None
    }
}

impl Helper {
    /// The fallback body for a non-forwardable method's reflexive stub, if this
    /// helper supplies one. `#[dyn_shim(panic)]` (`Stub(None)`) panics with a
    /// generated message naming the method and shim; `#[dyn_shim(stub = <expr>)]`
    /// (`Stub(Some(expr))`) evaluates `<expr>` instead, letting the method
    /// degrade to a value (`None`, `Default::default()`, ...) rather than abort.
    /// Other helpers supply no stub.
    fn stub_body(&self, shim: &Ident, name: &Ident) -> Option<TokenStream2> {
        match self {
            Helper::Stub(None) => {
                let msg = format!("`{name}` is not available on the type-erased `{shim}` shim");
                Some(quote! { ::std::panic!(#msg) })
            }
            Helper::Stub(Some(expr)) => Some(quote! { #expr }),
            _ => None,
        }
    }

    /// Parse a method's `#[dyn_shim(...)]` attribute, which must carry exactly
    /// one supported argument.
    fn parse(attr: &Attribute) -> syn::Result<Helper> {
        let mut helper = None;
        attr.parse_nested_meta(|meta| {
            let which = if meta.path.is_ident("skip") {
                Helper::Skip
            } else if meta.path.is_ident("panic") {
                Helper::Stub(None)
            } else if meta.path.is_ident("erase") {
                Helper::Erase
            } else if meta.path.is_ident("boxed") {
                Helper::Boxed
            } else if meta.path.is_ident("stub") {
                Helper::Stub(Some(meta.value()?.parse()?))
            } else {
                return Err(meta.error(
                    "unsupported dyn_shim argument, expected `skip`, `panic`, `erase`, `boxed`, \
                     or `stub = <expr>`",
                ));
            };
            if helper.replace(which).is_some() {
                return Err(meta.error("duplicate dyn_shim argument"));
            }
            Ok(())
        })?;
        helper.ok_or_else(|| {
            syn::Error::new_spanned(
                attr,
                "expected #[dyn_shim(skip)], #[dyn_shim(panic)], #[dyn_shim(erase)], \
                 #[dyn_shim(boxed)], or #[dyn_shim(stub = <expr>)]",
            )
        })
    }

    /// The helper argument on a method's `#[dyn_shim(...)]` attribute, if any.
    /// The arguments were validated up front, so parsing cannot fail here.
    fn of(method: &TraitItemFn) -> Option<Helper> {
        let attr = method
            .attrs
            .iter()
            .find(|a| a.path().is_ident("dyn_shim"))?;
        Helper::parse(attr).ok()
    }
}

/// True if the first parameter is a `self` receiver (`&self`, `&mut self`,
/// by-value `self`, or a typed receiver such as `self: Box<Self>`).
fn has_self_receiver(sig: &Signature) -> bool {
    matches!(sig.inputs.first(), Some(FnArg::Receiver(_)))
}

/// True if the method's `where` clause requires `Self: Sized`. Such a method is
/// excluded from the vtable, so it cannot be dispatched through the shim's
/// `dyn` type even though its signature is otherwise compatible.
fn requires_self_sized(sig: &Signature) -> bool {
    let Some(where_clause) = &sig.generics.where_clause else {
        return false;
    };
    where_clause.predicates.iter().any(|pred| {
        let syn::WherePredicate::Type(pred) = pred else {
            return false;
        };
        let Type::Path(bounded) = &pred.bounded_ty else {
            return false;
        };
        if bounded.qself.is_some() || !bounded.path.is_ident("Self") {
            return false;
        }
        pred.bounds
            .iter()
            .any(|bound| matches!(bound, syn::TypeParamBound::Trait(t) if t.path.is_ident("Sized")))
    })
}

/// True if the method declares a generic type or const parameter. Lifetime
/// parameters do not count, since they are forwarded as-is.
fn has_type_or_const_generics(sig: &Signature) -> bool {
    sig.generics
        .params
        .iter()
        .any(|p| !matches!(p, GenericParam::Lifetime(_)))
}

/// True if the return type or any argument type mentions `Self`.
fn signature_mentions_self(sig: &Signature) -> bool {
    signature_any_type(sig, |ty| type_finds(ty, true, false))
}

/// True if the return type or any argument type uses `impl Trait`. Split from
/// the `Self` check because `#[dyn_shim(erase)]` can rescue `impl Trait` (by
/// lowering it to a trait object) but never `Self`.
fn signature_mentions_impl_trait(sig: &Signature) -> bool {
    signature_any_type(sig, |ty| type_finds(ty, false, true))
}

/// True if `pred` holds for the return type or any argument type after the
/// receiver.
fn signature_any_type(sig: &Signature, pred: impl Fn(&Type) -> bool) -> bool {
    let return_bad = matches!(&sig.output, ReturnType::Type(_, ty) if pred(ty));
    let arg_bad = sig
        .inputs
        .iter()
        .skip(1)
        .any(|arg| matches!(arg, FnArg::Typed(pat) if pred(&pat.ty)));
    return_bad || arg_bad
}

/// Whether a type mentions `Self` and/or uses `impl Trait`, each toggled by a
/// flag. Either makes a method non-dyn-compatible.
fn type_finds(ty: &Type, find_self: bool, find_impl_trait: bool) -> bool {
    struct Finder {
        find_self: bool,
        find_impl_trait: bool,
        hit: bool,
    }
    impl<'ast> Visit<'ast> for Finder {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if self.find_self && path.segments.iter().any(|s| s.ident == "Self") {
                self.hit = true;
            }
            visit::visit_path(self, path);
        }
        fn visit_type_impl_trait(&mut self, it: &'ast syn::TypeImplTrait) {
            if self.find_impl_trait {
                self.hit = true;
            }
            visit::visit_type_impl_trait(self, it);
        }
    }
    let mut finder = Finder {
        find_self,
        find_impl_trait,
        hit: false,
    };
    finder.visit_type(ty);
    finder.hit
}
