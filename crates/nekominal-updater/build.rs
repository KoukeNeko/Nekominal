fn main() {
    println!("cargo:rerun-if-changed=updater.rc");
    println!("cargo:rerun-if-changed=../../assets/nekominal.ico");
    embed_resource::compile("updater.rc", embed_resource::NONE).manifest_required().unwrap();
}
