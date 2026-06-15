// Several methods declare lifetimes that could be elided; they are explicit on
// purpose, to exercise lifetime forwarding.
#![allow(clippy::needless_lifetimes)]

use dyn_shim::{dyn_shim, dyn_shim_foreign, dyn_shim_recognized, trait_object};
use std::fmt::Display;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

#[allow(dead_code)]
#[dyn_shim(DynTrait)]
trait SizedTrait {
    fn name(&self) -> String;
    fn bump(&mut self, by: i32);
    fn add(&self, a: i32, b: i32) -> i32;
    fn drain(&mut self) -> String;
    fn consume(self, suffix: &str) -> String;
    fn tagged<'a>(&self, tag: &'a str) -> String
    where
        'a: 'a;
    fn show<'a>(&self, x: &'a dyn Display) -> String;
    fn make() -> Self;
    fn answer() -> i32;
}

struct Foo(i32);
struct Bar(String);

impl SizedTrait for Foo {
    fn name(&self) -> String {
        format!("Foo({})", self.0)
    }
    fn bump(&mut self, by: i32) {
        self.0 += by;
    }
    fn add(&self, a: i32, b: i32) -> i32 {
        self.0 + a + b
    }
    fn drain(&mut self) -> String {
        let s = format!("drained {}", self.0);
        self.0 = 0;
        s
    }
    fn consume(self, suffix: &str) -> String {
        format!("Foo gone:{}{}", self.0, suffix)
    }
    fn tagged<'a>(&self, tag: &'a str) -> String
    where
        'a: 'a,
    {
        format!("[{}]Foo{}", tag, self.0)
    }
    fn show<'a>(&self, x: &'a dyn Display) -> String {
        format!("Foo<{}>", x)
    }
    fn make() -> Self {
        Foo(0)
    }
    fn answer() -> i32 {
        42
    }
}

impl SizedTrait for Bar {
    fn name(&self) -> String {
        format!("Bar({})", self.0)
    }
    fn bump(&mut self, by: i32) {
        self.0.push_str(&"!".repeat(by.max(0) as usize));
    }
    fn add(&self, a: i32, b: i32) -> i32 {
        self.0.len() as i32 + a + b
    }
    fn drain(&mut self) -> String {
        let s = format!("drained {}", self.0);
        self.0.clear();
        s
    }
    fn consume(self, suffix: &str) -> String {
        format!("Bar gone:{}{}", self.0, suffix)
    }
    fn tagged<'a>(&self, tag: &'a str) -> String
    where
        'a: 'a,
    {
        format!("[{}]Bar{}", tag, self.0)
    }
    fn show<'a>(&self, x: &'a dyn Display) -> String {
        format!("Bar<{}>", x)
    }
    fn make() -> Self {
        Bar(String::new())
    }
    fn answer() -> i32 {
        42
    }
}

#[test]
fn shared_ref_dispatch() {
    let r: &dyn DynTrait = &Foo(7);
    assert_eq!(r.name(), "Foo(7)");
}

#[test]
fn multi_arg() {
    let r: &dyn DynTrait = &Foo(10);
    assert_eq!(r.add(3, 4), 17);
}

#[test]
fn mut_ref_dispatch() {
    let mut b = Bar("x".into());
    let r: &mut dyn DynTrait = &mut b;
    r.bump(3);
    assert_eq!(r.name(), "Bar(x!!!)");
}

#[test]
fn mut_ref_returns_value() {
    let mut f = Foo(5);
    let r: &mut dyn DynTrait = &mut f;
    assert_eq!(r.drain(), "drained 5");
    assert_eq!(r.name(), "Foo(0)");
}

#[test]
fn box_self_consume() {
    let boxed: Box<dyn DynTrait> = Box::new(Foo(9));
    assert_eq!(boxed.consume("-end"), "Foo gone:9-end");
}

#[test]
fn lifetime_where_method() {
    let r: &dyn DynTrait = &Bar("z".into());
    assert_eq!(r.tagged("T"), "[T]Barz");
}

#[test]
fn lifetime_no_where() {
    let r: &dyn DynTrait = &Foo(1);
    assert_eq!(r.show(&99), "Foo<99>");
    assert_eq!(r.show(&"hi"), "Foo<hi>");
}

#[test]
fn mixed_box_collection() {
    let mut zoo: Vec<Box<dyn DynTrait>> = vec![Box::new(Foo(100)), Box::new(Bar("hi".into()))];
    for item in zoo.iter_mut() {
        item.bump(1);
    }
    let names: Vec<String> = zoo.iter().map(|x| x.name()).collect();
    assert_eq!(names, vec!["Foo(101)", "Bar(hi!)"]);

    let out: Vec<String> = zoo.into_iter().map(|x| x.consume("#")).collect();
    assert_eq!(out, vec!["Foo gone:101#", "Bar gone:hi!#"]);
}

