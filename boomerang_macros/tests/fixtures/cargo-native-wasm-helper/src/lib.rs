#![no_std]
#[cfg(not(target_arch = "wasm32"))]
compile_error!("target-only wasm helper must never compile on the proc-macro/build-script host");
pub fn value() -> u32 {
    21
}
