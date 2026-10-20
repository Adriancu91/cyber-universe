// Links every program with user.ld (absolute path, so it works whatever the linker's working directory).
fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg-bins=-T{dir}/user.ld");
    println!("cargo:rerun-if-changed=user.ld");
}
