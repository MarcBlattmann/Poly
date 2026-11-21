//! The runtime's tables: selectors, classes and their methods, categories,
//! protocols and images, method lookup and the `+load` order. Everything
//! here is data: calling into the program (`+load`, `+initialize`,
//! resolvers, `-dealloc`) is the caller's business, with this module saying
//! what to call.
//!
//! Classes are kept by the address of their class object. A selector is
//! the address of its canonical name string, so `sel_getName` is the
//! identity and two selectors are equal when their addresses are. A
//! `Method` is the address of a [`MethodRec`] the runtime owns (stable
//! while the program runs).

use crate::Mem;
use crate::abi;
use crate::image::{ObjcSections, objc_sections};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

pub type Sel = u64;
pub type Imp = u64;

/// One method the runtime knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodRec {
    pub sel: Sel,
    /// The address of the type string (an empty string for none).
    pub types: u64,
    pub imp: Imp,
    /// Whether a category brought it (it does not count as the class's own
    /// `+load`).
    pub category: bool,
}

/// Where a class is in `+initialize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Init {
    No,
    /// Being initialized by thread `tid`.
    Running(u64),
    Done,
}

/// An instance variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ivar {
    pub name: u64,
    pub types: u64,
    pub offset: u64,
    pub size: u64,
}

/// One class or metaclass.
#[derive(Clone, Debug)]
pub struct Class {
    pub addr: u64,
    pub meta: bool,
    /// The address of the name string.
    pub name: u64,
    pub isa: u64,
    pub superclass: u64,
    pub instance_size: u64,
    /// The methods, in lookup order (categories first, the last attached
    /// first), as `Method` handles.
    pub methods: Vec<u64>,
    pub protocols: Vec<u64>,
    /// Found by name (compiled, or a run-time class after registration).
    pub registered: bool,
    /// The image that defines it, by index; `None` for run-time classes.
    pub image: Option<usize>,
    pub init: Init,
    pub ivars: Vec<Ivar>,
}

/// An image the runtime has mapped.
#[derive(Clone, Copy, Debug)]
pub struct Image {
    pub header: u64,
    /// The address of the path string.
    pub path: u64,
}

/// A `+load` call: the receiver (a class object) and the implementation.
pub type LoadCall = (u64, Imp);

/// The tables.
#[derive(Debug, Default)]
pub struct Runtime {
    sels: BTreeMap<Vec<u8>, Sel>,
    classes: BTreeMap<u64, Class>,
    by_name: BTreeMap<Vec<u8>, u64>,
    methods: BTreeMap<u64, Box<MethodRec>>,
    cache: BTreeMap<(u64, Sel), Imp>,
    resolved: BTreeSet<(u64, Sel)>,
    protocols: BTreeMap<Vec<u8>, u64>,
    images: Vec<Image>,
    load_done: BTreeSet<u64>,
    cats_done: BTreeSet<u64>,
    /// Classes in the order they were registered (an image's in its class
    /// list's order).
    order: Vec<u64>,
    /// Categories whose class is not known yet: (class, category, image).
    pending: Vec<(u64, u64, usize)>,
    empty: u64,
}

impl Runtime {
    pub fn new() -> Runtime {
        Runtime::default()
    }

    // ---- selectors ------------------------------------------------------

    /// The selector for `name`, registering it with a fresh copy if new.
    pub fn sel(&mut self, m: &mut dyn Mem, name: &[u8]) -> Sel {
        if let Some(&s) = self.sels.get(name) {
            return s;
        }
        let mut z = name.to_vec();
        z.push(0);
        let at = m.alloc(&z);
        self.sels.insert(name.to_vec(), at);
        at
    }

    /// The selector for the name string at `at`, which becomes canonical
    /// if the name is new (image strings live as long as the program).
    pub fn sel_at(&mut self, m: &dyn Mem, at: u64) -> Sel {
        let name = m.cstr(at);
        *self.sels.entry(name).or_insert(at)
    }

