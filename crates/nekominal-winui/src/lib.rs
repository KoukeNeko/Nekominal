//! WinUI 3 runtime helpers for Nekominal (no terminal knowledge).
//!
//! * [`app`]: bootstrap of the Windows App Runtime, the `Application` subclass, `run`.
//! * [`surface`]: a `SwapChainPanel` bound to wgpu (DX12).
//! * [`ime`]: a `CoreTextEditContext` host for TSF input.
//! * [`window`]: HWND and screen-rectangle helpers.
//! * [`diag`]: file logging driven by `NEKOMINAL_LOG`.
// The bindings keep the Windows metadata names (`SetXxx`, `IFoo`, enum constants).
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

pub mod app;
pub mod diag;
pub mod ime;
pub mod surface;
pub mod window;
