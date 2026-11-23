fn main() {
    // "owner/repo" for self-updates. Leave repo.txt out to build without updates.
    println!("cargo:rerun-if-changed=repo.txt");
    println!("cargo:rerun-if-changed=assets/hub.ico");
    let repo = std::fs::read_to_string("repo.txt").unwrap_or_default();
    println!("cargo:rustc-env=BINGOHUB_REPO={}", repo.trim());

    // Item icons for the Classes page. They're game art, so they aren't in git: tools/item-icons
    // makes them from the installed game. Without them the page shows the cards without pictures.
    println!("cargo:rerun-if-changed=assets/icons.bin");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("icons.bin");
    std::fs::write(&out, std::fs::read("assets/icons.bin").unwrap_or_default()).unwrap();

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/hub.ico");
    res.set("ProductName", "Bingo Hub");
    res.set("FileDescription", "Bingo Hub");
    let _ = res.compile();
}
