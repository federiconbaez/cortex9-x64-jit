//! cortex9-x64-jit: stack bytecode → x86-64 machine code.
//!
//! - [`bytecode`]: instructions, semantics and verifier.
//! - [`interp`]: reference interpreter.
//! - [`asm`]: hand-written x86-64 encoder.
//! - [`jit`]: code generation and W^X executable memory (x86-64 Unix only).

#![warn(missing_docs)]

pub mod asm;
pub mod bytecode;
pub mod interp;
#[cfg(all(target_arch = "x86_64", unix))]
pub mod jit;