// Every dispatchable receiver type is forwarded into the shim: `&self`,
// `&mut self`, and the explicit smart-pointer receivers `Box<Self>`,
// `Rc<Self>`, `Arc<Self>`, and `Pin<&mut Self>`. An explicit `self: Self`
// is the typed spelling of by-value `self` and is rewritten to
// `self: Box<Self>` just like the shorthand.
#[dyn_shim(DynRecv)]
trait Receivers {
    fn by_ref(&self) -> i32;
    fn by_mut(&mut self) -> i32;
    fn by_box(self: Box<Self>) -> i32;
    fn by_rc(self: Rc<Self>) -> i32;
    fn by_arc(self: Arc<Self>) -> i32;
    fn by_pin(self: Pin<&mut Self>) -> i32;
    fn by_self(self: Self) -> i32;
}

struct Recv(i32);
impl Receivers for Recv {
    fn by_ref(&self) -> i32 {
        self.0
    }
    fn by_mut(&mut self) -> i32 {
        self.0 += 1;
        self.0
    }
    fn by_box(self: Box<Self>) -> i32 {
        self.0 + 10
    }
    fn by_rc(self: Rc<Self>) -> i32 {
        self.0 + 20
    }
    fn by_arc(self: Arc<Self>) -> i32 {
        self.0 + 30
    }
    fn by_pin(self: Pin<&mut Self>) -> i32 {
        self.0 + 40
    }
    fn by_self(self) -> i32 {
        self.0 + 50
    }
}

#[test]
fn ref_receivers() {
    let r: &dyn DynRecv = &Recv(1);
    assert_eq!(r.by_ref(), 1);

    let mut owned = Recv(1);
    let m: &mut dyn DynRecv = &mut owned;
    assert_eq!(m.by_mut(), 2);
}

#[test]
fn box_receiver() {
    let b: Box<dyn DynRecv> = Box::new(Recv(1));
    assert_eq!(b.by_box(), 11);
}

#[test]
fn rc_receiver() {
    let rc: Rc<dyn DynRecv> = Rc::new(Recv(1));
    assert_eq!(rc.by_rc(), 21);
}

#[test]
fn arc_receiver() {
    let arc: Arc<dyn DynRecv> = Arc::new(Recv(1));
    assert_eq!(arc.by_arc(), 31);
}

#[test]
fn pin_receiver() {
    let mut pinned: Pin<Box<dyn DynRecv>> = Box::pin(Recv(1));
    assert_eq!(pinned.as_mut().by_pin(), 41);
}

#[test]
fn explicit_self_receiver() {
    let b: Box<dyn DynRecv> = Box::new(Recv(1));
    assert_eq!(b.by_self(), 51);
}

// A source trait that is itself not dyn-compatible (it carries an associated
// const and an associated type) still yields a working shim from its
// dispatchable methods. Associated items are not copied onto the shim, and the
// method that returns the associated type is skipped because it mentions Self.
#[allow(dead_code)]
#[dyn_shim(DynAssoc)]
trait HasAssoc {
    const TAG: u8;
    type Item;
    fn label(&self) -> String;
    fn item(&self) -> Self::Item;
}

struct Assoc;
impl HasAssoc for Assoc {
    const TAG: u8 = 9;
    type Item = i32;
    fn label(&self) -> String {
        "Assoc".into()
    }
    fn item(&self) -> i32 {
        1
    }
}

#[test]
fn assoc_items_trait_shimmed() {
    let d: &dyn DynAssoc = &Assoc;
    assert_eq!(d.label(), "Assoc");
}

// Bounds on the shim's name become its supertraits (and bounds on the blanket
// impl): an auto trait lets the trait object cross threads, and a re-added
// dyn-compatible supertrait is callable on the `dyn` type.
#[dyn_shim(DynTask: Send + Display)]
trait Task: Display {
    fn id(&self) -> i32;
}

struct Job(i32);
impl std::fmt::Display for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job#{}", self.0)
    }
}
impl Task for Job {
    fn id(&self) -> i32 {
        self.0
    }
}

#[test]
fn bounds_on_shim() {
    let t: Box<dyn DynTask> = Box::new(Job(4));
    assert_eq!(format!("{t}"), "job#4");
    assert_eq!(std::thread::spawn(move || t.id()).join().unwrap(), 4);
}

