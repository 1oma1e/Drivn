//! Sources of phase 3 (Spec - Sources constructeurs, « Tests »): a fake network records every
//! request; no test touches the real network.

use std::cell::RefCell;

use drivn_core::decide::{State, decide};
use drivn_core::model::{
    Board, Device, HintKind, InstalledDriver, SCHEMA_VERSION, Signature, Snapshot, VendorResult,
};
use drivn_core::sources::SourceId;
use drivn_core::sources::guard::{Guard, GuardState, MAX_REQUESTS_PER_SCAN, Permit};
use drivn_core::sources::hosts::{HttpsUrl, UrlRefused};
use drivn_core::sources::http::{FetchError, Http, Request, Response, USER_AGENT, fetch};
use drivn_core::sources::vendors::{
    ASK_INTEL, VendorContext, check_intel, check_vendors, check_vendors_with_progress,
};
use drivn_core::sources::{intel, ms_catalog, nvidia, oracle, text};

const NOW: i64 = 1_793_500_000;
const MEI: &str = "PCI\\VEN_8086&DEV_7A68&SUBSYS_88821043&REV_11";
const MEI_GUID: &str = "ad53b108-4ed3-4bed-a2c3-15f60a6215c0";
const OLD_GUID: &str = "a8ad9459-2fba-4be9-b8b2-9533bfa0b824";

/// Answers by URL prefix; records every request.
#[derive(Default)]
struct FakeNet {
    answers: Vec<(String, u16, String)>,
    sent: RefCell<Vec<Request>>,
    pauses: RefCell<Vec<u64>>,
    /// Answers given once each, before `answers` (a passing page, then the real one).
    once: RefCell<Vec<(String, u16, String)>>,
}

impl FakeNet {
    fn on(mut self, url_prefix: &str, status: u16, body: &str) -> Self {
        self.answers
            .push((url_prefix.to_owned(), status, body.to_owned()));
        self
    }
    fn first(self, url_prefix: &str, status: u16, body: &str) -> Self {
        self.once
            .borrow_mut()
            .push((url_prefix.to_owned(), status, body.to_owned()));
        self
    }
    fn urls(&self) -> Vec<String> {
        self.sent
            .borrow()
            .iter()
            .map(|r| r.url.to_string())
            .collect()
    }
}

impl Http for FakeNet {
    fn send(&self, request: &Request) -> Result<Response, String> {
        self.sent.borrow_mut().push(request.clone());
        let url = request.url.to_string();
        let mut once = self.once.borrow_mut();
        if let Some(at) = once
            .iter()
            .position(|(p, _, _)| url.starts_with(p.as_str()))
        {
            let (_, status, body) = once.remove(at);
            return Ok(Response {
                status,
                location: None,
                body: body.into_bytes(),
            });
        }
        let (_, status, body) = self
            .answers
            .iter()
            .find(|(prefix, _, _)| url.starts_with(prefix.as_str()))
            .ok_or_else(|| format!("no answer for {url}"))?;
        Ok(Response {
            status: *status,
            location: None,
            body: body.as_bytes().to_vec(),
        })
    }

    fn pause(&self, seconds: u64) {
        self.pauses.borrow_mut().push(seconds);
    }
}

fn row(guid: &str, n: usize, title: &str, date: &str, version: &str) -> String {
    format!(
        r#"<tr id="{guid}_R{n}"><td id="{guid}_C0_R{n}"></td><td id="{guid}_C1_R{n}"><a id="{guid}_link">
        {title}</a></td><td id="{guid}_C2_R{n}">Windows 11 Client, version 25H2 and later, Servicing Drivers</td>
        <td id="{guid}_C3_R{n}">Drivers (Other Hardware)</td><td id="{guid}_C4_R{n}">{date}</td>
        <td id="{guid}_C5_R{n}">{version}</td><td id="{guid}_C6_R{n}"><span id="{guid}_size">236 KB</span>
        <span class="noDisplay" id="{guid}_originalSize">242265</span></td></tr>"#
    )
}

fn search_page(total: u32, rows: &[String]) -> String {
    format!(
        r#"<html><title>Microsoft Update Catalog</title><span id="ctl00_catalogBody_searchDuration">1 - 25 of {total} (page 1 of 2)</span>
        <table id="ctl00_catalogBody_updateMatches"><tr id="headerRow"></tr>{}</table></html>"#,
        rows.join("")
    )
}

const NO_RESULT: &str = r#"<html><title>Microsoft Update Catalog</title><span id="ctl00_catalogBody_noResultText">We did not find any results for </span></html>"#;

fn details(provider: &str, ids: &[&str]) -> String {
    let divs: String = ids
        .iter()
        .map(|id| {
            format!(
                "<div style=\"padding-bottom: 0.3em;\">\n {} \n</div>",
                id.replace('&', "&amp;")
            )
        })
        .collect();
    format!(
        r#"<div id="ScopedViewHandler_company">Wistron Corporation</div><span id="ScopedViewHandler_driverProvider">{provider}</span>
        <span id="ScopedViewHandler_version">2627.10.5.0</span><div id="driverhwIDs">{divs}</div><div id="driverupdateIDs"></div>"#
    )
}

fn mei() -> Device {
    Device {
        instance_id: format!("{MEI}\\3&11583659&0&B0"),
        description: Some("Intel(R) Management Engine Interface #1".to_owned()),
        class: Some("System".to_owned()),
        class_guid: None,
        manufacturer: Some("Intel".to_owned()),
        present: true,
        problem_code: None,
        hardware_ids: vec![
            MEI.to_owned(),
            "PCI\\VEN_8086&DEV_7A68&SUBSYS_88821043".to_owned(),
            "PCI\\VEN_8086&DEV_7A68&CC_078000".to_owned(),
            "PCI\\VEN_8086&DEV_7A68".to_owned(),
        ],
        compatible_ids: vec!["PCI\\VEN_8086&CC_0780".to_owned()],
        driver: Some(InstalledDriver {
            version: Some("2603.9.4.0".to_owned()),
            date: Some("2026-01-13".to_owned()),
            provider: Some("Intel".to_owned()),
            inf_path: Some("oem13.inf".to_owned()),
            rank: None,
            matching_device_id: Some("PCI\\VEN_8086&DEV_7A68&CC_078000".to_owned()),
            signature: Signature::Signed { signer: None },
        }),
    }
}

