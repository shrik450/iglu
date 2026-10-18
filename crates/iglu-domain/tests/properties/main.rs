//! Properties of the core's pure functions, checked on generated inputs.
//!
//! Each property runs `PROPTEST_CASES` cases, 256 by default; CI's deeper
//! run raises it. A failure is shrunk to a smallest case and saved under
//! `proptest-regressions/`, which is committed so it runs first from then on.

mod access;
mod grammars;
mod lifecycle;
mod names;
mod outputs;
mod support;
mod text;
