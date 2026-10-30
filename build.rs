fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    #[cfg(windows)]
    {
        let mut resources = winresource::WindowsResource::new();
        resources.set_icon("assets/app.ico");
        resources.set("ProductName", "Scribe Seek");
        resources.set("FileDescription", "Scribe Seek — local transcription");
        resources.set(
            "OriginalFilename",
            &format!("ScribeSeek-v{}.exe", env!("CARGO_PKG_VERSION")),
        );
        resources
            .compile()
            .expect("Could not embed Windows application resources");
    }
}
