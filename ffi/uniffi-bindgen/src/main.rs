//! `cargo run -p uniffi-bindgen -- generate --library <libroadmesh_ffi.so> --language kotlin --out-dir <dir>`

fn main() {
    uniffi::uniffi_bindgen_main()
}
