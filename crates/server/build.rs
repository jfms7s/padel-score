// The web app is embedded from web/build. Make sure the folder exists so the Rust side
// compiles (and its tests run) before the SvelteKit app has been built.
fn main() -> std::io::Result<()> {
    let dir = std::path::Path::new("../../web/build");
    if !dir.join("index.html").exists() {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><meta charset=utf-8><title>Padel score</title><p>The web app isn't built yet. Run <code>npm run build</code> in <code>web/</code>.</p>\n",
        )?;
    }
    println!("cargo:rerun-if-changed=../../web/build");
    println!("cargo:rerun-if-changed=migrations");
    Ok(())
}