    /// The selector of `name` if it is registered.
    pub fn sel_lookup(&self, name: &[u8]) -> Option<Sel> {
        self.sels.get(name).copied()
    }

    /// An empty string (type encodings given as NULL).
    fn empty(&mut self, m: &mut dyn Mem) -> u64 {
        if self.empty == 0 {
            self.empty = m.alloc(&[0]);
        }
        self.empty
    }

    // ---- methods --------------------------------------------------------

    fn new_method(&mut self, rec: MethodRec) -> u64 {
        let b = Box::new(rec);
        let h = &*b as *const MethodRec as u64;
        self.methods.insert(h, b);
        h
    }

    /// The method a handle names.
    pub fn method(&self, h: u64) -> Option<&MethodRec> {
        self.methods.get(&h).map(|b| &**b)
    }

    fn methods_from_list(&mut self, m: &dyn Mem, list: u64, category: bool) -> Vec<u64> {
        abi::method_list(m, list)
            .into_iter()
            .filter(|e| e.name != 0)
            .map(|e| {
                let sel = self.sel_at(m, e.name);
                self.new_method(MethodRec { sel, types: e.types, imp: e.imp, category })
            })
            .collect()
    }

    // ---- classes --------------------------------------------------------

    pub fn class(&self, cls: u64) -> Option<&Class> {
        self.classes.get(&cls)
    }

    pub fn class_mut(&mut self, cls: u64) -> Option<&mut Class> {
        self.classes.get_mut(&cls)
    }

    /// The class whose metaclass is `meta`.
    pub fn class_for_meta(&self, meta: u64) -> Option<u64> {
        self.classes.values().find(|c| !c.meta && c.isa == meta).map(|c| c.addr)
    }

    /// The registered class named `name`.
    pub fn class_named(&self, name: &[u8]) -> Option<u64> {
        self.by_name.get(name).copied()
    }

    /// The name of a class (the bytes of its name string).
    pub fn class_name(&self, m: &dyn Mem, cls: u64) -> Option<Vec<u8>> {
        self.classes.get(&cls).map(|c| m.cstr(c.name))
    }

    /// Registered classes (not metaclasses), in name order.
    pub fn class_list(&self) -> Vec<u64> {
        self.by_name.values().copied().collect()
    }

    /// Read the compiled class object at `cls` (and its metaclass) into
    /// the tables. Returns false for a duplicate name (the first stays).
    pub fn add_compiled_class(&mut self, m: &dyn Mem, cls: u64, image: Option<usize>) -> bool {
        if self.classes.contains_key(&cls) {
            return true;
        }
        let name = m.u64(abi::class_ro(m, cls) + abi::RO_NAME);
        let name_bytes = m.cstr(name);
        if self.by_name.contains_key(&name_bytes) {
            return false;
        }
        let meta = m.u64(cls + abi::CLASS_ISA);
        self.read_class(m, cls, image);
        if meta != 0 && !self.classes.contains_key(&meta) {
            self.read_class(m, meta, image);
        }
        self.by_name.insert(name_bytes, cls);
        self.order.push(cls);
        // Categories that waited for this class.
        let waiting: Vec<(u64, u64, usize)> = self.pending.iter().copied().filter(|p| p.0 == cls).collect();
        self.pending.retain(|p| p.0 != cls);
        for (_, cat, img) in waiting {
            self.attach_category(m, cat, img);
        }
        true
    }

