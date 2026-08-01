fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_manifest_file("assets/app.manifest");
        res.set_icon("assets/app.ico");
        res.set("FileDescription", "AutoGSE");
        res.set("ProductName", "AutoGSE");
        res.compile().expect("failed to embed Windows manifest");
    }
}
