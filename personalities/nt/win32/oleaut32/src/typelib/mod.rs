//! Type libraries (ADR 0220, B2b): the objects (`ITypeLib`, `ITypeInfo`, `ICreateTypeLib2`,
//! `ICreateTypeInfo`), loading and registration, and `ITypeInfo::Invoke`.

pub(crate) mod desc;
pub(crate) mod ext;
pub(crate) mod invoke;
pub(crate) mod load;
pub(crate) mod objects;