    fn read_class(&mut self, m: &dyn Mem, cls: u64, image: Option<usize>) {
        let ro = abi::class_ro(m, cls);
        let flags = m.u32(ro + abi::RO_FLAGS);
        let methods = self.methods_from_list(m, m.u64(ro + abi::RO_BASE_METHODS), false);
        let protocols = abi::protocol_list(m, m.u64(ro + abi::RO_BASE_PROTOCOLS));
        let ivars = abi::ivar_list(m, m.u64(ro + abi::RO_IVARS))
            .into_iter()
            .map(|i| Ivar {
                name: i.name,
                types: i.types,
                offset: if i.offset_ptr == 0 { 0 } else { u64::from(m.u32(i.offset_ptr)) },
                size: u64::from(i.size),
            })
            .collect();
        self.classes.insert(
            cls,
            Class {
                addr: cls,
                meta: flags & abi::RO_META != 0,
                name: m.u64(ro + abi::RO_NAME),
                isa: m.u64(cls + abi::CLASS_ISA),
                superclass: m.u64(cls + abi::CLASS_SUPERCLASS),
                instance_size: u64::from(m.u32(ro + abi::RO_INSTANCE_SIZE)),
                methods,
                protocols,
                registered: true,
                image,
                init: Init::No,
                ivars,
            },
        );
        self.cache.clear();
    }

    /// Attach the category at `cat` to its class (or wait for the class).
    fn attach_category(&mut self, m: &dyn Mem, cat: u64, image: usize) {
        let cls = m.u64(cat + abi::CAT_CLASS);
        if cls == 0 {
            return;
        }
        if !self.classes.contains_key(&cls) {
            self.pending.push((cls, cat, image));
            return;
        }
        let inst = self.methods_from_list(m, m.u64(cat + abi::CAT_INSTANCE_METHODS), true);
        let class_methods = self.methods_from_list(m, m.u64(cat + abi::CAT_CLASS_METHODS), true);
        let protos = abi::protocol_list(m, m.u64(cat + abi::CAT_PROTOCOLS));
        let meta = self.classes[&cls].isa;
        if let Some(c) = self.classes.get_mut(&cls) {
            c.methods.splice(0..0, inst);
            c.protocols.extend(protos);
        }
        if let Some(c) = self.classes.get_mut(&meta) {
            c.methods.splice(0..0, class_methods);
        }
        self.cache.clear();
    }

    // ---- images ---------------------------------------------------------

    /// The images mapped so far.
    pub fn images(&self) -> &[Image] {
        &self.images
    }

    /// Map the image with its Mach-O header at `header` (`path`: the
    /// address of its path string): unique its selector references,
    /// register its protocols, classes and categories. Returns the `+load`
    /// calls to make, in order, and the image's sections.
    pub fn map_image(&mut self, m: &mut dyn Mem, header: u64, path: u64) -> (Vec<LoadCall>, ObjcSections) {
        let Some(s) = objc_sections(m, header) else { return (Vec::new(), ObjcSections::default()) };
        let index = self.images.len();
        self.images.push(Image { header, path });
        if s.is_empty() {
            return (Vec::new(), s);
        }
        for slot in s.selrefs.pointers() {
            let at = m.u64(slot);
            if at == 0 {
                continue;
            }
            let sel = self.sel_at(m, at);
            if sel != at {
                m.write_u64(slot, sel);
            }
        }
        for slot in s.protolist.pointers() {
            let p = m.u64(slot);
            if p != 0 {
                let name = m.cstr(m.u64(p + abi::PROTO_NAME));
                self.protocols.entry(name).or_insert(p);
            }
        }
        for slot in s.classlist.pointers() {
            let cls = m.u64(slot);
            if cls != 0 {
                self.add_compiled_class(m, cls, Some(index));
            }
        }
        for slot in s.catlist.pointers() {
            let cat = m.u64(slot);
            if cat != 0 {
                self.attach_category(m, cat, index);
            }
        }
        let plan = self.load_plan(m, &s);
        (plan, s)
    }

    /// The class's own `+load` (not a category's, not inherited).
    fn own_load(&self, cls: u64) -> Option<Imp> {
        let load = self.sel_lookup(b"load")?;
        let meta = self.classes.get(&cls)?.isa;
        self.classes.get(&meta)?.methods.iter().find_map(|h| {
            let r = &self.methods[h];
            (r.sel == load && !r.category).then_some(r.imp)
        })
    }

