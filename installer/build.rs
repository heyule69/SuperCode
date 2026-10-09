fn main() {
    println!("cargo:rerun-if-changed=../src-tauri/icons/icon.ico");
    println!("cargo:rerun-if-changed=generated/payload.zip");
    println!("cargo:rerun-if-changed=generated/manifest.json");
    tauri_build::build();
}
