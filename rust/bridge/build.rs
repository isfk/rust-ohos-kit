//! Compiles the NAPI glue.
//!
//! `csrc/napi_ohos.c` is guarded by `__has_include(<napi/native_api.h>)`, so on a
//! desktop it compiles to nothing and the crate still builds — the same trick as
//! the Go version, for the same reason: a host `cargo test` must not need the
//! HarmonyOS NDK.
//!
//! The objects go to the linker one by one, and not into a `.a`, because of how
//! a static library is used: an archive member is only pulled in when something
//! already needs a symbol it defines. Nothing in Rust calls the C glue — the
//! glue calls Rust — so a `.a` here would be dropped whole, taking the
//! `__attribute__((constructor))` that registers the module with it. That failure
//! is silent: the `.so` builds, the exports are all there, and ArkTS gets a
//! module with no functions in it.

fn main() {
    println!("cargo:rerun-if-changed=csrc/napi_ohos.c");
    let objects = cc::Build::new()
        .file("csrc/napi_ohos.c")
        .warnings(false)
        .compile_intermediates();
    for object in objects {
        println!("cargo:rustc-link-arg={}", object.display());
    }
}
