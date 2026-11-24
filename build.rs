// Intègre l'icône et les informations du programme dans lyra.exe (Windows).
fn main() {
    println!("cargo:rerun-if-changed=assets/lyra.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/lyra.ico");
        res.set("ProductName", "Lyra");
        res.set("FileDescription", "Lyra, nettoyeur de métadonnées");
        res.set("LegalCopyright", "OpenLyra");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Icône Windows non intégrée : {e}");
        }
    }
}