// A forwarded method keeps its whole signature and its attributes. `unsafe`
// and an explicit ABI are carried onto the shim, and a `#[cfg]`-gated method
// is gated identically on the source trait, the shim trait, and the blanket
// impl: `sometimes` is compiled out everywhere here, so its missing impl (and
// nonexistent argument type) must not break the build.
#[allow(dead_code)]
#[dyn_shim(DynSig)]
trait SigPreserving {
    /// Doc carried onto the shim.
    #[must_use]
    fn answer(&self) -> i32;
    unsafe fn raw(&self) -> i32;
    extern "C" fn c_abi(&self) -> i32;
    #[cfg(any())]
    fn sometimes(&self, arg: NoSuchType) -> NoSuchType;
}

struct Sig(i32);
impl SigPreserving for Sig {
    fn answer(&self) -> i32 {
        self.0
    }
    unsafe fn raw(&self) -> i32 {
        -self.0
    }
    extern "C" fn c_abi(&self) -> i32 {
        self.0 * 2
    }
}

#[test]
fn signature_and_attrs_preserved() {
    let d: Box<dyn DynSig> = Box::new(Sig(7));
    assert_eq!(d.answer(), 7);
    // The shim method is `unsafe fn`, so it must be called in an unsafe block.
    assert_eq!(unsafe { d.raw() }, -7);
    assert_eq!(d.c_abi(), 14);
}

// A recognized `Clone` bound is not a supertrait of the shim (that would
// break dyn-compatibility). It becomes a bound on the blanket impl plus
// hidden machinery that makes the shim's boxed trait objects cloneable.
#[dyn_shim(DynSticker: Clone)]
trait Sticker {
    fn label(&self) -> String;
    fn relabel(&mut self, to: &str);
}

#[derive(Clone)]
struct Tag(String);
impl Sticker for Tag {
    fn label(&self) -> String {
        self.0.clone()
    }
    fn relabel(&mut self, to: &str) {
        self.0 = to.into();
    }
}

#[test]
fn clone_bound_clones_box() {
    let original: Box<dyn DynSticker> = Box::new(Tag("a".into()));
    let mut copy = original.clone();
    copy.relabel("b");
    assert_eq!(original.label(), "a");
    assert_eq!(copy.label(), "b");
}

// The hidden clone machinery dispatches through `Clone` by name, so it works
// even when the source trait declares its own method called `clone`. The two
// never collide: the box's `Clone` impl copies the value, the shim method
// forwards to the implementor's inherent trait method.
#[dyn_shim(DynRevision: Clone)]
trait Revision {
    fn clone(&self) -> u32; // domain method, not std `Clone`
}

#[derive(Clone)]
struct Doc(u32);
impl Revision for Doc {
    fn clone(&self) -> u32 {
        self.0
    }
}

#[test]
fn clone_bound_with_clone_named_method() {
    let original: Box<dyn DynRevision> = Box::new(Doc(7));
    let copy = std::clone::Clone::clone(&original);
    assert_eq!(DynRevision::clone(&*copy), 7);
}

// Recognized bounds compose with the rest of the list: auto traits stay
// supertraits of the shim and additionally select which `dyn` marker
// combinations get the `Clone` and `Hash` machinery, in any order, alongside
// ordinary method skipping.
#[allow(dead_code)]
#[dyn_shim(DynColor: Hash + Send + Clone + Sync)]
trait Color {
    fn rgb(&self) -> (u8, u8, u8);
    fn mix(&self, other: Self) -> Self; // skipped: mentions Self
}

#[derive(Clone, Hash)]
struct Red;
impl Color for Red {
    fn rgb(&self) -> (u8, u8, u8) {
        (255, 0, 0)
    }
    fn mix(&self, _other: Self) -> Self {
        Red
    }
}

#[test]
fn clone_covers_marker_combinations() {
    let plain: Box<dyn DynColor> = Box::new(Red);
    assert_eq!(plain.clone().rgb(), (255, 0, 0));

    // The clone keeps the marker, so it can cross the thread boundary.
    let send: Box<dyn DynColor + Send> = Box::new(Red);
    let copy = send.clone();
    assert_eq!(
        std::thread::spawn(move || copy.rgb()).join().unwrap(),
        (255, 0, 0)
    );

    let sync: Box<dyn DynColor + Sync> = Box::new(Red);
    assert_eq!(sync.clone().rgb(), (255, 0, 0));

    // Marker order in the type is irrelevant; one impl covers each subset.
    let both: Box<dyn Sync + Send + DynColor> = Box::new(Red);
    assert_eq!(both.clone().rgb(), (255, 0, 0));
}

// The recognized auto traits are not limited to Send and Sync; any listed
// std auto trait selects marker combinations.
#[dyn_shim(DynGauge: Clone + Unpin)]
trait Gauge {
    fn level(&self) -> i32;
}

