//! Compiles the NAPI glue and the log pipe.
//!
//! `csrc/napi_ohos.c` is guarded by `__has_include(<napi/native_api.h>)`, so on a
//! desktop it compiles to nothing and the crate still builds — the same trick as
//! the Go version, for the same reason: a host `cargo test` must not need the
//! HarmonyOS NDK.
//!
//! `csrc/log_ohos.c` has no such guard. It is C for the phone only (it pipes the
//! core's stdout/stderr into hilog), so the target decides for it: this script
//! defines `RUSTKIT_WITH_HILOG` when building for OHOS, and the file is empty
//! everywhere else. Reading the target here beats a header probe in the C file:
//! one place knows what is being built for.
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
    println!("cargo:rerun-if-changed=csrc/log_ohos.c");

    // `target_env`, not `target_os`: aarch64-unknown-linux-ohos reports
    // target_os=linux. AGENTS.md lists this as a trap for a reason.
    let ohos = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("ohos");

    let mut build = cc::Build::new();
    build
        .file("csrc/napi_ohos.c")
        .file("csrc/log_ohos.c")
        .warnings(false);
    if ohos {
        build.define("RUSTKIT_WITH_HILOG", None);
    }

    for object in build.compile_intermediates() {
        println!("cargo:rustc-link-arg={}", object.display());
    }
}