fn windows_driver(mut device: Device) -> Device {
    if let Some(d) = device.driver.as_mut() {
        d.inf_path = Some("heci.inf".to_owned());
    }
    device
}

fn context() -> VendorContext<'static> {
    VendorContext {
        now: NOW,
        os_release: Some("26H2"),
        java_versions: &[],
        board: None,
    }
}

fn mei_net() -> FakeNet {
    FakeNet::default()
        .on(
            "https://www.catalog.update.microsoft.com/Search.aspx?q=VEN_8086%26DEV_7A68%2026H2",
            200,
            NO_RESULT,
        )
        .on(
            "https://www.catalog.update.microsoft.com/Search.aspx?q=VEN_8086%26DEV_7A68%2026H1",
            200,
            NO_RESULT,
        )
        .on(
            "https://www.catalog.update.microsoft.com/Search.aspx?q=VEN_8086%26DEV_7A68%2025H2",
            200,
            &search_page(
                629,
                &[
                    row(
                        MEI_GUID,
                        0,
                        "Intel System Driver Update (2627.10.5.0)",
                        "6/29/2026",
                        "2627.10.5.0",
                    ),
                    row(
                        OLD_GUID,
                        1,
                        "Intel System Driver Update (2603.9.4.0)",
                        "1/12/2026",
                        "2603.9.4.0",
                    ),
                ],
            ),
        )
        .on(
            "https://www.catalog.update.microsoft.com/ScopedViewInline.aspx",
            200,
            &details(
                "Intel",
                &["pci\\ven_8086&dev_9cba", "pci\\ven_8086&dev_7a68"],
            ),
        )
}

fn snapshot(devices: Vec<Device>, vendors: Vec<VendorResult>) -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        tool_version: "test".to_owned(),
        created_utc: "2026-11-01T00:00:00Z".to_owned(),
        machine: None,
        anonymized: true,
        includes_absent_devices: false,
        devices,
        windows_update: None,
        vendors,
    }
}

// ---------- R-SRC-01: hosts ----------

#[test]
fn r_src_01_only_https_to_a_download_host() {
    assert!(HttpsUrl::parse("https://www.catalog.update.microsoft.com/Search.aspx?q=x").is_ok());
    assert_eq!(
        HttpsUrl::parse("http://www.catalog.update.microsoft.com/"),
        Err(UrlRefused::NotHttps)
    );
    // "Link only" or forbidden hosts are never called. Written `https:` + `//…` so that
    // `check-docs` keeps refusing any such host written in the real code.
    let url = |rest: &str| format!("https:{}", rest);
    for rest in [
        "//www.nvidia.com/drivers",
        "//www.asus.com/support",
        "//www.java.com/",
        "//evil.example/x",
        "//www.catalog.update.microsoft.com.evil.example/",
    ] {
        assert!(
            matches!(
                HttpsUrl::parse(&url(rest)),
                Err(UrlRefused::HostNotAllowed(_))
            ),
            "{rest}"
        );
    }
    for rest in [
        "//user@www.catalog.update.microsoft.com/",
        "//www.catalog.update.microsoft.com:444/",
        "//www.catalog.update.microsoft.com/a b",
        "//www.catalog.update.microsoft.com/#x",
    ] {
        assert_eq!(
            HttpsUrl::parse(&url(rest)),
            Err(UrlRefused::Malformed),
            "{rest}"
        );
    }
}

#[test]
fn r_src_01_redirect_outside_the_list_is_refused() {
    struct Redirect;
    impl Http for Redirect {
        fn send(&self, _: &Request) -> Result<Response, String> {
            Ok(Response {
                status: 302,
                location: Some("https://mirror.example/file.cab".to_owned()),
                body: Vec::new(),
            })
        }
    }
    let url = HttpsUrl::parse("https://catalog.s.download.windowsupdate.com/x.cab").unwrap();
    assert!(matches!(
        fetch(&Redirect, Request::get(url, 10)),
        Err(FetchError::Refused(UrlRefused::HostNotAllowed(_)))
    ));
}

#[test]
fn r_src_01_catalog_link_must_have_the_exact_format() {
    let ok = format!(
        "downloadInformation[0].files[0].url = 'https://catalog.s.download.windowsupdate.com/c/msdownload/update/driver/drvs/2026/09/{MEI_GUID}_9cb5dee4c245b5c6c56d7f57a1e1539293d87d99.cab';\r\n\
         downloadInformation[0].files[0].sha256 = 'N7Nx2joOMfHwk05nw5dyT7jGh9DjB1RgZRe3lWYQE3E=';\r\n"
    );
    let download = ms_catalog::read_download_dialog(&ok).unwrap();
    assert_eq!(download.url.host(), "catalog.s.download.windowsupdate.com");
    assert_eq!(download.sha256.len(), 64);
    for bad in [
        ok.replace("catalog.s.download.windowsupdate.com", "download.example"),
        ok.replace("/driver/drvs/", "/software/"),
        ok.replace(".cab'", ".exe'"),
        ok.replace("9cb5dee4", "zzzzzzzz"),
        format!("{ok}{}", ok.replace("files[0]", "files[1]")),
    ] {
        assert!(ms_catalog::read_download_dialog(&bad).is_err(), "{bad}");
    }
}

// ---------- R-SRC-11: no workaround, few requests ----------

#[test]
fn r_src_11_requests_carry_only_drivns_user_agent() {
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    check_vendors(&[mei()], &net, &mut guard, &context());
    let sent = net.sent.borrow();
    assert!(!sent.is_empty());
    for request in sent.iter() {
        let names: Vec<&str> = request.headers.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["User-Agent"], "{}", request.url);
        assert_eq!(request.headers[0].1, USER_AGENT);
        assert!(USER_AGENT.starts_with("Drivn/"));
    }
}

#[test]
fn r_src_11_a_403_blocks_the_source_for_24_hours_and_survives_a_restart() {
    let net = FakeNet::default().on("https://www.catalog.update.microsoft.com/", 403, "denied");
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    assert_eq!(net.sent.borrow().len(), 1);
    let catalog = &results[0];
    assert!(catalog.error.is_some());
    assert_eq!(catalog.unchecked, [mei().instance_id]);
    // Saved, read back after a restart of the app: still blocked, no request.
    let saved = serde_json::to_string(&guard.state).unwrap();
    let state: GuardState = serde_json::from_str(&saved).unwrap();
    let net2 = mei_net();
    let mut guard2 = Guard::new(state, NOW + 3600);
    check_vendors(&[mei()], &net2, &mut guard2, &context());
    assert!(net2.urls().iter().all(|u| !u.contains("catalog.update")));
    // After 24 h, asked again.
    let mut guard3 = Guard::new(guard2.state.clone(), NOW + 24 * 3600 + 1);
    assert_eq!(
        guard3.permit(SourceId::MsCatalog, "k", NOW + 24 * 3600 + 1),
        Permit::Go
    );
}