#[derive(Clone)]
struct Dial(i32);
impl Gauge for Dial {
    fn level(&self) -> i32 {
        self.0
    }
}

#[test]
fn clone_covers_listed_unpin_marker() {
    let pinned: Box<dyn DynGauge + Unpin> = Box::new(Dial(7));
    assert_eq!(pinned.clone().level(), 7);
}

#[test]
fn hash_matches_concrete_value() {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    let bh = BuildHasherDefault::<DefaultHasher>::default();

    let boxed: Box<dyn DynColor> = Box::new(Red);
    let by_ref: &dyn DynColor = &Red;
    assert_eq!(bh.hash_one(&*boxed), bh.hash_one(&Red));
    assert_eq!(bh.hash_one(by_ref), bh.hash_one(&Red));
    // Box<dyn DynColor> hashes via std's forwarding impl for Box<T: Hash>.
    assert_eq!(bh.hash_one(&boxed), bh.hash_one(&Red));
    // Marker-variant dyn types hash through their own impls.
    let marked: &(dyn DynColor + Send + Sync) = &Red;
    assert_eq!(bh.hash_one(marked), bh.hash_one(&Red));
}

#[test]
fn to_owned_escapes_borrow() {
    use std::borrow::Cow;

    let concrete = Red;
    let borrowed: &dyn DynColor = &concrete;
    // `.clone()` on a `&dyn` would copy the reference; `.to_owned()` returns
    // an owned box.
    let owned: Box<dyn DynColor> = borrowed.to_owned();
    assert_eq!(owned.rgb(), (255, 0, 0));

    let cow: Cow<'_, dyn DynColor> = Cow::Borrowed(borrowed);
    let owned: Box<dyn DynColor> = cow.into_owned();
    assert_eq!(owned.rgb(), (255, 0, 0));

    // Marker variants get `ToOwned` too.
    let borrowed: &(dyn DynColor + Send + Sync) = &concrete;
    let owned: Box<dyn DynColor + Send + Sync> = borrowed.to_owned();
    assert_eq!(
        std::thread::spawn(move || owned.rgb()).join().unwrap(),
        (255, 0, 0)
    );
}

// Duplicate bounds are harmless, deduplicated like the language itself
// dedupes `trait Foo: A + A`: the machinery and marker combos are generated
// once.
#[dyn_shim(DynBadge: Clone + Send + Clone + Send)]
trait Badge {
    fn number(&self) -> u32;
}

#[derive(Clone)]
struct Lanyard(u32);
impl Badge for Lanyard {
    fn number(&self) -> u32 {
        self.0
    }
}

#[test]
fn duplicate_bounds_dedupe() {
    let badge: Box<dyn DynBadge + Send> = Box::new(Lanyard(3));
    assert_eq!(badge.clone().number(), 3);
}

// A pass-through bound whose trait has associated types works when the
// `dyn` type binds them at the use site. The bound mirrors the source
// trait's supertrait, re-adding it to the shim.
#[dyn_shim(DynSamples: Iterator)]
trait Samples: Iterator {
    fn label(&self) -> String;
}

struct Ramp(u8);
impl Iterator for Ramp {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        self.0 += 1;
        Some(self.0)
    }
}
impl Samples for Ramp {
    fn label(&self) -> String {
        "ramp".into()
    }
}

#[test]
fn assoc_type_bound_binds_at_use_site() {
    let mut source: Box<dyn DynSamples<Item = u8>> = Box::new(Ramp(0));
    assert_eq!(source.label(), "ramp");
    let head: Vec<u8> = source.by_ref().take(3).collect();
    assert_eq!(head, [1, 2, 3]);
}

// `#[dyn_shim_foreign]` shims a trait the annotating crate does not own. The
// module here stands in for a dependency: the macro cannot read its body, so
// the annotated trait restates the signatures to forward (and is itself
// discarded, not re-emitted). The blanket impl forwards to the foreign path.
mod thirdparty {
    #[allow(dead_code)]
    pub trait Channel {
        fn connect() -> Self; // receiverless; not restated below
        fn label(&self) -> String;
        fn deliver(&mut self, message: &str) -> usize;
        fn close(self) -> usize; // by-value self
    }
}

struct Email(usize);
impl thirdparty::Channel for Email {
    fn connect() -> Self {
        Email(0)
    }
    fn label(&self) -> String {
        "email".into()
    }
    fn deliver(&mut self, _message: &str) -> usize {
        self.0 += 1;
        self.0
    }
    fn close(self) -> usize {
        self.0
    }
}

// The annotated trait is the shim; only the dyn-compatible methods to forward
// are listed (the receiverless `connect` is omitted). The foreign trait's path
// is the attribute argument.
#[dyn_shim_foreign(thirdparty::Channel)]
trait DynChannel {
    fn label(&self) -> String;
    fn deliver(&mut self, message: &str) -> usize;
    fn close(self) -> usize; // by-value -> self: Box<Self>
}

