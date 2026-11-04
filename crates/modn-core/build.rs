#[path = "src/bucket_digest.rs"]
mod bucket_digest;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main");
    println!("cargo:rerun-if-changed={}", root.display());
    let digest = bucket_digest::directory_digest(&root).expect("hash repository main bucket");
    println!("cargo:rustc-env=MAIN_BUCKET_DIGEST={digest}");
}