#[test]
fn r_src_11_captcha_or_changed_page_counts_as_a_refusal() {
    let net = FakeNet::default().on(
        "https://www.catalog.update.microsoft.com/",
        200,
        "<html>Please solve this CAPTCHA</html>",
    );
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    assert!(results[0].error.is_some());
    assert!(guard.blocked(SourceId::MsCatalog, NOW + 1).is_some());
    assert!(results[0].offers.is_empty());
}

#[test]
fn r_src_11_unreadable_page_is_asked_again_once_after_5_seconds() {
    // ADR-0031: a passing page without counter, then the real page: MEI found, no block.
    let passing = "<html><title>Microsoft Update Catalog</title><p>busy</p></html>";
    let net = mei_net().first(
        "https://www.catalog.update.microsoft.com/Search.aspx",
        200,
        passing,
    );
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    assert_eq!(*net.pauses.borrow(), [5]);
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    assert_eq!(results[0].offers.len(), 1);
    assert!(guard.blocked(SourceId::MsCatalog, NOW + 1).is_none());

    // Unreadable twice: blocked 24 h after exactly two requests, page kept for analysis.
    let net = FakeNet::default().on("https://www.catalog.update.microsoft.com/", 200, passing);
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    assert_eq!(net.sent.borrow().len(), 2);
    assert_eq!(*net.pauses.borrow(), [5]);
    assert!(results[0].error.is_some());
    assert!(guard.blocked(SourceId::MsCatalog, NOW + 1).is_some());
    assert_eq!(guard.state.rejected_page["ms_catalog"], passing);
    assert!(
        guard.state.cache.is_empty(),
        "an unreadable page is not kept as an answer"
    );
    // The kept page goes with the block.
    let later = Guard::new(guard.state.clone(), NOW + 24 * 3600 + 1);
    assert!(later.state.rejected_page.is_empty());
}

#[test]
fn r_src_11_a_403_is_not_asked_again() {
    let net = FakeNet::default().on("https://www.catalog.update.microsoft.com/", 403, "denied");
    let mut guard = Guard::new(GuardState::default(), NOW);
    check_vendors(&[mei()], &net, &mut guard, &context());
    assert_eq!(net.sent.borrow().len(), 1);
    assert!(net.pauses.borrow().is_empty());
}

#[test]
fn r_vrai_01_vendor_step_names_each_question_before_asking_it() {
    // Spec - Scan complet: "Microsoft Update Catalog : Intel MEI", measured, never guessed.
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let mut seen = Vec::new();
    check_vendors_with_progress(
        &[mei()],
        &net,
        &mut guard,
        &context(),
        &mut |done, total, asking| {
            seen.push(format!(
                "{done}/{total} {}",
                asking.map_or("-".to_owned(), |a| format!(
                    "{}: {}",
                    a.source.key(),
                    a.about
                ))
            ));
        },
    );
    assert_eq!(
        seen,
        [
            "0/1 -",
            "0/1 ms_catalog: Intel(R) Management Engine Interface #1",
            "1/1 -"
        ]
    );
}

#[test]
fn r_src_11_answers_are_kept_24_hours() {
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    check_vendors(&[mei()], &net, &mut guard, &context());
    let first = net.sent.borrow().len();
    let mut again = Guard::new(guard.state.clone(), NOW + 3600);
    check_vendors(&[mei()], &net, &mut again, &context());
    assert_eq!(
        net.sent.borrow().len(),
        first,
        "second scan within 24 h: no request"
    );
}

#[test]
fn r_src_11_never_more_than_60_requests_per_source_and_scan() {
    let mut guard = Guard::new(GuardState::default(), NOW);
    for n in 0..MAX_REQUESTS_PER_SCAN {
        assert_eq!(
            guard.permit(SourceId::Nvidia, &n.to_string(), NOW),
            Permit::Go
        );
    }
    assert_eq!(
        guard.permit(SourceId::Nvidia, "61", NOW),
        Permit::LimitReached
    );
    assert_eq!(guard.permit(SourceId::Oracle, "1", NOW), Permit::Go);
}

#[test]
fn r_src_11_device_with_a_driver_of_windows_is_not_asked() {
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    check_vendors(&[windows_driver(mei())], &net, &mut guard, &context());
    assert!(net.sent.borrow().is_empty());
}

#[test]
fn r_src_11_oracle_only_when_java_8_is_installed_and_once_a_day() {
    let net = FakeNet::default().on(
        "https://javadl-esd-secure.oracle.com/update/baseline.version",
        200,
        "27.0.1.0.3\r\n11.0.33.0.3\r\n1.8.0_511\r\n1.7.0_521\r\n",
    );
    let mut guard = Guard::new(GuardState::default(), NOW);
    check_vendors(&[], &net, &mut guard, &context());
    assert!(net.sent.borrow().is_empty());
    let java = ["8.0.5010.8".to_owned()];
    let ctx = VendorContext {
        java_versions: &java,
        ..context()
    };
    let results = check_vendors(&[], &net, &mut guard, &ctx);
    let oracle = results
        .iter()
        .find(|r| r.source == SourceId::Oracle)
        .unwrap();
    assert_eq!(oracle.java8_latest_update, Some(511));
    check_vendors(&[], &net, &mut guard, &ctx);
    assert_eq!(net.sent.borrow().len(), 1);
}

// ---------- Catalog: what it finds on the reference PC ----------

#[test]
fn ms_catalog_finds_the_intel_mei_of_driver_booster() {
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    let offer = &results[0].offers[0];
    assert_eq!(offer.version.as_deref(), Some("2627.10.5.0"));
    assert_eq!(offer.update_id.as_deref(), Some(MEI_GUID));
    assert_eq!(offer.hardware_id, "PCI\\VEN_8086&DEV_7A68");
    assert_eq!(offer.date.as_deref(), Some("2026-06-29"));
    assert_eq!(offer.publisher.as_deref(), Some("Wistron Corporation"));
    // 26H2 and 26H1 (nothing), 25H2, then the details of the newest row only.
    assert_eq!(net.sent.borrow().len(), 4);
}