#[test]
fn foreign_trait_shimmed() {
    let mut ch: Box<dyn DynChannel> = Box::new(Email(0));
    assert_eq!(ch.label(), "email");
    assert_eq!(ch.deliver("hi"), 1);
    assert_eq!(ch.deliver("again"), 2);
    assert_eq!(ch.close(), 2); // by-value self forwarded through Box<Self>
}

// Recognized bounds, marker combinations, and supertraits work through the
// foreign form exactly as the local one: `Clone` makes the boxed trait
// objects cloneable, `Send` selects the marker variant, and a path to the
// foreign trait carrying generic arguments is forwarded verbatim.
mod widgets {
    pub trait Paint<C> {
        fn color(&self) -> C;
    }
}

#[derive(Clone)]
struct Square;
impl widgets::Paint<u8> for Square {
    fn color(&self) -> u8 {
        7
    }
}

#[dyn_shim_foreign(widgets::Paint<u8>)]
trait DynPaint: Clone + Send {
    fn color(&self) -> u8;
}

#[test]
fn foreign_recognized_bound_and_markers() {
    let plain: Box<dyn DynPaint> = Box::new(Square);
    assert_eq!(plain.clone().color(), 7);

    let send: Box<dyn DynPaint + Send> = Box::new(Square);
    let copy = send.clone();
    assert_eq!(std::thread::spawn(move || copy.color()).join().unwrap(), 7);
}

// `reflexive = boxed` additionally emits `impl Munch for Box<dyn DynMunch>`, so
// the boxed trait object satisfies the source trait itself and can be passed to
// code generic over `Munch`. The dyn-compatible methods forward through the
// shim (by-value `self` arrives as the shim's `Box<Self>`); `fresh` is not
// dyn-compatible, so it is opted into a panicking stub.
#[dyn_shim(DynMunch, reflexive = boxed)]
trait Munch {
    fn crunch(self) -> u32;
    fn bites(&self) -> u32;
    #[dyn_shim(panic)]
    fn fresh() -> Self;
}

struct Apple(u32);
impl Munch for Apple {
    fn crunch(self) -> u32 {
        self.0
    }
    fn bites(&self) -> u32 {
        self.0 * 2
    }
    fn fresh() -> Self {
        Apple(1)
    }
}

fn eat(m: impl Munch) -> u32 {
    m.crunch()
}
fn count(m: &impl Munch) -> u32 {
    m.bites()
}

#[test]
fn reflexive_box_satisfies_source_trait() {
    let m: Box<dyn DynMunch> = Box::new(Apple(7));
    // `&Box<dyn DynMunch>` is an `&impl Munch`: forwards `bites` through the shim.
    assert_eq!(count(&m), 14);
    // `Box<dyn DynMunch>` is an `impl Munch`: by-value `crunch` consumes the box.
    assert_eq!(eat(m), 7);
}

#[test]
#[should_panic = "not available on the type-erased `DynMunch` shim"]
fn reflexive_stub_panics_when_called() {
    fn make<T: Munch>() -> T {
        T::fresh()
    }
    let _: Box<dyn DynMunch> = make();
}

// `reflexive = bare` emits `impl Shape for dyn DynShape`, so a `&dyn DynShape`
// (or `&mut`) satisfies the source trait directly, by reference.
#[dyn_shim(DynShape, reflexive = bare)]
trait Shape {
    fn area(&self) -> u32;
    fn grow(&mut self, by: u32);
}

struct Sq(u32);
impl Shape for Sq {
    fn area(&self) -> u32 {
        self.0 * self.0
    }
    fn grow(&mut self, by: u32) {
        self.0 += by;
    }
}

fn measure<T: Shape + ?Sized>(s: &T) -> u32 {
    s.area()
}

#[test]
fn reflexive_bare_satisfies_by_ref() {
    let mut sq = Sq(3);
    {
        let d: &mut dyn DynShape = &mut sq;
        Shape::grow(d, 1);
    }
    let d: &dyn DynShape = &sq;
    assert_eq!(measure(d), 16);
}

// `reflexive = bare + boxed` emits both impls, so a borrow (`&dyn DynGreeter`)
// and an owned box (`Box<dyn DynGreeter>`) each satisfy the source trait.
#[dyn_shim(DynGreeter, reflexive = bare + boxed)]
trait Greeter {
    fn greet(&self) -> String;
}

struct Hi;
impl Greeter for Hi {
    fn greet(&self) -> String {
        "hi".into()
    }
}

