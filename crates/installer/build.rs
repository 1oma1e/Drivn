// The installer is the only Drivn binary that asks for administrator rights (R-DRT-01): its
// manifest says `requireAdministrator`, so Windows shows the UAC before it starts.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
    }
}
