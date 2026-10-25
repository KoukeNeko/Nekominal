fn main() {
    println!("cargo:rerun-if-changed=nekominal.rc");
    println!("cargo:rerun-if-changed=../../assets/nekominal.ico");
    embed_resource::compile("nekominal.rc", embed_resource::NONE).manifest_required().unwrap();
}