fn by_ref<T: Greeter + ?Sized>(g: &T) -> String {
    g.greet()
}
fn by_val(g: impl Greeter) -> String {
    g.greet()
}

#[test]
fn reflexive_both_satisfies_ref_and_box() {
    let b: Box<dyn DynGreeter> = Box::new(Hi);
    assert_eq!(by_ref(&*b), "hi"); // &dyn DynGreeter: Greeter (bare)
    assert_eq!(by_val(b), "hi"); // Box<dyn DynGreeter>: Greeter (boxed)
}

// `reflexive` works through the foreign form too: a `Box<dyn DynBrew>` is made
// to satisfy the foreign `cafe::Brew`, so it can be handed to code in (or
// generic over) the dependency that expects `impl Brew`, including the
// by-value `finish` that the dependency cannot dispatch itself.
mod cafe {
    pub trait Brew {
        fn sip(&self) -> u32;
        fn finish(self) -> u32;
    }
}

struct Cup(u32);
impl cafe::Brew for Cup {
    fn sip(&self) -> u32 {
        self.0
    }
    fn finish(self) -> u32 {
        self.0 + 1
    }
}

#[dyn_shim_foreign(cafe::Brew, reflexive = boxed)]
trait DynBrew {
    fn sip(&self) -> u32;
    fn finish(self) -> u32; // by-value -> self: Box<Self>
}

fn taste(b: &impl cafe::Brew) -> u32 {
    b.sip()
}
fn drink(b: impl cafe::Brew) -> u32 {
    b.finish()
}

#[test]
fn foreign_reflexive_satisfies_source() {
    let b: Box<dyn DynBrew> = Box::new(Cup(4));
    assert_eq!(taste(&b), 4);
    assert_eq!(drink(b), 5);
}

// `dyn_shim_recognized(Clone)` exposes std `Clone` as a standalone
// dyn-compatible shim, the way the `dyn_clone` crate's `DynClone` does:
// `impl<T: Clone> DynCloneable for T` is generated, and `Box<dyn DynCloneable>`
// is itself `Clone`, cloning the underlying concrete value. The listed `Send`
// marker covers the `+ Send` variant too.
#[dyn_shim_recognized(Clone + Send)]
trait DynCloneable {}

static CLONES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Tracked;
impl Clone for Tracked {
    fn clone(&self) -> Self {
        CLONES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Tracked
    }
}

#[test]
fn recognized_clone_standalone() {
    use std::sync::atomic::Ordering::SeqCst;

    let a: Box<dyn DynCloneable> = Box::new(Tracked);
    let before = CLONES.load(SeqCst);
    let _b = a.clone(); // Box<dyn DynCloneable>: Clone -> Tracked::clone
    assert_eq!(CLONES.load(SeqCst), before + 1);

    // The `+ Send` marker variant is cloneable and crosses thread boundaries.
    let s: Box<dyn DynCloneable + Send> = Box::new(Tracked);
    let s2 = s.clone();
    std::thread::spawn(move || drop(s2)).join().unwrap();

    // `dyn DynCloneable` gets `ToOwned`, so a borrow can escape as an owned box.
    let owned: Box<dyn DynCloneable> = (*a).to_owned();
    drop(owned);
}

// `dyn_shim_recognized(Hash)` exposes std `Hash`: `dyn DynHashable` implements
// `Hash`, hashing like the underlying concrete value, and through std's
// forwarding impl so does `Box<dyn DynHashable>`.
#[dyn_shim_recognized(Hash)]
trait DynHashable {}

#[test]
fn recognized_hash_standalone() {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    let bh = BuildHasherDefault::<DefaultHasher>::default();

    let boxed: Box<dyn DynHashable> = Box::new(42u32);
    let by_ref: &dyn DynHashable = &42u32;
    let expected = bh.hash_one(42u32);
    assert_eq!(bh.hash_one(&*boxed), expected);
    assert_eq!(bh.hash_one(by_ref), expected);
    // Different values hash differently.
    assert_ne!(bh.hash_one(by_ref), bh.hash_one(7u32));
}

// The crate's ready-made `DynClone` shim (behind the `dyn_clone` feature): a boxed
// trait object is cloneable, the marker variants are covered, and a borrow can
// escape as an owned box through `ToOwned`.
#[cfg(feature = "dyn_clone")]
#[test]
fn shipped_dyn_clone() {
    use dyn_shim::DynClone;

    #[derive(Clone)]
    struct Widget;

    let a: Box<dyn DynClone + Send + Sync> = Box::new(Widget);
    let b = a.clone();
    std::thread::spawn(move || drop(b)).join().unwrap();

    let r: &dyn DynClone = &Widget;
    let owned: Box<dyn DynClone> = r.to_owned();
    drop(owned);
}

