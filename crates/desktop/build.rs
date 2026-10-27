fn main() {
    // Embed the icon in the Windows executable (taskbar, shortcuts, Explorer).
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=windows/hyperb.rc");
        println!("cargo:rerun-if-changed=windows/hyperb.ico");
        embed_resource::compile("windows/hyperb.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
