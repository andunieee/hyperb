fn main() {
    // Each demo exports the same unmangled `he_*` functions through
    // `hypereffect::wasm_app!`, which is not gated to wasm32. Nothing calls
    // them natively, so let the linker keep one copy.
    println!("cargo:rustc-link-arg-bins=-Wl,--allow-multiple-definition");
}
