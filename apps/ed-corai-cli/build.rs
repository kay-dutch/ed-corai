//! Embeds the station icon and version information in the Windows program.

fn main() {
    println!("cargo:rerun-if-changed=../ed-corai/assets/ed-corai.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../ed-corai/assets/ed-corai.ico");
        res.set("ProductName", "ED Corai");
        res.set("FileDescription", "ED Corai command line and MCP server");
        res.set(
            "LegalCopyright",
            "Copyright (c) 2026 kay-dutch. MIT licensed.",
        );
        // Not fatal: a contributor without the Windows SDK's rc.exe can still build and test.
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon or version info: {e}");
        }
    }
}