    /// A category's `+load`.
    fn category_load(&self, m: &dyn Mem, cat: u64) -> Option<Imp> {
        let load = self.sel_lookup(b"load")?;
        abi::method_list(m, m.u64(cat + abi::CAT_CLASS_METHODS))
            .into_iter()
            .find(|e| e.name != 0 && m.cstr(e.name) == m.cstr(load))
            .map(|e| e.imp)
    }

    /// The `+load` order of an image (the `_objc_load_image` sheet): each
    /// class with `+load` after its superclasses, in class-list order; a
    /// superclass without `+load` of its own that the chain passes runs
    /// its categories' `+load` there; then the other categories in
    /// category-list order.
    fn load_plan(&mut self, m: &dyn Mem, s: &ObjcSections) -> Vec<LoadCall> {
        let cats: Vec<u64> = s.nlcatlist.pointers().map(|p| m.u64(p)).filter(|&c| c != 0).collect();
        let mut plan = Vec::new();
        for slot in s.nlclslist.pointers() {
            let cls = m.u64(slot);
            if cls == 0 || !self.classes.contains_key(&cls) || self.load_done.contains(&cls) {
                continue;
            }
            self.load_done.insert(cls);
            let sup = self.classes[&cls].superclass;
            self.load_chain(m, sup, &cats, &mut plan);
            if let Some(imp) = self.own_load(cls) {
                plan.push((cls, imp));
            }
        }
        for cat in cats {
            if self.cats_done.insert(cat) {
                let cls = m.u64(cat + abi::CAT_CLASS);
                if let Some(imp) = self.category_load(m, cat).filter(|_| cls != 0) {
                    plan.push((cls, imp));
                }
            }
        }
        plan
    }

    fn load_chain(&mut self, m: &dyn Mem, cls: u64, cats: &[u64], plan: &mut Vec<LoadCall>) {
        // Iterative: collect the chain up to a class already done.
        let mut chain = Vec::new();
        let mut c = cls;
        while c != 0 && self.classes.contains_key(&c) && !self.load_done.contains(&c) && chain.len() < 1024 {
            chain.push(c);
            c = self.classes[&c].superclass;
        }
        for &c in chain.iter().rev() {
            self.load_done.insert(c);
            if let Some(imp) = self.own_load(c) {
                plan.push((c, imp));
                continue;
            }
            for &cat in cats {
                if m.u64(cat + abi::CAT_CLASS) == c && !self.cats_done.contains(&cat) {
                    self.cats_done.insert(cat);
                    if let Some(imp) = self.category_load(m, cat) {
                        plan.push((c, imp));
                    }
                }
            }
        }
    }

    // ---- lookup ---------------------------------------------------------

    /// The method for `sel` in `cls` or its superclasses.
    pub fn find_method(&self, cls: u64, sel: Sel) -> Option<u64> {
        let mut c = cls;
        let mut guard = 0;
        while c != 0 && guard < 4096 {
            let class = self.classes.get(&c)?;
            if let Some(&h) = class.methods.iter().find(|h| self.methods[*h].sel == sel) {
                return Some(h);
            }
            c = class.superclass;
            guard += 1;
        }
        None
    }

    /// The method `cls` itself defines for `sel`.
    pub fn own_method(&self, cls: u64, sel: Sel) -> Option<u64> {
        self.classes.get(&cls)?.methods.iter().copied().find(|h| self.methods[h].sel == sel)
    }

    /// The implementation a message `sel` to an instance of `cls` runs
    /// (cached until the tables change).
    pub fn lookup(&mut self, cls: u64, sel: Sel) -> Option<Imp> {
        if let Some(&imp) = self.cache.get(&(cls, sel)) {
            return Some(imp);
        }
        let imp = self.methods[&self.find_method(cls, sel)?].imp;
        self.cache.insert((cls, sel), imp);
        Some(imp)
    }

    /// Whether the resolver should be asked about `sel` for `cls` (the
    /// first time only).
    pub fn should_resolve(&mut self, cls: u64, sel: Sel) -> bool {
        self.resolved.insert((cls, sel))
    }

