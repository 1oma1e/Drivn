//! Official tools of a publisher (ADR-0023, Spec - Sources constructeurs, section 3): Drivn starts
//! one only if the local executable is signed by the expected signer, without admin rights
//! (R-SRC-10); otherwise it shows the official page only.

/// Where the tool lives (under `Program Files (x86)` or `Program Files`), who must sign it, and
/// its page. The page is written without its scheme: it is opened in the browser, never called by
/// Drivn (it is a "lien seulement" host of docs/SOURCES.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfficialTool {
    pub kind: &'static str,
    pub name: &'static str,
    pub relative_path: &'static str,
    pub signer: &'static str,
    pub page: &'static str,
}

/// Java Update: seen on the reference PC on 2026-11-01 (`jucheck.exe`, signature valid, "Oracle America,
/// Inc."). Intel DSA (ADR-0033): not installed on the reference PC, path and signer still to verify;
/// absent, Drivn only opens Intel's page.
pub const TOOLS: &[OfficialTool] = &[
    OfficialTool {
        kind: "java",
        name: "Java Update",
        relative_path: "Common Files\\Java\\Java Update\\jucheck.exe",
        signer: "Oracle America, Inc.",
        page: "www.java.com/fr/download/",
    },
    OfficialTool {
        kind: "intel_dsa",
        name: "Intel Driver & Support Assistant",
        relative_path: "Intel\\Driver and Support Assistant\\DSATray.exe",
        signer: "Intel Corporation",
        page: "www.intel.com/content/www/us/en/support/detect.html",
    },
];

pub fn tool(kind: &str) -> Option<&'static OfficialTool> {
    TOOLS.iter().find(|t| t.kind == kind)
}

/// The page to open in the browser.
pub fn page_url(tool: &OfficialTool) -> String {
    format!("https://{}", tool.page)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Run,
    /// Not run: the reason, and the official page instead.
    LinkOnly(String),
}

/// R-SRC-10: `found`: the executable exists (no link); `signer`: its Authenticode signer, read in
/// the validated chain, or why it is not valid.
pub fn decide_launch(tool: &OfficialTool, found: bool, signer: Result<&str, &str>) -> Launch {
    if !found {
        return Launch::LinkOnly("not_installed".to_owned());
    }
    match signer {
        Ok(name) if name == tool.signer => Launch::Run,
        Ok(name) => Launch::LinkOnly(format!("signed by « {name} », not by {}", tool.signer)),
        Err(why) => Launch::LinkOnly(format!("signature non valide ({why})")),
    }
}
