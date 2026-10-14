//! Regenerates the WinUI 3 bindings used by the chrome spike from the Windows App SDK 2.5.1 metadata.
//!
//! usage: winui-gen <winmd-dir> <out.rs> <filter-file>...
//!
//! `<winmd-dir>` is `crates/tools/reactor-metadata/winmd` of microsoft/windows-rs (30 .winmd files plus
//! `extras.winmd` with IWindowNative / ISwapChainPanelNative).  Filter files are merged in order.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(args.len() >= 4, "usage: winui-gen <winmd-dir> <out.rs> <filter-file>...");
    let t = std::time::Instant::now();
    let mut b = windows_bindgen::builder();
    b.input(&args[1])
        .input_default() // bundled Windows.winmd + Windows.Win32.winmd
        .output(&args[2])
        .implements(std::fs::read_to_string("implements.txt").unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect::<Vec<_>>())
        .composes(std::fs::read_to_string("compose.txt").unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect::<Vec<_>>())
        .minimal()
        .flat();
    for filter in &args[3..] {
        b.filter_file(filter);
    }
    b.write();
    println!("generated {} in {:.2?}", args[2], t.elapsed());
}