    /// `class_addMethod`: false if `cls` itself has the selector.
    pub fn add_method(&mut self, m: &mut dyn Mem, cls: u64, sel: Sel, imp: Imp, types: Option<&[u8]>) -> bool {
        if !self.classes.contains_key(&cls) || self.own_method(cls, sel).is_some() {
            return false;
        }
        let types = match types {
            Some(t) => {
                let mut z = t.to_vec();
                z.push(0);
                m.alloc(&z)
            }
            None => self.empty(m),
        };
        let h = self.new_method(MethodRec { sel, types, imp, category: false });
        if let Some(c) = self.classes.get_mut(&cls) {
            c.methods.push(h);
        }
        self.cache.clear();
        true
    }

    /// `class_replaceMethod`: the previous IMP of the class's own method,
    /// or 0 when the method was added.
    pub fn replace_method(&mut self, m: &mut dyn Mem, cls: u64, sel: Sel, imp: Imp, types: Option<&[u8]>) -> Imp {
        if let Some(h) = self.own_method(cls, sel) {
            return self.set_imp(h, imp);
        }
        self.add_method(m, cls, sel, imp, types);
        0
    }

    /// `method_setImplementation`: the previous IMP.
    pub fn set_imp(&mut self, h: u64, imp: Imp) -> Imp {
        let Some(r) = self.methods.get_mut(&h) else { return 0 };
        let old = r.imp;
        r.imp = imp;
        self.cache.clear();
        old
    }

    /// `method_exchangeImplementations`.
    pub fn exchange(&mut self, a: u64, b: u64) {
        let (Some(ia), Some(ib)) = (self.method(a).map(|r| r.imp), self.method(b).map(|r| r.imp)) else { return };
        self.set_imp(a, ib);
        self.set_imp(b, ia);
    }

    /// Whether `cls` is `other` or inherits from it.
    pub fn is_subclass(&self, cls: u64, other: u64) -> bool {
        let mut c = cls;
        let mut guard = 0;
        while c != 0 && guard < 4096 {
            if c == other {
                return true;
            }
            c = self.classes.get(&c).map_or(0, |k| k.superclass);
            guard += 1;
        }
        false
    }

    // ---- run-time classes ----------------------------------------------

    /// `objc_allocateClassPair`, with the class and metaclass objects
    /// already allocated at `cls` and `meta` (zeroed, `abi::CLASS_SIZE`
    /// bytes each) and the name copied to `name`. Writes their isa and
    /// superclass words. False if the name is taken or the superclass is
    /// not registered.
    pub fn allocate_class_pair(&mut self, m: &mut dyn Mem, superclass: u64, name: u64, cls: u64, meta: u64) -> bool {
        let name_bytes = m.cstr(name);
        if self.by_name.contains_key(&name_bytes) {
            return false;
        }
        let (super_meta, root_meta, size) = if superclass == 0 {
            (cls, meta, 8)
        } else {
            let Some(s) = self.classes.get(&superclass).filter(|s| s.registered) else { return false };
            let sm = s.isa;
            let root = self.classes.get(&sm).map_or(sm, |k| k.isa);
            (sm, root, s.instance_size)
        };
        m.write_u64(cls + abi::CLASS_ISA, meta);
        m.write_u64(cls + abi::CLASS_SUPERCLASS, superclass);
        m.write_u64(meta + abi::CLASS_ISA, root_meta);
        m.write_u64(meta + abi::CLASS_SUPERCLASS, super_meta);
        let base = Class {
            addr: cls,
            meta: false,
            name,
            isa: meta,
            superclass,
            instance_size: size,
            methods: Vec::new(),
            protocols: Vec::new(),
            registered: false,
            image: None,
            init: Init::No,
            ivars: Vec::new(),
        };
        let metaclass = Class {
            addr: meta,
            meta: true,
            isa: root_meta,
            superclass: super_meta,
            instance_size: abi::CLASS_SIZE,
            ..base.clone()
        };
        self.classes.insert(cls, base);
        self.classes.insert(meta, metaclass);
        true
    }