#[test]
fn r_mat_01_catalog_update_that_does_not_list_the_device_id_is_not_offered() {
    let net = mei_net().on(
        "https://www.catalog.update.microsoft.com/ScopedViewInline.aspx",
        200,
        &details("Intel", &["pci\\ven_8086&dev_7a69"]),
    );
    // `on` adds after: put the other answer first.
    let net = FakeNet {
        answers: net.answers.into_iter().rev().collect(),
        ..FakeNet::default()
    };
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    assert!(results[0].offers.is_empty());
}

#[test]
fn r_vrai_04_list_cut_by_the_catalog_without_newer_is_unknown_not_up_to_date() {
    let net = FakeNet::default().on(
        "https://www.catalog.update.microsoft.com/Search.aspx",
        200,
        &search_page(
            1000,
            &[row(
                OLD_GUID,
                0,
                "Intel System Driver Update (2603.9.4.0)",
                "1/12/2026",
                "2603.9.4.0",
            )],
        ),
    );
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    assert_eq!(results[0].unchecked, [mei().instance_id]);
    let report = decide(&snapshot(vec![mei()], results));
    assert_eq!(report.decisions[0].unchecked_by, [SourceId::MsCatalog]);
}

// ---------- Engine: announced, never "update", never ticked ----------

#[test]
fn r_ver_03_catalog_offer_is_announced_never_update() {
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    let report = decide(&snapshot(vec![mei()], results));
    let decision = &report.decisions[0];
    assert!(
        matches!(decision.state, State::Announced { .. }),
        "{:?}",
        decision.state
    );
    let chosen = decision.chosen.as_ref().unwrap();
    assert_eq!(chosen.offer.source, Some(SourceId::MsCatalog));
    assert!(chosen.offer.installable);
}

#[test]
fn r_mat_02_catalog_offer_on_a_less_precise_id_is_risky() {
    let net = mei_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[mei()], &net, &mut guard, &context());
    let report = decide(&snapshot(vec![mei()], results));
    // The installed driver matched `…&CC_078000`; the Catalog lists only `PCI\VEN_8086&DEV_7A68`, further down.
    assert!(matches!(
        report.decisions[0].state,
        State::Announced {
            risk: Some(drivn_core::decide::Risk::LessPrecise)
        }
    ));
}

#[test]
fn r_but_01_nvidia_hd_audio_goes_with_the_package_of_the_card() {
    let card = Device {
        instance_id: "PCI\\VEN_10DE&DEV_2702&SUBSYS_41411458&REV_A1\\4&1".to_owned(),
        description: Some("NVIDIA GeForce RTX 4080 SUPER".to_owned()),
        hardware_ids: vec!["PCI\\VEN_10DE&DEV_2702&SUBSYS_41411458&REV_A1".to_owned()],
        compatible_ids: vec![],
        driver: Some(InstalledDriver {
            version: Some("32.0.16.1692".to_owned()),
            date: Some("2026-09-04".to_owned()),
            provider: Some("NVIDIA".to_owned()),
            inf_path: Some("oem19.inf".to_owned()),
            rank: None,
            matching_device_id: None,
            signature: Signature::Signed { signer: None },
        }),
        ..mei()
    };
    let card_newest = || card.clone();
    let audio = Device {
        instance_id: "HDAUDIO\\FUNC_01&VEN_10DE&DEV_00A4&SUBSYS_14584141&REV_1001\\5&1".to_owned(),
        description: Some("NVIDIA High Definition Audio".to_owned()),
        hardware_ids: vec![
            "HDAUDIO\\FUNC_01&VEN_10DE&DEV_00A4&SUBSYS_14584141&REV_1001".to_owned(),
        ],
        compatible_ids: vec![],
        driver: Some(InstalledDriver {
            version: Some("1.4.6.3".to_owned()),
            date: Some("2026-07-23".to_owned()),
            provider: Some("NVIDIA Corporation".to_owned()),
            inf_path: Some("oem22.inf".to_owned()),
            rank: None,
            matching_device_id: None,
            signature: Signature::Signed { signer: None },
        }),
        ..mei()
    };
    let json = r#"<html><body>{ "Success" : "1", "IDS" : [{ "downloadInfo": { "Success" : "1", "Version" : "621.01", "Name" : "GeForce%20Game%20Ready%20Driver", "ReleaseDateTime" : "Wed Oct 21, 2026", "DownloadURL" : "https://us.download.nvidia.com/Windows/621.01/621.01-desktop-win10-win11-64bit-international-dch-whql.exe", "DownloadURLFileSize" : "1.04 GB" }}]}</body></html>"#;
    let net = FakeNet::default()
        .on("https://www.catalog.update.microsoft.com/", 200, NO_RESULT)
        .on("https://gfwsl.geforce.com/", 200, json);
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[card.clone(), audio.clone()], &net, &mut guard, &context());
    let urls: Vec<_> = results
        .iter()
        .filter(|r| r.source == SourceId::Nvidia)
        .flat_map(|r| &r.offers)
        .map(|o| o.download_url.clone())
        .collect();
    assert_eq!(urls.len(), 2, "card and HD Audio, one package");
    assert_eq!(urls[0], urls[1]);
    let report = decide(&snapshot(vec![card.clone(), audio.clone()], results));
    let card_decision = &report.decisions[0];
    assert!(matches!(card_decision.state, State::Announced { .. }));
    let chosen = card_decision.chosen.as_ref().unwrap();
    assert_eq!(chosen.offer.vendor_version.as_deref(), Some("621.01"));
    assert!(
        !chosen.offer.installable,
        "graphics driver: never installed in phase 3"
    );
    // ADR-0034: HD Audio is in the list, updated by the same NVIDIA installer; R-VER-02: never
    // "update available" (Drivn does not know its version inside the package), only announced.
    let audio_decision = &report.decisions[1];
    assert!(matches!(audio_decision.state, State::Announced { .. }));
    let audio_offer = &audio_decision.chosen.as_ref().unwrap().offer;
    assert_eq!(audio_offer.vendor_version, chosen.offer.vendor_version);
    assert_eq!(audio_offer.source, chosen.offer.source);
    assert!(!audio_offer.installable);

    // No newer package for the card: HD Audio keeps only a hint.
    let mut newest = card_newest();
    newest.driver.as_mut().unwrap().version = Some("32.0.16.2101".to_owned());
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(
        &[newest.clone(), audio.clone()],
        &net,
        &mut guard,
        &context(),
    );
    let report = decide(&snapshot(vec![newest, audio], results));
    assert!(!matches!(
        report.decisions[1].state,
        State::Update | State::Announced { .. }
    ));
    assert_eq!(
        report.decisions[1].hints[0].kind,
        HintKind::PackageMayContainNewer
    );
}