// With the `dyn_clone` feature on, a recognized `Clone` bound makes the shim a
// subtrait of the standalone `DynClone`, so its boxed trait object upcasts to
// `Box<dyn DynClone>` and flows into DynClone-typed APIs, while still cloning
// as its own `Box<dyn DynSticker>`.
#[cfg(feature = "dyn_clone")]
#[test]
fn clone_bound_shim_upcasts_to_dyn_clone() {
    fn keep(_: Box<dyn dyn_shim::DynClone>) {}

    let a: Box<dyn DynSticker> = Box::new(Tag("a".into()));
    let copy = a.clone(); // still clones as Box<dyn DynSticker>
    assert_eq!(copy.label(), "a");

    let erased: Box<dyn dyn_shim::DynClone> = a; // trait upcast
    keep(erased.clone());
}

// The crate's ready-made `DynHash` shim (behind the `dyn_hash` feature): the boxed
// trait object hashes like the underlying concrete value.
#[cfg(feature = "dyn_hash")]
#[test]
fn shipped_dyn_hash() {
    use dyn_shim::DynHash;
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

    let bh = BuildHasherDefault::<DefaultHasher>::default();
    let boxed: Box<dyn DynHash> = Box::new(99u32);
    assert_eq!(bh.hash_one(&*boxed), bh.hash_one(99u32));
}

// With the `dyn_hash` feature on, a recognized `Hash` bound makes the shim a
// subtrait of the standalone `DynHash`, so its trait object upcasts to
// `&dyn DynHash` and hashes like the concrete value.
#[cfg(feature = "dyn_hash")]
#[test]
fn hash_bound_shim_upcasts_to_dyn_hash() {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

    let bh = BuildHasherDefault::<DefaultHasher>::default();
    let color: &dyn DynColor = &Red;
    let erased: &dyn dyn_shim::DynHash = color; // trait upcast
    assert_eq!(bh.hash_one(erased), bh.hash_one(&Red));
}

// `#[trait_object(Hash)]` bolts the recognized `Hash` capability onto a trait
// the user owns that is already dyn-compatible, generating no shim. The trait
// carries the `DynHash` carrier as an explicit supertrait, and the attribute
// emits `impl Hash for dyn Tagged`, which also covers `&dyn Tagged` and (through
// std's forwarding impl) `Box<dyn Tagged>`.
#[cfg(feature = "dyn_hash")]
mod trait_object_hash {
    use dyn_shim::{DynHash, trait_object};
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

    #[trait_object(DynHash)]
    trait Tagged: DynHash {
        fn tag(&self) -> u32;
    }

    #[derive(Hash)]
    struct Label(u32);
    impl Tagged for Label {
        fn tag(&self) -> u32 {
            self.0
        }
    }

    #[test]
    fn dyn_trait_is_hash() {
        let bh = BuildHasherDefault::<DefaultHasher>::default();
        let boxed: Box<dyn Tagged> = Box::new(Label(5));
        let by_ref: &dyn Tagged = &Label(5);
        // The trait's own methods still work; the attribute only adds the impl.
        assert_eq!(boxed.tag(), 5);
        assert_eq!(bh.hash_one(&*boxed), bh.hash_one(&Label(5)));
        assert_eq!(bh.hash_one(by_ref), bh.hash_one(&Label(5)));
        // Box<dyn Tagged> hashes via std's forwarding impl for Box<T: Hash>.
        assert_eq!(bh.hash_one(&boxed), bh.hash_one(&Label(5)));
        assert_ne!(bh.hash_one(by_ref), bh.hash_one(&Label(6)));
    }
}

// `#[trait_object(Clone)]` makes `Box<dyn Drawing>` cloneable and `dyn Drawing`
// `ToOwned`, cloning the underlying concrete value into a fresh box. The carrier
// is the `DynClone` supertrait.
#[cfg(feature = "dyn_clone")]
mod trait_object_clone {
    use dyn_shim::{DynClone, trait_object};

    #[trait_object(DynClone)]
    trait Drawing: DynClone {
        fn area(&self) -> u32;
    }

    #[derive(Clone)]
    struct Rect {
        w: u32,
        h: u32,
    }
    impl Drawing for Rect {
        fn area(&self) -> u32 {
            self.w * self.h
        }
    }

    #[test]
    fn boxed_dyn_trait_clones() {
        let original: Box<dyn Drawing> = Box::new(Rect { w: 2, h: 3 });
        let copy = original.clone();
        assert_eq!(copy.area(), 6);
        // The clone is an independent allocation: dropping the original leaves
        // the copy intact.
        drop(original);
        assert_eq!(copy.area(), 6);
    }

