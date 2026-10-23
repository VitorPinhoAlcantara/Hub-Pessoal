fn main() {
    println!("cargo:rerun-if-changed=hub.rc");
    println!("cargo:rerun-if-changed=../../assets/app/hub.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("hub.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("falha ao embutir o ícone");
    }
}
