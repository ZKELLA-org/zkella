//! Instruction-budget measurement for the contract entrypoints that do not
//! consume a zero-knowledge proof. Proof-consuming entrypoints are measured by
//! the cost-parity tests inside each contract crate. Every test here registers
//! the real compiled WASM from `target/wasm32v1-none/release`.