    #[test]
    fn dyn_trait_to_owned_escapes_borrow() {
        use std::borrow::Cow;

        let rect = Rect { w: 4, h: 5 };
        let borrowed: &dyn Drawing = &rect;
        let owned: Box<dyn Drawing> = borrowed.to_owned();
        assert_eq!(owned.area(), 20);

        let cow: Cow<'_, dyn Drawing> = Cow::Borrowed(borrowed);
        assert_eq!(cow.into_owned().area(), 20);
    }
}

// `#[trait_object(DynRecipe)]` mounts a `#[dyn_shim]` shim's source trait onto a
// *different* dyn-compatible trait the user owns. `Recipe` is not dyn-compatible
// (its `portion` method is generic), so it cannot be a supertrait of `Dish`.
// Instead `Dish` inherits the generated `DynRecipe` shim, and the attribute
// invokes the shim's mount macro to emit `impl Recipe for dyn Dish` and
// `impl Recipe for Box<dyn Dish>`, forwarding the dyn-compatible methods through
// the shim. So a `&dyn Dish` and an owned `Box<dyn Dish>` each satisfy `Recipe`,
// even though `dyn Dish` carries it through the `DynRecipe` carrier, not as a
// supertrait of `Recipe` itself.
#[dyn_shim(DynRecipe)]
trait Recipe {
    fn calories(&self) -> u32;
    fn scaled(&self, factor: u32) -> u32;
    #[dyn_shim(panic)]
    fn portion<U: From<u32>>(&self) -> U; // generic: not dyn-compatible
}

#[trait_object(DynRecipe)]
trait Dish: DynRecipe {
    fn name(&self) -> &str;
}

struct Pasta;
impl Recipe for Pasta {
    fn calories(&self) -> u32 {
        400
    }
    fn scaled(&self, factor: u32) -> u32 {
        400 * factor
    }
    fn portion<U: From<u32>>(&self) -> U {
        U::from(Recipe::calories(self))
    }
}
impl Dish for Pasta {
    fn name(&self) -> &str {
        "pasta"
    }
}

fn total(r: &(impl Recipe + ?Sized)) -> u32 {
    r.calories()
}
fn consume(r: impl Recipe) -> u32 {
    r.scaled(2)
}

#[test]
fn trait_object_mounts_shim_source_onto_principal() {
    let dish: Box<dyn Dish> = Box::new(Pasta);
    assert_eq!(dish.name(), "pasta"); // Dish's own method still works
    // `&dyn Dish` satisfies `Recipe` by reference (the bare mount).
    assert_eq!(total(&*dish), 400);
    // The generic `portion` lives only on the concrete type (it is a panicking
    // stub on the erased mount); call it before erasing.
    assert_eq!(Recipe::portion::<u32>(&Pasta), 400);
    // `Box<dyn Dish>` satisfies `Recipe` by value (the boxed mount).
    assert_eq!(consume(dish), 800);
}

// Hash and Clone combine in one attribute, and listed auto traits select the
// covered `dyn` marker variants, exactly as for a recognized bound.
#[cfg(all(feature = "dyn_clone", feature = "dyn_hash"))]
mod trait_object_combined {
    use dyn_shim::{DynClone, DynHash, trait_object};
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};

    #[trait_object(DynHash + DynClone + Send + Sync)]
    trait Node: DynHash + DynClone {
        fn id(&self) -> u32;
    }

    #[derive(Hash, Clone)]
    struct Leaf(u32);
    impl Node for Leaf {
        fn id(&self) -> u32 {
            self.0
        }
    }

    #[test]
    fn plain_dyn_is_hash_and_clone() {
        let bh = BuildHasherDefault::<DefaultHasher>::default();
        let boxed: Box<dyn Node> = Box::new(Leaf(3));
        assert_eq!(boxed.clone().id(), 3);
        assert_eq!(bh.hash_one(&*boxed), bh.hash_one(&Leaf(3)));
    }

    #[test]
    fn marker_variants_covered() {
        let bh = BuildHasherDefault::<DefaultHasher>::default();

        // `+ Send` clones and crosses a thread boundary.
        let send: Box<dyn Node + Send> = Box::new(Leaf(4));
        let copy = send.clone();
        assert_eq!(std::thread::spawn(move || copy.id()).join().unwrap(), 4);

        // `+ Send + Sync`, in any order, hashes through its own impl.
        let both: &(dyn Node + Sync + Send) = &Leaf(4);
        assert_eq!(bh.hash_one(both), bh.hash_one(&Leaf(4)));

        // `+ Sync` is cloneable too.
        let sync: Box<dyn Node + Sync> = Box::new(Leaf(4));
        assert_eq!(sync.clone().id(), 4);
    }
}
