//! `drivn-scan`: phase 0 prototype (docs/specs/Spec - Scan lecture seule.md).
//! Writes a JSON snapshot of all devices and drivers and prints a short summary.
//! Read-only, no administrator rights, no network unless `--wu` is given.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use drivn_core::decide::{State, decide};
use drivn_core::model::Snapshot;
use drivn_core::scan::{ScanContext, ScanOptions, scan};
use drivn_core::summary::summary;
use drivn_core::time::iso_utc_from_unix_seconds;
use drivn_windows::vendors::ask_vendors;
use drivn_windows::{CfgMgrDevices, WindowsUpdateAgent};

const USAGE: &str = "Usage : drivn-scan [--apps] [--wu] [--vendors] [--all] [--anonymize] [--out <dossier>]
       drivn-scan --decide <instantane.json> [--json <fichier>]
  --wu         demande aussi a Windows Update les drivers proposes (reseau, sans telecharger)
  --vendors    demande aussi au Microsoft Update Catalog, a NVIDIA, a ASUS et a Oracle (reseau, sources fragiles)
  --apps       liste seulement les applications a mettre a jour, par winget (lecture seule)
  --all        inclut les appareils deconnectes
  --anonymize  retire le nom du PC et les numeros de serie
  --out        dossier du fichier JSON (par defaut : snapshots)
  --decide     lit un instantane et affiche les decisions (ni Windows, ni reseau)
  --json       ecrit aussi l'instantane et ses decisions dans un fichier (donnees du banc d essai app/dev)";

struct Args {
    options: ScanOptions,
    out: PathBuf,
    vendors: bool,
    decide: Option<PathBuf>,
    json: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut options = ScanOptions::default();
    let mut out = PathBuf::from("snapshots");
    let (mut decide, mut json) = (None, None);
    let mut vendors = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wu" => options.windows_update = true,
            "--vendors" => vendors = true,
            "--all" => options.include_absent = true,
            "--anonymize" => options.anonymize = true,
            "--out" => out = args.next().ok_or("--out demande un dossier")?.into(),
            "--decide" => decide = Some(args.next().ok_or("--decide demande un fichier")?.into()),
            "--json" => json = Some(args.next().ok_or("--json demande un fichier")?.into()),
            other => return Err(format!("option inconnue : {other}")),
        }
    }
    Ok(Args {
        options,
        out,
        vendors,
        decide,
        json,
    })
}

/// Prints the decisions of an existing snapshot: every proposed update with its evidence.
fn print_decisions(path: &Path, json: Option<&Path>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let snapshot: Snapshot =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let report = decide(&snapshot);
    for (device, decision) in snapshot.devices.iter().zip(&report.decisions) {
        let name = device.description.as_deref().unwrap_or(&device.instance_id);
        for source in &decision.unchecked_by {
            println!("{name} : pas pu verifier chez {source:?}");
        }
        for hint in &decision.hints {
            println!(
                "{name} : {:?} {}",
                hint.kind,
                hint.text.as_deref().unwrap_or("")
            );
        }
        let Some(chosen) = &decision.chosen else {
            continue;
        };
        let announced = match (&decision.state, chosen.offer.source) {
            (State::Announced { .. }, Some(source)) => format!(" [annoncee par {source:?}]"),
            _ => String::new(),
        };
        let from = device
            .driver
            .as_ref()
            .and_then(|d| d.version.clone())
            .unwrap_or_else(|| "aucun".to_owned());
        let risk = chosen
            .risk
            .map(|r| format!(" (a verifier : {r:?})"))
            .unwrap_or_default();
        println!(
            "{name} : {from} -> {}{risk}{announced}\n    ID {}",
            chosen
                .offer
                .version
                .as_deref()
                .or(chosen.offer.vendor_version.as_deref())
                .unwrap_or("?"),
            chosen.offer.hardware_id
        );
    }
    for result in &snapshot.vendors {
        println!(
            "Source {:?} : {} requete(s){}{}",
            result.source,
            result.requests,
            result
                .error
                .as_deref()
                .map(|e| format!(", inconnu : {e}"))
                .unwrap_or_default(),
            result
                .java8_latest_update
                .map(|u| format!(", Java 8 Update {u} chez Oracle"))
                .unwrap_or_default()
        );
        // ADR-0032: shown only, never downloaded nor installed.
        if let Some(bios) = &result.bios {
            println!(
                "BIOS {} : {} ({}) ; dernier chez {:?} : {} ({}), plus recent : {}",
                bios.board,
                bios.installed.as_deref().unwrap_or("inconnu"),
                bios.installed_date.as_deref().unwrap_or("?"),
                result.source,
                bios.latest,
                bios.latest_date.as_deref().unwrap_or("?"),
                bios.newer
                    .map_or("inconnu", |n| if n { "oui" } else { "non" })
            );
        }
    }
    for offer in &report.unmatched_offers {
        println!(
            "Offre sans appareil ({:?}) : {}",
            offer.reason,
            offer.title.as_deref().unwrap_or("sans titre")
        );
    }
    if let Some(out) = json {
        let both = serde_json::json!({ "snapshot": snapshot, "report": report });
        let text = serde_json::to_string(&both).map_err(|e| e.to_string())?;
        std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(())
}

fn run() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("--apps") {
        // ADR-0034: read-only list, nothing is updated here.
        let apps = drivn_windows::winget::upgrades()?;
        for app in &apps {
            println!(
                "{} ({}) : {} -> {}",
                app.name, app.id, app.version, app.available
            );
        }
        println!("{} application(s) a mettre a jour (winget)", apps.len());
        return Ok(());
    }
    let args = parse_args()?;
    if let Some(path) = &args.decide {
        return print_decisions(path, args.json.as_deref());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("horloge : {e}"))?
        .as_secs();
    let created_utc = iso_utc_from_unix_seconds(i64::try_from(now).map_err(|e| e.to_string())?);
    let context = ScanContext {
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        created_utc: created_utc.clone(),
        machine: std::env::var("COMPUTERNAME").ok(),
    };
    if args.options.windows_update {
        eprintln!("Recherche Windows Update en cours (peut prendre quelques minutes)...");
    }
    let mut snapshot = scan(
        &CfgMgrDevices,
        &WindowsUpdateAgent { online: true },
        &args.options,
        context,
    )?;
    if args.vendors {
        eprintln!("Sources des editeurs (Catalog, NVIDIA, Oracle) : une requete a la fois...");
        snapshot.vendors = ask_vendors(&snapshot.devices);
    }

    let label = match (&snapshot.machine, snapshot.anonymized) {
        (Some(machine), false) => machine.clone(),
        _ => "anonyme".to_owned(),
    };
    let stamp: String = created_utc.chars().filter(char::is_ascii_digit).collect();
    std::fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    let path = args.out.join(format!("snapshot-{label}-{stamp}.json"));
    let json = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;

    print!("{}", summary(&snapshot));
    println!("Instantané : {}", path.display());
    if args.vendors {
        print_decisions(&path, None)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("drivn-scan : {error}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