// ---------- Readers ----------

#[test]
fn r_vrai_01_text_read_on_a_site_is_plain_and_cut() {
    let read = text::plain(&format!(
        "<b>Intel</b> &amp; <script>x</script>{}",
        "a".repeat(400)
    ));
    assert!(!read.contains('<'));
    assert!(read.starts_with("Intel & x"));
    assert_eq!(read.chars().count(), text::MAX_CHARS);
}

#[test]
fn readers_of_oracle_and_nvidia_versions() {
    assert_eq!(oracle::installed_java8_update("8.0.5110.9"), Some(511));
    assert_eq!(oracle::installed_java8_update("17.0.1"), None);
    assert_eq!(
        oracle::latest_java8_update("1.8.0_511\r\n1.7.0_521"),
        Some(511)
    );
    assert_eq!(nvidia::nvidia_version("32.0.16.1692"), Some((616, 92)));
    assert_eq!(nvidia::parse_release("621.01"), Some((621, 1)));
    assert_eq!(
        nvidia::product_id(&["PCI\\VEN_10DE&DEV_2702&SUBSYS_41411458&REV_A1".to_owned()]),
        Some(1041)
    );
    assert_eq!(
        nvidia::product_id(&["PCI\\VEN_10DE&DEV_9999".to_owned()]),
        None
    );
    assert_eq!(
        ms_catalog::search_term(MEI).as_deref(),
        Some("VEN_8086&DEV_7A68")
    );
    assert_eq!(
        ms_catalog::releases_from(Some("26H2")),
        ["26H2", "26H1", "25H2"]
    );
    assert_eq!(
        ms_catalog::releases_from(Some("24H2")),
        ["24H2", "23H2", "22H2"]
    );
}

// ---------- R-SRC-08: archive guards ----------

#[test]
fn r_src_08_archive_names_and_sizes_are_guarded() {
    use drivn_core::sources::archive::{Limits, safe_parts};
    assert_eq!(
        safe_parts("HDAudio/nvhda.inf").unwrap(),
        ["HDAudio", "nvhda.inf"]
    );
    for bad in [
        r"..\x.inf",
        "a/../../x",
        r"\abs.inf",
        "/abs.inf",
        "C:x.inf",
        r"\\?\C:\x",
        "a.inf:stream",
        "CON",
        "nul.txt",
        "dir/name.",
        "dir/name ",
        "",
    ] {
        assert!(safe_parts(bad).is_err(), "{bad}");
    }
    let mut limits = Limits::default();
    assert!(limits.admit("a.inf", 10, 1000).is_ok());
    assert!(
        limits.admit("A.INF", 10, 1000).is_err(),
        "same name, other case"
    );
    let mut bomb = Limits::default();
    assert!(
        bomb.admit("big.bin", 101_000, 1000).is_err(),
        "ratio above 100"
    );
}

#[test]
fn r_src_02_catalog_hash_is_compared_before_anything() {
    let bytes = b"cab";
    let good = drivn_core::install::plan::sha256_hex(bytes);
    assert!(ms_catalog::hash_matches(bytes, &good.to_uppercase()));
    assert!(!ms_catalog::hash_matches(b"cab!", &good));
}

// ---------- R-SRC-10: official tools ----------

#[test]
fn r_src_10_official_tool_runs_only_with_the_expected_signature() {
    use drivn_core::sources::tools::{Launch, decide_launch, page_url, tool};
    let java = tool("java").unwrap();
    assert_eq!(
        decide_launch(java, true, Ok("Oracle America, Inc.")),
        Launch::Run
    );
    assert!(matches!(
        decide_launch(java, false, Ok("Oracle America, Inc.")),
        Launch::LinkOnly(_)
    ));
    assert!(matches!(
        decide_launch(java, true, Ok("Contoso")),
        Launch::LinkOnly(_)
    ));
    assert!(matches!(
        decide_launch(java, true, Err("unsigned")),
        Launch::LinkOnly(_)
    ));
    assert!(page_url(java).starts_with("https:"));
    assert!(tool("other").is_none());
}

// ---------- R-SRC-08: nothing after the signed zone of an executable ----------

