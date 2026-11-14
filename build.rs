fn main() {
    // "owner/repo" for self-updates. Leave repo.txt out to build without updates.
    println!("cargo:rerun-if-changed=repo.txt");
    let repo = std::fs::read_to_string("repo.txt").unwrap_or_default();
    println!("cargo:rustc-env=BINGOHUB_REPO={}", repo.trim());

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/hub.ico");
    res.set("ProductName", "Bingo Hub");
    res.set("FileDescription", "Bingo Hub");
    let _ = res.compile();
}
