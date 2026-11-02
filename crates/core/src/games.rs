//! Game components (ADR-0016, docs/specs/Spec - Composants de jeu.md): software games need, told
//! apart from drivers. Pure classification of the "Uninstall" registry entries; reading the registry
//! lives in `drivn-windows`. Only the installed version is known: the latest one stays unknown until
//! its official source is checked (R-VRAI-04).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    VisualCpp,
    DotNet,
    Java,
    PhysX,
    Vulkan,
    OpenAl,
    Xna,
    GameInput,
}

/// One entry of an "Uninstall" registry key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledProgram {
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameComponent {
    pub kind: GameKind,
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
}

/// Name **and** publisher must both match: a name alone is easy to imitate.
pub fn classify(name: &str, publisher: Option<&str>) -> Option<GameKind> {
    let name = name.to_lowercase();
    let publisher = publisher.unwrap_or_default().to_lowercase();
    let from = |who: &str| publisher.contains(who);
    if name.contains("microsoft visual c++")
        && name.contains("redistributable")
        && from("microsoft")
    {
        Some(GameKind::VisualCpp)
    } else if (name.contains("microsoft .net")
        || name.contains("microsoft windows desktop runtime")
        || name.contains("microsoft asp.net core"))
        && !name.contains("host") // .NET Host / Host FX Resolver: parts of a runtime, listed apart
        && from("microsoft")
    {
        Some(GameKind::DotNet)
    } else if name.contains("java")
        && !name.contains("development kit")
        && !name.contains("updater")
        && from("oracle")
    {
        Some(GameKind::Java)
    } else if name.contains("physx") && from("nvidia") {
        Some(GameKind::PhysX)
    } else if name.contains("vulkan") && (from("lunarg") || from("khronos")) {
        Some(GameKind::Vulkan)
    } else if name.contains("openal") && from("creative") {
        Some(GameKind::OpenAl)
    } else if name.contains("xna framework") && from("microsoft") {
        Some(GameKind::Xna)
    } else if name.contains("gameinput") && from("microsoft") {
        Some(GameKind::GameInput)
    } else {
        None
    }
}

/// The last `1.2.3`-like word of a name. .NET installers store an internal number as their
/// `DisplayVersion` (80.56.61678 for .NET 10.0.14): the real version is in the name.
fn version_in_name(name: &str) -> Option<String> {
    name.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .rev()
        .find(|word| word.contains('.') && word.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(str::to_owned)
}

/// The game components among the installed programs, once each (the same program is often listed
/// under several keys), sorted by kind then name.
pub fn game_components(programs: impl IntoIterator<Item = InstalledProgram>) -> Vec<GameComponent> {
    let mut found: Vec<GameComponent> = programs
        .into_iter()
        .filter_map(|p| {
            let kind = classify(&p.name, p.publisher.as_deref())?;
            let version = match kind {
                GameKind::DotNet => version_in_name(&p.name).or(p.version),
                _ => p.version,
            };
            Some(GameComponent {
                kind,
                name: p.name,
                version,
                publisher: p.publisher,
            })
        })
        .collect();
    found.sort_by(|a, b| (a.kind, &a.name, &a.version).cmp(&(b.kind, &b.name, &b.version)));
    found.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(name: &str, version: &str, publisher: &str) -> InstalledProgram {
        InstalledProgram {
            name: name.to_owned(),
            version: Some(version.to_owned()),
            publisher: Some(publisher.to_owned()),
        }
    }

    #[test]
    fn known_components_need_name_and_publisher() {
        let vc = "Microsoft Visual C++ 2015-2022 Redistributable (x64) - 14.40.33810";
        assert_eq!(
            classify(vc, Some("Microsoft Corporation")),
            Some(GameKind::VisualCpp)
        );
        assert_eq!(classify(vc, Some("Contoso")), None);
        assert_eq!(classify(vc, None), None);
        assert_eq!(
            classify(
                "Microsoft Windows Desktop Runtime - 8.0.11 (x64)",
                Some("Microsoft Corporation")
            ),
            Some(GameKind::DotNet)
        );
        assert_eq!(
            classify("Java 8 Update 501", Some("Oracle Corporation")),
            Some(GameKind::Java)
        );
        assert_eq!(
            classify(
                "NVIDIA PhysX System Software 9.23.1019",
                Some("NVIDIA Corporation")
            ),
            Some(GameKind::PhysX)
        );
    }

    #[test]
    fn other_game_runtimes_need_their_publisher() {
        let cases = [
            (
                "Vulkan Run Time Libraries 1.3.250.1",
                "LunarG, Inc.",
                GameKind::Vulkan,
            ),
            ("OpenAL", "Creative Labs", GameKind::OpenAl),
            (
                "Microsoft XNA Framework Redistributable 4.0 Refresh",
                "Microsoft Corporation",
                GameKind::Xna,
            ),
            (
                "Microsoft GameInput",
                "Microsoft Corporation",
                GameKind::GameInput,
            ),
        ];
        for (name, publisher, kind) in cases {
            assert_eq!(classify(name, Some(publisher)), Some(kind), "{name}");
        }
    }

    #[test]
    fn look_alikes_are_not_game_components() {
        // A JDK is a developer tool, not what games need; other software is out of scope (ADR-0016).
        assert_eq!(
            classify("Java SE Development Kit 21", Some("Oracle Corporation")),
            None
        );
        assert_eq!(
            classify(
                "Microsoft Visual Studio Code",
                Some("Microsoft Corporation")
            ),
            None
        );
        assert_eq!(classify("Steam", Some("Valve Corporation")), None);
        assert_eq!(
            classify(
                "Microsoft .NET Host - 10.0.14 (x64)",
                Some("Microsoft Corporation")
            ),
            None
        );
        assert_eq!(
            classify("Java Auto Updater", Some("Oracle Corporation")),
            None
        );
        assert_eq!(
            classify("Vulkan Run Time Libraries 1.3.250.1", Some("Contoso")),
            None
        );
    }

    #[test]
    fn dotnet_version_comes_from_its_name() {
        let found = game_components([
            program(
                "Microsoft .NET Runtime - 10.0.14 (x64)",
                "80.56.61678",
                "Microsoft Corporation",
            ),
            program(
                "Microsoft .NET Runtime - 10.0.14 (x64)",
                "10.0.14.50000",
                "Microsoft Corporation",
            ),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].version.as_deref(), Some("10.0.14"));
    }

    #[test]
    fn duplicates_are_listed_once_and_sorted_by_kind() {
        let found = game_components([
            program("Java 8 Update 501", "8.0.5010.8", "Oracle Corporation"),
            program("Java 8 Update 501", "8.0.5010.8", "Oracle Corporation"),
            program("Steam", "2.10", "Valve Corporation"),
            program(
                "Microsoft Visual C++ 2015-2022 Redistributable (x86) - 14.40.33810",
                "14.40.33810.0",
                "Microsoft Corporation",
            ),
        ]);
        let kinds: Vec<GameKind> = found.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [GameKind::VisualCpp, GameKind::Java]);
    }
}