fn pe_header(cert_offset: u32, cert_size: u32) -> Vec<u8> {
    let mut h = vec![0u8; 1024];
    h[0..2].copy_from_slice(b"MZ");
    h[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    h[0x80..0x84].copy_from_slice(b"PE\0\0");
    let optional = 0x80 + 24;
    h[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    let entry = optional + 112 + 4 * 8;
    h[entry..entry + 4].copy_from_slice(&cert_offset.to_le_bytes());
    h[entry + 4..entry + 8].copy_from_slice(&cert_size.to_le_bytes());
    h
}

#[test]
fn r_src_08_bytes_after_the_signed_zone_are_refused() {
    use drivn_core::sources::pe::nothing_after_signature;
    let header = pe_header(10_000, 2_000);
    assert!(nothing_after_signature(&header, 12_000).is_ok());
    assert!(
        nothing_after_signature(&header, 12_007).is_ok(),
        "padding to 8"
    );
    assert!(
        nothing_after_signature(&header, 12_008).is_err(),
        "payload appended"
    );
    assert!(
        nothing_after_signature(&header, 11_000).is_err(),
        "table beyond the end"
    );
    assert!(
        nothing_after_signature(&pe_header(0, 0), 5_000).is_err(),
        "not signed"
    );
    assert!(nothing_after_signature(b"not a PE file", 13).is_err());
}

#[test]
fn r_src_11_a_windows_release_empty_for_two_devices_is_not_asked_again() {
    // Three Intel devices; 26H2 and 26H1 have nothing, 25H2 answers for all.
    let device = |dev: &str| Device {
        instance_id: format!(r"PCI\VEN_8086&DEV_{dev}\1"),
        hardware_ids: vec![format!(r"PCI\VEN_8086&DEV_{dev}&SUBSYS_88821043&REV_11")],
        ..mei()
    };
    let page = search_page(
        1,
        &[row(
            OLD_GUID,
            0,
            "Intel (2603.9.4.0)",
            "1/12/2026",
            "2603.9.4.0",
        )],
    );
    let mut net = FakeNet::default();
    for dev in ["7A01", "7A02", "7A03"] {
        net = net
            .on(&format!("https://www.catalog.update.microsoft.com/Search.aspx?q=VEN_8086%26DEV_{dev}%2026H2"), 200, NO_RESULT)
            .on(&format!("https://www.catalog.update.microsoft.com/Search.aspx?q=VEN_8086%26DEV_{dev}%2026H1"), 200, NO_RESULT)
            .on(&format!("https://www.catalog.update.microsoft.com/Search.aspx?q=VEN_8086%26DEV_{dev}%2025H2"), 200, &page);
    }
    let mut guard = Guard::new(GuardState::default(), NOW);
    check_vendors(
        &[device("7A01"), device("7A02"), device("7A03")],
        &net,
        &mut guard,
        &context(),
    );
    let urls = net.urls();
    assert_eq!(
        urls.iter().filter(|u| u.contains("26H2")).count(),
        2,
        "{urls:?}"
    );
    assert_eq!(urls.iter().filter(|u| u.contains("25H2")).count(), 3);
}

// ---------- ASUS (ADR-0032) ----------

const ASUS: &str = "https://rog.asus.com/support/webapi/ProductV2/";

fn asus_board(manufacturer: &str) -> Board {
    Board {
        system_manufacturer: Some("System manufacturer".to_owned()),
        manufacturer: Some(manufacturer.to_owned()),
        product: Some("ROG STRIX Z790-E GAMING WIFI".to_owned()),
        bios_version: Some("3001".to_owned()),
        bios_date: Some("05/19/2025".to_owned()),
    }
}

fn asus_file(title: &str, version: &str, date: &str) -> String {
    format!(
        r#"{{"Title":"{title}","Version":"{version}","ReleaseDate":"{date}","DownloadUrl":{{"Global":"/pub/x.zip"}},"sha256":"","HardwareInfoList":null}}"#
    )
}

fn asus_list(categories: &[(&str, Vec<String>)]) -> String {
    let obj: Vec<String> = categories
        .iter()
        .map(|(name, files)| {
            format!(
                r#"{{"Name":"{name}","Count":{},"Files":[{}]}}"#,
                files.len(),
                files.join(",")
            )
        })
        .collect();
    format!(
        r#"{{"Result":{{"Count":{},"Obj":[{}]}},"Status":"SUCCESS","Message":"SUCCESS"}}"#,
        obj.len(),
        obj.join(",")
    )
}

fn asus_net() -> FakeNet {
    FakeNet::default()
        .on(
            &format!("{ASUS}GetPDBIOS?website=global&model=rog-strix-z790-e-gaming-wifi"),
            200,
            &asus_list(&[(
                "BIOS",
                vec![
                    asus_file("", "3202", "2026/08/17"),
                    asus_file("", "3107", "2025/12/15"),
                ],
            )]),
        )
        .on(
            &format!(
                "{ASUS}GetPDDrivers?website=global&model=rog-strix-z790-e-gaming-wifi&osid=52"
            ),
            200,
            &asus_list(&[
                (
                    "Chipset",
                    vec![
                        asus_file(
                            "Intel ME V2627.10.5.0 for Windows 11 64-bit.",
                            "2627.10.5.0",
                            "2026/07/02",
                        ),
                        asus_file(
                            "Intel Chipset driver V10.1.20532.8842",
                            "10.1.20532.8842",
                            "2026/07/02",
                        ),
                    ],
                ),
                (
                    "Wireless",
                    vec![asus_file(
                        "Intel WiFi driver v24.20.0.4 for Windows 10/11 64-bit.",
                        "24.20.0.4",
                        "2026/03/09",
                    )],
                ),
                (
                    "LAN",
                    vec![asus_file(
                        "Intel  I226 LAN driver V2.1.5.10 for Windows 11 64-bit.",
                        "2.1.5.10",
                        "2026/03/31",
                    )],
                ),
            ]),
        )
}

fn lan(description: &str, version: &str) -> Device {
    let mut device = mei();
    device.instance_id = format!("PCI\\VEN_8086&DEV_125C\\{description}");
    device.description = Some(description.to_owned());
    device.class = Some("Net".to_owned());
    device.hardware_ids = vec!["PCI\\VEN_8086&DEV_125C&SUBSYS_88671043".to_owned()];
    if let Some(d) = device.driver.as_mut() {
        d.version = Some(version.to_owned());
    }
    device
}

#[test]
fn r_src_07_asus_only_when_the_motherboard_is_asus() {
    // A built PC: the system maker is the BIOS default text, the motherboard is ASUS.
    let board = asus_board("ASUSTeK COMPUTER INC.");
    let net = asus_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let ctx = VendorContext {
        board: Some(&board),
        ..context()
    };
    let results = check_vendors(&[], &net, &mut guard, &ctx);
    assert!(
        net.urls()
            .iter()
            .all(|u| u.starts_with("https://rog.asus.com/"))
    );
    assert_eq!(net.sent.borrow().len(), 2, "BIOS and drivers, nothing else");
    let asus = results.iter().find(|r| r.source == SourceId::Asus).unwrap();
    let bios = asus.bios.as_ref().unwrap();
    assert_eq!((bios.latest.as_str(), bios.newer), ("3202", Some(true)));
    assert_eq!(bios.latest_date.as_deref(), Some("2026-08-17"));

    // Another motherboard maker, or no board read: no request to ASUS at all.
    for other in [Some(asus_board("Micro-Star International Co., Ltd.")), None] {
        let net = asus_net();
        let mut guard = Guard::new(GuardState::default(), NOW);
        let ctx = VendorContext {
            board: other.as_ref(),
            ..context()
        };
        check_vendors(&[], &net, &mut guard, &ctx);
        assert!(net.sent.borrow().is_empty());
    }
}

#[test]
fn r_mat_01_asus_hint_never_makes_an_update() {
    let board = asus_board("ASUSTeK COMPUTER INC.");
    let net = asus_net();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let ctx = VendorContext {
        board: Some(&board),
        ..context()
    };
    let wifi = {
        let mut d = lan("Intel(R) Wi-Fi 6E AX211 160MHz", "1.0.0.0");
        d.instance_id = "PCI\\VEN_8086&DEV_7A70\\wifi".to_owned();
        d
    };
    // Seen on the reference PC: Microsoft's virtual adapter has "Wi-Fi" in its name, not an Intel driver.
    let virtual_wifi = {
        let mut d = lan("Microsoft Wi-Fi Direct Virtual Adapter", "10.0.0.1");
        d.instance_id = r"SWD\WIFIDIRECT\virtual".to_owned();
        if let Some(driver) = d.driver.as_mut() {
            driver.provider = Some("Microsoft".to_owned());
        }
        d
    };
    let devices = vec![
        windows_driver(mei()),
        lan("Intel(R) Ethernet Controller I226-V", "2.1.4.3"),
        lan("Intel(R) Ethernet Controller I225-V", "1.0.0.0"),
        wifi,
        virtual_wifi,
    ];
    let results = check_vendors(&devices, &net, &mut guard, &ctx);
    let asus = results.iter().find(|r| r.source == SourceId::Asus).unwrap();
    let hinted: Vec<&str> = asus
        .hints
        .iter()
        .map(|h| h.device_instance_id.as_str())
        .collect();
    // ME tied by family and name; the I226 driver only to the I226, never to the I225 or the
    // Wi-Fi; the Intel Wi-Fi driver to the Intel card, never to Microsoft's virtual adapter.
    assert_eq!(
        hinted,
        [
            devices[0].instance_id.as_str(),
            devices[1].instance_id.as_str(),
            devices[3].instance_id.as_str()
        ]
    );
    assert!(
        asus.hints
            .iter()
            .all(|h| h.kind == HintKind::PublisherListsNewer)
    );
    assert!(asus.offers.is_empty(), "an ASUS entry is never an offer");
    // The decision stays what Windows Update says: a hint is never "to update".
    let report = decide(&snapshot(devices.clone(), results));
    for decision in &report.decisions[..4] {
        if decision.hints.is_empty() {
            continue;
        }
        assert_ne!(decision.state, State::Update);
        assert!(!matches!(decision.state, State::Announced { .. }));
        assert_eq!(decision.hints.len(), 1);
    }
    // Not newer than the installed driver: no hint.
    let mut newest = mei();
    if let Some(d) = newest.driver.as_mut() {
        d.version = Some("2627.10.5.0".to_owned());
    }
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(&[newest], &net, &mut guard, &ctx);
    let asus = results.iter().find(|r| r.source == SourceId::Asus).unwrap();
    assert!(asus.hints.is_empty());
}

#[test]
fn r_vrai_04_model_unknown_at_asus_is_said_not_blocked() {
    let board = asus_board("ASUSTeK COMPUTER INC.");
    let net = FakeNet::default().on(
        ASUS,
        200,
        r#"{"Result":null,"Status":"FAIL","Message":"FAIL"}"#,
    );
    let mut guard = Guard::new(GuardState::default(), NOW);
    let ctx = VendorContext {
        board: Some(&board),
        ..context()
    };
    let results = check_vendors(&[], &net, &mut guard, &ctx);
    let asus = results.iter().find(|r| r.source == SourceId::Asus).unwrap();
    assert!(asus.error.as_deref().unwrap().contains("unknown at ASUS"));
    assert!(asus.bios.is_none());
    assert!(guard.blocked(SourceId::Asus, NOW + 1).is_none());
}

// ---------- Intel (ADR-0033) ----------

/// Intel alone, as one result in a list (the scan itself does not ask Intel, see below).
fn check_intel_only(
    devices: &[Device],
    net: &FakeNet,
    guard: &mut Guard,
    ctx: &VendorContext<'_>,
) -> Vec<VendorResult> {
    vec![check_intel(devices, net, guard, ctx)]
}

#[test]
fn r_src_11_intel_is_not_asked_while_it_refuses_drivn() {
    // 403 seen on 2026-11-01: the scan does not call Intel at all (ADR-0033).
    const { assert!(!ASK_INTEL) };
    let net = FakeNet::default();
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_vendors(
        &[intel_wifi("24.60.0.3", AX211_IDS)],
        &net,
        &mut guard,
        &context(),
    );
    assert!(net.urls().iter().all(|u| !u.contains("intel.com")));
    assert!(results.iter().all(|r| r.source != SourceId::Intel));
}

const INTEL_WIFI: &str = "https://www.intel.com/content/www/us/en/download/19351/";
const SHA: &str = "14b12299d6df2acb499de1d44d9243bb2bc4c050c1876af5abfd06c0b64100c33e357c8c9b2d587dd0792229029cb75797dfd33ffb3eb09b506ad787a611bece";

fn intel_page(products: &str, links: &[&str], hashes: &[&str]) -> String {
    let links: String = links
        .iter()
        .map(|l| format!(r#"<a data-href="{l}">Download</a>"#))
        .collect();
    let hashes: String = hashes
        .iter()
        .map(|h| format!(r#"<li class="detail">SHA512: {h}</li>"#))
        .collect();
    format!(
        r#"<html><head><meta name="DownloadVersion" content="24.80.0"><meta name="LastUpdate" content="10/20/2026 07:00:00">
        <meta name="RelatedProductIds" content="{products}"></head><body>{links}<li>Size: 72.3 MB</li>{hashes}
        <a href="https://downloadmirror.intel.com/929786/ReleaseNotes_WiFi_24.80.0.pdf">Notes</a></body></html>"#
    )
}

fn good_intel_page() -> String {
    intel_page(
        "246131,204837,204836",
        &["https://downloadmirror.intel.com/929786/WiFi-24.80.0-Driver64-Win10-Win11.exe"],
        &[SHA],
    )
}

fn intel_wifi(version: &str, ids: &[&str]) -> Device {
    let mut device = mei();
    device.instance_id =
        "PCI\\VEN_8086&DEV_7A70&SUBSYS_00948086&REV_11\\3&11583659&0&A3".to_owned();
    device.description = Some("Intel(R) Wi-Fi 6E AX211 160MHz".to_owned());
    device.class = Some("Net".to_owned());
    device.hardware_ids = ids.iter().map(|&id| id.to_owned()).collect();
    if let Some(d) = device.driver.as_mut() {
        d.version = Some(version.to_owned());
    }
    device
}

const AX211_IDS: &[&str] = &[
    "PCI\\VEN_8086&DEV_7A70&SUBSYS_00948086&REV_11",
    "PCI\\VEN_8086&DEV_7A70&SUBSYS_00948086",
    "PCI\\VEN_8086&DEV_7A70",
];

#[test]
fn r_mat_01_intel_table_id_must_equal_a_device_id() {
    let net = FakeNet::default().on(INTEL_WIFI, 200, &good_intel_page());
    let mut guard = Guard::new(GuardState::default(), NOW);
    // `DEV_7A700` starts like the table's ID but is not equal: not asked.
    let near = intel_wifi(
        "24.60.0.3",
        &["PCI\\VEN_8086&DEV_7A700", "PCI\\VEN_8086&DEV_7A71"],
    );
    let to_intel = |net: &FakeNet| {
        net.urls()
            .iter()
            .filter(|u| u.contains("intel.com"))
            .count()
    };
    check_intel_only(&[near], &net, &mut guard, &context());
    assert_eq!(to_intel(&net), 0);
    // A driver of Windows is not asked either (R-SRC-11).
    check_intel_only(
        &[windows_driver(intel_wifi("24.60.0.3", AX211_IDS))],
        &net,
        &mut guard,
        &context(),
    );
    assert_eq!(to_intel(&net), 0);
    let results = check_intel_only(
        &[intel_wifi("24.60.0.3", AX211_IDS)],
        &net,
        &mut guard,
        &context(),
    );
    let asked: Vec<String> = net
        .urls()
        .into_iter()
        .filter(|u| u.contains("intel.com"))
        .collect();
    assert_eq!(
        asked,
        [format!(
            "{INTEL_WIFI}intel-wireless-wi-fi-drivers-for-windows-10-and-windows-11.html"
        )]
    );
    let intel = results
        .iter()
        .find(|r| r.source == SourceId::Intel)
        .unwrap();
    assert_eq!(intel.offers.len(), 1);
    assert_eq!(intel.offers[0].hardware_id, "PCI\\VEN_8086&DEV_7A70");
}

#[test]
fn r_src_01_intel_package_link_has_the_exact_format() {
    assert!(intel::read_page(&good_intel_page()).is_ok());
    let page = intel::read_page(&good_intel_page()).unwrap();
    assert_eq!(page.package.host(), "downloadmirror.intel.com");
    assert_eq!(page.sha512, SHA);
    assert_eq!(page.date.as_deref(), Some("2026-10-20"));
    // Two packages, two hashes, no hash, a folder that is not a number: unreadable.
    let two = intel_page(
        "204837",
        &[
            "https://downloadmirror.intel.com/1/a.exe",
            "https://downloadmirror.intel.com/2/b.exe",
        ],
        &[SHA],
    );
    let other_sha = SHA.replace('1', "2");
    let cases = [
        two,
        intel_page(
            "204837",
            &["https://downloadmirror.intel.com/1/a.exe"],
            &[SHA, &other_sha],
        ),
        intel_page("204837", &["https://downloadmirror.intel.com/1/a.exe"], &[]),
        intel_page(
            "204837",
            &["https://downloadmirror.intel.com/x1/a.exe"],
            &[SHA],
        ),
        intel_page(
            "204837",
            &["https://downloadmirror.intel.com/1/sub/a.exe"],
            &[SHA],
        ),
        intel_page("204837", &["https://mirror.example.com/1/a.exe"], &[SHA]),
    ];
    for case in &cases {
        assert!(intel::read_page(case).is_err(), "{case}");
    }
}

#[test]
fn r_ver_03_intel_offer_is_announced_and_launched_by_intel() {
    let net = FakeNet::default().on(INTEL_WIFI, 200, &good_intel_page());
    let mut guard = Guard::new(GuardState::default(), NOW);
    let older = intel_wifi("24.60.0.3", AX211_IDS);
    let results = check_intel_only(std::slice::from_ref(&older), &net, &mut guard, &context());
    let intel = results
        .iter()
        .find(|r| r.source == SourceId::Intel)
        .unwrap();
    let offer = &intel.offers[0];
    assert!(offer.announced_newer && !offer.installable);
    assert_eq!(offer.vendor_version.as_deref(), Some("24.80.0"));
    assert_eq!(offer.sha512.as_deref(), Some(SHA));
    let report = decide(&snapshot(vec![older], results));
    assert!(matches!(report.decisions[0].state, State::Announced { .. }));

    // The package 24.80.0 covers the driver 24.80.0.2 (the reference PC): not newer.
    let same = intel_wifi("24.80.0.2", AX211_IDS);
    let results = check_intel_only(&[same], &net, &mut guard, &context());
    let intel = results
        .iter()
        .find(|r| r.source == SourceId::Intel)
        .unwrap();
    assert!(intel.offers.is_empty() && intel.unchecked.is_empty());

    // The page no longer lists the product of the table: unknown, never "up to date".
    let moved = FakeNet::default().on(
        INTEL_WIFI,
        200,
        &intel_page("1", &["https://downloadmirror.intel.com/1/a.exe"], &[SHA]),
    );
    let mut guard = Guard::new(GuardState::default(), NOW);
    let results = check_intel_only(
        &[intel_wifi("24.60.0.3", AX211_IDS)],
        &moved,
        &mut guard,
        &context(),
    );
    let intel = results
        .iter()
        .find(|r| r.source == SourceId::Intel)
        .unwrap();
    assert!(intel.offers.is_empty());
    assert_eq!(intel.unchecked.len(), 1);
}

#[test]
fn r_vrai_01_winget_installer_link_is_read_before_the_update() {
    use drivn_core::sources::winget::{parse_installer_url, show_args};
    let out = "Release Notes Url: https://github.com/x\nInstaller:\n  Installer Type: inno\n  Installer Url: https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.4/Git-2.56.0.4-64-bit.exe\n  Installer SHA256: 4961a4a8\n";
    assert_eq!(
        parse_installer_url(out).as_deref(),
        Some(
            "https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.4/Git-2.56.0.4-64-bit.exe"
        )
    );
    assert_eq!(
        parse_installer_url("Installer Url: http://plain.example/x.exe"),
        None
    );
    assert!(
        show_args("Git.Git")
            .is_some_and(|a| a[0] == "show" && !a.iter().any(|x| x.contains("accept")))
    );
    assert!(show_args("bad id; rm").is_none());
}