    /// `class_addIvar` (before registration only).
    pub fn add_ivar(&mut self, cls: u64, name: u64, types: u64, size: u64, align_log2: u8) -> bool {
        let Some(c) = self.classes.get_mut(&cls) else { return false };
        if c.registered || c.meta || c.ivars.iter().any(|i| i.name == name) {
            return false;
        }
        let align = 1u64 << align_log2.min(12);
        let offset = c.instance_size.next_multiple_of(align);
        c.ivars.push(Ivar { name, types, offset, size });
        c.instance_size = offset + size;
        true
    }

    /// `objc_registerClassPair`.
    pub fn register_class_pair(&mut self, m: &dyn Mem, cls: u64) {
        let Some(c) = self.classes.get_mut(&cls) else { return };
        if c.registered {
            return;
        }
        c.registered = true;
        // The instance size is fixed at a whole number of pointers (the
        // allocateClassPair visible test: 8 + 4 grew to 16).
        c.instance_size = c.instance_size.next_multiple_of(8);
        let name = c.name;
        self.by_name.insert(m.cstr(name), cls);
        self.order.push(cls);
    }

    /// The classes image `image` defines, in its class list's order.
    pub fn classes_of_image(&self, image: usize) -> Vec<u64> {
        self.order.iter().copied().filter(|c| self.classes.get(c).is_some_and(|k| k.image == Some(image))).collect()
    }

    /// The instance variable `name` of `cls` or a superclass.
    pub fn find_ivar(&self, m: &dyn Mem, cls: u64, name: &[u8]) -> Option<Ivar> {
        let mut c = cls;
        let mut guard = 0;
        while c != 0 && guard < 4096 {
            let k = self.classes.get(&c)?;
            if let Some(i) = k.ivars.iter().find(|i| m.cstr(i.name) == name) {
                return Some(*i);
            }
            c = k.superclass;
            guard += 1;
        }
        None
    }

    // ---- protocols ------------------------------------------------------

    /// The protocol named `name`.
    pub fn protocol_named(&self, name: &[u8]) -> Option<u64> {
        self.protocols.get(name).copied()
    }

    /// Whether protocol `p` is `other` or includes it (by name, through
    /// the protocols it extends).
    pub fn protocol_includes(&self, m: &dyn Mem, p: u64, other: u64) -> bool {
        let want = m.cstr(m.u64(other + abi::PROTO_NAME));
        let mut todo = alloc::vec![p];
        let mut seen = 0;
        while let Some(q) = todo.pop() {
            seen += 1;
            if seen > 4096 {
                break;
            }
            if m.cstr(m.u64(q + abi::PROTO_NAME)) == want {
                return true;
            }
            todo.extend(abi::protocol_list(m, m.u64(q + abi::PROTO_PROTOCOLS)));
        }
        false
    }

    /// Whether `cls` (or a superclass) adopts `proto` or a protocol that
    /// includes it (protocols are the same when their names are).
    pub fn conforms(&self, m: &dyn Mem, cls: u64, proto: u64) -> bool {
        let want = m.cstr(m.u64(proto + abi::PROTO_NAME));
        let mut c = cls;
        let mut guard = 0;
        while c != 0 && guard < 1024 {
            let Some(k) = self.classes.get(&c) else { return false };
            let mut todo: Vec<u64> = k.protocols.clone();
            let mut seen = 0;
            while let Some(p) = todo.pop() {
                seen += 1;
                if seen > 4096 {
                    break;
                }
                if m.cstr(m.u64(p + abi::PROTO_NAME)) == want {
                    return true;
                }
                todo.extend(abi::protocol_list(m, m.u64(p + abi::PROTO_PROTOCOLS)));
            }
            c = k.superclass;
            guard += 1;
        }
        false
    }
}
