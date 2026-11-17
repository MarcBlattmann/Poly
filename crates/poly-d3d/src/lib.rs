//! The rules of Poly's software Direct3D 11 device and its DXGI objects (ADR 0211).
//!
//! Poly has no GPU. Its `d3d11.dll` makes a software device that behaves like Windows' WARP device
//! (the Microsoft Basic Render Driver every Windows machine has) for everything that is not a
//! shader: creation, the interface chain, resources in memory, views, state objects, queries,
//! copies, clears and maps; `dxgi.dll` lists one software adapter and makes swap chains;
//! `dcomp.dll` binds them to windows. Everything that is a rule (which descriptions are valid,
//! how a description is completed or normalised, what a clear stores, which sizes a query
//! has) lives here, tested on the host; the DLLs hold the COM objects and their memory.
//!
//! The behaviour is the observed behaviour of the spec sheets `specs/windows/{d3d11,dxgi,dcomp}.dll`
//! (spec agent `spec-chrome-win-s5`) and Learn's Direct3D 11 / DXGI pages; constants and layouts
//! are those of the public SDK headers (`d3d11.h`, `d3dcommon.h`, `dxgi.h`, `dxgiformat.h`,
//! `winerror.h`).

#![no_std]

extern crate alloc;

pub mod clear;
pub mod create;
pub mod format;
pub mod hr;
pub mod privdata;
pub mod query;
pub mod resource;
pub mod state;
pub mod swap;
pub mod view;
