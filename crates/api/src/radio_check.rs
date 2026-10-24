//! Pemeriksa stasiun radio (D7a).
//!
//! URL stream beberapa stasiun memuat token (`rj-tok`) yang bisa kedaluwarsa;
//! pemutar lalu hanya mendengar sunyi. Cron Trigger mengetuk stasiun aktif,
//! mencatat hasilnya di `radio_checks`, dan menandai `is_active = 0` kalau
//! kegagalannya meyakinkan. Pemeriksa **hanya menandai**: tautan baru tetap
//! ditempel admin lewat dashboard.
//!
//! Aturan yang disengaja:
//!
//! - **Hanya baca header.** Stream radio tidak pernah selesai, jadi
//!   [`Probe::get`] berhenti setelah header diterima dan memutus koneksi
//!   (`radio_probe.rs`).
//! - **Hanya URL yang pantas diperiksa** ([`probeable`]): `https`, nama host
//!   biasa (bukan IP, bukan `localhost`), tanpa userinfo, port 443. Yang lain
//!   dicatat `skipped` dan **tidak pernah** dimatikan. Port non-standar
//!   (mis. `:1936` milik Jak FM dan Gen98) dilewati karena `fetch` di
//!   Workers produksi dilaporkan mengabaikan port dan tetap ke 443; hasilnya
//!   akan gagal palsu. Belum terverifikasi: lihat README.
//! - **Redirect tidak diikuti otomatis.** Maksimal [`MAX_REDIRECTS`] lompatan,
//!   diikuti satu per satu, tiap tujuan harus lolos [`probeable`] lagi.
//! - **Satu kegagalan tidak mematikan stasiun.** Dibutuhkan
//!   [`FAIL_THRESHOLD`] kegagalan berturut-turut **dan** rentang
//!   [`MIN_FAIL_SPAN_SECS`] sejak kegagalan pertama.
//! - **Jika semua yang diperiksa dalam satu putaran gagal** (minimal
//!   [`OUTAGE_GUARD_MIN`] stasiun), putaran itu dianggap tidak meyakinkan:
//!   kemungkinan besar masalahnya di sisi Worker (IP Cloudflare diblokir,
//!   jaringan), bukan di semua stasiun sekaligus.
//! - **Tidak ada URL di log atau pesan error**: URL stream memuat token.
//!
//! Anggaran subrequest plan gratis (50 per eksekusi, termasuk panggilan ke
//! Turso): 1 baca + `BATCH x (1 + MAX_REDIRECTS)` + 1 tulis, dijaga oleh
//! `const` di bawah. Logika murni terhadap [`Client`] dan [`Probe`], dites
//! native dengan transport palsu.

use std::fmt;
use std::future::Future;

use futures_util::future::join_all;
use serde::Serialize;
use turso_http::{Client, Error, Stmt, Transport};

use crate::config::Config;
use crate::content::{opt_text, text};
use crate::time;

const DUE: &str = include_str!("queries/radio_check_due.sql");
const UPSERT: &str = include_str!("queries/radio_check_upsert.sql");
const DISABLE: &str = include_str!("queries/radio_check_disable.sql");
const CLEANUP: &str = include_str!("queries/radio_check_cleanup.sql");
const LIST: &str = include_str!("queries/admin_radio_list.sql");
const LAST: &str = include_str!("queries/admin_radio_last.sql");

/// Ekspresi cron pemeriksa (UTC): menit ke-17 tiap jam. Harus sama dengan salah
/// satu isi `crons` di `wrangler.toml` (ada tesnya); `scheduled` memakainya untuk
/// membedakan dari cron jurnal.
pub const CRON: &str = "17 * * * *";

/// Menit ke-berapa tiap jam cron berjalan (bagian dari [`CRON`]; ada tesnya).
pub const CRON_MINUTE: i64 = 17;
/// Jarak minimum antar "Cek sekarang" dari dashboard.
pub const MIN_MANUAL_SECS: i64 = 60;

/// Stasiun yang diperiksa per eksekusi (yang paling lama belum dicek lebih dulu).
pub const BATCH: u32 = 12;
/// Pemeriksaan serentak. Workers hanya menjalankan 6 koneksi yang menunggu
/// header sekaligus; sisanya antre.
pub const PARALLEL: usize = 6;
/// Lompatan redirect yang diikuti per stasiun.
pub const MAX_REDIRECTS: u8 = 2;
/// Kegagalan berturut-turut yang dibutuhkan sebelum stasiun dimatikan.
pub const FAIL_THRESHOLD: u32 = 3;
/// Selisih minimum antara kegagalan pertama dan saat mematikan (6 jam).
pub const MIN_FAIL_SPAN_SECS: i64 = 6 * 3600;
/// Jumlah minimum stasiun yang diperiksa agar "semuanya gagal" dianggap
/// gangguan di sisi Worker.
pub const OUTAGE_GUARD_MIN: usize = 3;
/// Batas penghitung kegagalan (sama dengan `CHECK` di migrasi 0011).
const MAX_FAILURES: u32 = 1000;

/// Subrequest plan gratis: 50 per eksekusi.
const SUBREQUEST_LIMIT: usize = 50;
const _: () = assert!(
    1 + BATCH as usize * (1 + MAX_REDIRECTS as usize) < SUBREQUEST_LIMIT,
    "radio check exceeds the Workers Free subrequest limit"
);

/// Subrequest di luar pemeriksaan pada "Cek sekarang": gerbang admin, membaca
/// waktu cek terakhir, mencatat riwayat, dan satu cadangan.
const MANUAL_OVERHEAD: usize = 4;
const _: () = assert!(
    2 + BATCH as usize * (1 + MAX_REDIRECTS as usize) + MANUAL_OVERHEAD <= SUBREQUEST_LIMIT,
    "manual radio check exceeds the Workers Free subrequest limit"
);

// -------------------------------------------------------------------- error

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioError {
    /// Database tidak terjangkau atau menolak query.
    Db(Error),
    /// Isi database tidak berbentuk yang diharapkan.
    BadData(&'static str),
    /// Pengaturan Worker belum lengkap (nama bagian yang kurang).
    NotConfigured(&'static str),
    /// "Cek sekarang" terlalu cepat setelah pemeriksaan sebelumnya.
    TooSoon { retry_after_secs: u64 },
}

impl fmt::Display for RadioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RadioError::Db(e) => write!(f, "{e}"),
            RadioError::BadData(what) => write!(f, "unexpected data: {what}"),
            RadioError::NotConfigured(what) => write!(f, "not configured: {what}"),
            RadioError::TooSoon { retry_after_secs } => {
                write!(f, "checked too recently, retry in {retry_after_secs}s")
            }
        }
    }
}

impl std::error::Error for RadioError {}

impl From<Error> for RadioError {
    fn from(e: Error) -> Self {
        RadioError::Db(e)
    }
}

// ------------------------------------------------------------------- probe

/// Jawaban satu permintaan tanpa mengikuti redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    /// Header `Location` (hanya berarti untuk redirect).
    pub location: Option<String>,
}

/// Kegagalan di level jaringan. Sengaja tanpa teks: pesan error `fetch` bisa
/// memuat URL (dan tokennya).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    Timeout,
    Network,
}

/// Pengetuk stream. Diimplementasikan oleh `fetch` di Worker
/// (`radio_probe.rs`) dan oleh transport palsu di test.
pub trait Probe {
    /// Satu `GET` tanpa mengikuti redirect; selesai begitu header diterima dan
    /// badan dibuang (stream tidak pernah selesai).
    fn get(&self, url: &str) -> impl Future<Output = Result<Reply, ProbeError>>;
}

// ------------------------------------------------------------------ hasil

/// Hasil satu pemeriksaan. Nilainya sama dengan `CHECK` `last_result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ok,
    /// Status bukan 2xx.
    Http,
    Network,
    Timeout,
    /// Redirect tanpa tujuan, atau berantai lebih dari [`MAX_REDIRECTS`].
    Redirect,
    /// Tidak diperiksa (URL tidak pantas diperiksa). Tidak pernah mematikan.
    Skipped,
    /// Tidak meyakinkan (429, atau gangguan di sisi Worker). Tidak dihitung.
    Inconclusive,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Ok => "ok",
            Kind::Http => "http",
            Kind::Network => "network",
            Kind::Timeout => "timeout",
            Kind::Redirect => "redirect",
            Kind::Skipped => "skipped",
            Kind::Inconclusive => "inconclusive",
        }
    }

    /// Kebalikan [`Kind::as_str`]; nilai lain (database diubah tangan) -> `None`.
    pub fn parse(s: &str) -> Option<Self> {
        [
            Kind::Ok,
            Kind::Http,
            Kind::Network,
            Kind::Timeout,
            Kind::Redirect,
            Kind::Skipped,
            Kind::Inconclusive,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    /// Hasil yang dihitung sebagai kegagalan stasiun.
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Kind::Http | Kind::Network | Kind::Timeout | Kind::Redirect
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub kind: Kind,
    /// Status HTTP terakhir yang diterima, bila ada.
    pub status: Option<u16>,
}

impl Outcome {
    fn of(kind: Kind, status: Option<u16>) -> Self {
        Self { kind, status }
    }
}

// ----------------------------------------------------------- URL yang boleh

fn authority(url_after_scheme: &str) -> &str {
    url_after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
}

/// Nama host DNS biasa: label `a-z0-9-`, ada titik, label terakhir diawali
/// huruf (menolak `1.2.3.4`, `0x7f.0.0.1`), bukan `localhost`/`.local`/`.internal`.
fn host_ok(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 || !host.contains('.') {
        return false;
    }
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let well_formed = labels.iter().all(|l| {
        (1..=63).contains(&l.len())
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    well_formed
        && labels
            .last()
            .is_some_and(|tld| tld.starts_with(|c: char| c.is_ascii_lowercase()))
}

/// URL yang boleh diperiksa Worker. Selain yang lolos ini, stasiun dicatat
/// `skipped`, bukan dianggap mati.
pub fn probeable(url: &str) -> bool {
    if !api_types::validate::is_http_url(url) {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = authority(rest);
    if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
        return false;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    port.is_none_or(|p| p == "443") && host_ok(host)
}

/// Tujuan redirect: `https://...` mutlak, atau path (`/x`) pada host yang sama.
/// `None` bila bentuknya lain, atau tujuannya tidak pantas diperiksa.
pub fn next_hop(current: &str, location: &str) -> Option<String> {
    let loc = location.trim();
    if loc.is_empty() {
        return None;
    }
    let target = if loc.starts_with("https://") {
        loc.to_owned()
    } else if loc.starts_with('/') && !loc.starts_with("//") && !loc.starts_with("/\\") {
        let rest = current.strip_prefix("https://")?;
        format!("https://{}{loc}", authority(rest))
    } else {
        return None;
    };
    probeable(&target).then_some(target)
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Memeriksa satu URL: mengikuti redirect satu per satu (maksimal
/// [`MAX_REDIRECTS`]) dan menilai status akhirnya.
pub async fn probe_url<P: Probe>(probe: &P, url: &str) -> Outcome {
    if !probeable(url) {
        return Outcome::of(Kind::Skipped, None);
    }
    let mut current = url.to_owned();
    for hop in 0..=MAX_REDIRECTS {
        let reply = match probe.get(&current).await {
            Ok(r) => r,
            Err(ProbeError::Timeout) => return Outcome::of(Kind::Timeout, None),
            Err(ProbeError::Network) => return Outcome::of(Kind::Network, None),
        };
        let status = Some(reply.status);
        match reply.status {
            200..=299 => return Outcome::of(Kind::Ok, status),
            // Server membatasi kecepatan kita: bukan kesalahan stasiun.
            429 => return Outcome::of(Kind::Inconclusive, status),
            s if is_redirect(s) => {
                let Some(location) = reply.location else {
                    return Outcome::of(Kind::Redirect, status);
                };
                if hop == MAX_REDIRECTS {
                    return Outcome::of(Kind::Redirect, status);
                }
                match next_hop(&current, &location) {
                    Some(next) => current = next,
                    // Tujuan yang tidak boleh kita ketuk: tidak bisa dinilai.
                    None => return Outcome::of(Kind::Skipped, status),
                }
            }
            _ => return Outcome::of(Kind::Http, status),
        }
    }
    Outcome::of(Kind::Redirect, None)
}

// ---------------------------------------------------------------- keadaan

/// Keadaan tersimpan satu stasiun (detik UTC).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub failures: u32,
    pub failing_since: Option<i64>,
    pub last_ok: Option<i64>,
    /// Terisi bila pemeriksa sendiri yang mematikan stasiun.
    pub disabled_at: Option<i64>,
}

/// Keadaan berikutnya, dan apakah stasiun harus dimatikan sekarang.
///
/// Stasiun yang diperiksa pasti aktif. Kalau `disabled_at` masih terisi,
/// berarti admin mengaktifkannya lagi setelah dimatikan pemeriksa: mulai dari
/// nol, supaya satu kegagalan berikutnya tidak langsung mematikannya lagi.
pub fn advance(prior: &State, kind: Kind, now: i64) -> (State, bool) {
    let base = if prior.disabled_at.is_some() {
        State {
            last_ok: prior.last_ok,
            ..State::default()
        }
    } else {
        prior.clone()
    };
    match kind {
        Kind::Ok => (
            State {
                failures: 0,
                failing_since: None,
                last_ok: Some(now),
                disabled_at: None,
            },
            false,
        ),
        Kind::Skipped | Kind::Inconclusive => (base, false),
        Kind::Http | Kind::Network | Kind::Timeout | Kind::Redirect => {
            let failures = base.failures.saturating_add(1).min(MAX_FAILURES);
            let since = base.failing_since.unwrap_or(now);
            let disable = failures >= FAIL_THRESHOLD && now - since >= MIN_FAIL_SPAN_SECS;
            (
                State {
                    failures,
                    failing_since: Some(since),
                    last_ok: base.last_ok,
                    disabled_at: disable.then_some(now),
                },
                disable,
            )
        }
    }
}

// ----------------------------------------------------------------- putaran

/// Ringkasan satu putaran (untuk log dan respons "Cek sekarang"; tanpa URL).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub checked: usize,
    pub ok: usize,
    pub failed: usize,
    pub skipped: usize,
    pub inconclusive: usize,
    /// Id stasiun yang dimatikan putaran ini.
    pub disabled: Vec<u32>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "checked={} ok={} failed={} skipped={} inconclusive={} disabled={:?}",
            self.checked, self.ok, self.failed, self.skipped, self.inconclusive, self.disabled
        )
    }
}

struct Due {
    id: u32,
    url: String,
    state: State,
}

fn int(res: &turso_http::StmtResult, row: usize, col: &str) -> Option<i64> {
    res.get(row, col).and_then(turso_http::Value::as_i64)
}

/// Waktu yang tidak terbaca dianggap tidak ada (bukan galat): isinya bisa
/// diubah lewat database, dan salah baca hanya berarti mulai dari nol.
fn stamp(res: &turso_http::StmtResult, row: usize, col: &'static str) -> Option<i64> {
    opt_text(res, row, col)
        .ok()
        .flatten()
        .and_then(|s| time::parse_utc(&s))
}

async fn load_due<T: Transport>(db: &Client<T>) -> Result<Vec<Due>, RadioError> {
    let res = db.execute(Stmt::new(DUE).bind(BATCH)).await?;
    let mut due = Vec::with_capacity(res.rows.len());
    for row in 0..res.rows.len() {
        let id = int(&res, row, "id")
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(RadioError::BadData("id"))?;
        let url = text(&res, row, "stream_url").map_err(|_| RadioError::BadData("stream_url"))?;
        let checked_url = opt_text(&res, row, "checked_url")
            .ok()
            .flatten()
            .unwrap_or_default();
        // Tautan sudah diganti sejak cek terakhir: hasil lama bukan milik URL ini.
        let state = if checked_url == url {
            State {
                failures: int(&res, row, "failures")
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                failing_since: stamp(&res, row, "failing_since"),
                last_ok: stamp(&res, row, "last_ok_at"),
                disabled_at: stamp(&res, row, "disabled_at"),
            }
        } else {
            State::default()
        };
        due.push(Due { id, url, state });
    }
    Ok(due)
}

fn upsert(due: &Due, kind: Kind, status: Option<u16>, next: &State, now: i64) -> Stmt {
    let at = |s: Option<i64>| s.map(time::format_utc);
    Stmt::new(UPSERT)
        .bind(due.id)
        .bind(due.url.as_str())
        .bind(time::format_utc(now))
        .bind(at(next.last_ok))
        .bind(at(next.failing_since))
        .bind(next.failures)
        .bind(kind.as_str())
        .bind(status.map(i64::from))
        .bind(at(next.disabled_at))
        .no_rows()
}

/// Satu putaran: baca stasiun aktif yang paling lama belum dicek, ketuk,
/// lalu simpan hasilnya (dan matikan yang pantas) dalam satu transaksi.
pub async fn check<T: Transport, P: Probe>(
    db: &Client<T>,
    probe: &P,
    now_secs: i64,
) -> Result<Report, RadioError> {
    let due = load_due(db).await?;
    let mut report = Report::default();
    if due.is_empty() {
        return Ok(report);
    }

    let mut outcomes: Vec<Outcome> = Vec::with_capacity(due.len());
    for chunk in due.chunks(PARALLEL) {
        outcomes.extend(join_all(chunk.iter().map(|d| probe_url(probe, &d.url))).await);
    }

    // Semua yang diperiksa gagal sekaligus = curiga pada sisi kita.
    let judged = outcomes
        .iter()
        .filter(|o| o.kind == Kind::Ok || o.kind.is_failure())
        .count();
    let failing = outcomes.iter().filter(|o| o.kind.is_failure()).count();
    if judged >= OUTAGE_GUARD_MIN && failing == judged {
        for o in &mut outcomes {
            if o.kind.is_failure() {
                o.kind = Kind::Inconclusive;
            }
        }
    }

    let mut stmts = Vec::with_capacity(due.len() * 2 + 1);
    let mut disables = Vec::new();
    for (d, o) in due.iter().zip(&outcomes) {
        let (next, disable) = advance(&d.state, o.kind, now_secs);
        stmts.push(upsert(d, o.kind, o.status, &next, now_secs));
        if disable {
            disables.push(Stmt::new(DISABLE).bind(d.id).bind(d.url.as_str()).no_rows());
            report.disabled.push(d.id);
        }
        match o.kind {
            Kind::Ok => report.ok += 1,
            Kind::Skipped => report.skipped += 1,
            Kind::Inconclusive => report.inconclusive += 1,
            _ => report.failed += 1,
        }
    }
    report.checked = due.len();
    stmts.extend(disables);
    stmts.push(Stmt::new(CLEANUP).no_rows());
    db.transaction(stmts).await?;
    Ok(report)
}

// ------------------------------------------------------- dashboard (D7b)

/// Status satu stasiun untuk dashboard. **Tanpa URL stream** (memuat token);
/// hanya hasil pemeriksaannya.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StationStatus {
    pub id: u32,
    pub label: String,
    pub is_active: bool,
    /// `false` bila URL-nya tidak boleh diketuk Worker (dilewati, bukan mati).
    pub checkable: bool,
    /// Tautan diganti sejak pemeriksaan terakhir: hasil di bawah milik tautan lama.
    pub link_changed: bool,
    pub last_result: Option<String>,
    pub last_status: Option<u16>,
    pub last_checked_at: Option<String>,
    pub last_ok_at: Option<String>,
    pub failing_since: Option<String>,
    pub consecutive_failures: u32,
    /// Dimatikan oleh pemeriksa (bukan oleh admin) dan belum diaktifkan lagi.
    pub disabled_by_check: bool,
}

/// Aturan yang berlaku, supaya dashboard tidak menyalin angkanya.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rules {
    pub fail_threshold: u32,
    pub min_fail_hours: u32,
    pub batch: u32,
    pub min_manual_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RadioStatus {
    /// Jam server (UTC), supaya "x menit lalu" tidak bergantung jam perangkat.
    pub now: String,
    /// Kapan cron otomatis berikutnya berjalan.
    pub next_run_at: String,
    pub rules: Rules,
    pub items: Vec<StationStatus>,
}

/// Saat cron berikutnya (detik UTC): menit [`CRON_MINUTE`] jam berikutnya.
pub fn next_run(now_secs: i64) -> i64 {
    let this_hour = now_secs.div_euclid(3600) * 3600 + CRON_MINUTE * 60;
    if this_hour > now_secs {
        this_hour
    } else {
        this_hour + 3600
    }
}

fn clean_stamp(res: &turso_http::StmtResult, row: usize, col: &'static str) -> Option<String> {
    stamp(res, row, col).map(time::format_utc)
}

/// Status semua stasiun (aktif maupun tidak), urutan seperti di pemutar.
pub async fn status<T: Transport>(
    db: &Client<T>,
    now_secs: i64,
) -> Result<RadioStatus, RadioError> {
    let res = db.execute(Stmt::new(LIST)).await?;
    let mut items = Vec::with_capacity(res.rows.len());
    for row in 0..res.rows.len() {
        let id = int(&res, row, "id")
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(RadioError::BadData("id"))?;
        let label = text(&res, row, "label").map_err(|_| RadioError::BadData("label"))?;
        let url = text(&res, row, "stream_url").map_err(|_| RadioError::BadData("stream_url"))?;
        let is_active = int(&res, row, "is_active").ok_or(RadioError::BadData("is_active"))? == 1;
        let checked_url = opt_text(&res, row, "checked_url").ok().flatten();
        let last_result = opt_text(&res, row, "last_result")
            .ok()
            .flatten()
            .and_then(|r| Kind::parse(&r));
        items.push(StationStatus {
            id,
            label,
            is_active,
            checkable: probeable(&url),
            link_changed: checked_url.is_some_and(|c| c != url),
            last_result: last_result.map(|k| k.as_str().to_owned()),
            last_status: int(&res, row, "last_status").and_then(|n| u16::try_from(n).ok()),
            last_checked_at: clean_stamp(&res, row, "last_checked_at"),
            last_ok_at: clean_stamp(&res, row, "last_ok_at"),
            failing_since: clean_stamp(&res, row, "failing_since"),
            consecutive_failures: int(&res, row, "failures")
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            disabled_by_check: !is_active && stamp(&res, row, "disabled_at").is_some(),
        });
    }
    Ok(RadioStatus {
        now: time::format_utc(now_secs),
        next_run_at: time::format_utc(next_run(now_secs)),
        rules: Rules {
            fail_threshold: FAIL_THRESHOLD,
            min_fail_hours: u32::try_from(MIN_FAIL_SPAN_SECS / 3600).unwrap_or(0),
            batch: BATCH,
            min_manual_secs: MIN_MANUAL_SECS,
        },
        items,
    })
}

/// "Cek sekarang": seperti satu putaran cron, tetapi menolak bila pemeriksaan
/// terakhir (cron atau manual) kurang dari [`MIN_MANUAL_SECS`] yang lalu.
pub async fn check_now<T: Transport, P: Probe>(
    db: &Client<T>,
    probe: &P,
    now_secs: i64,
) -> Result<Report, RadioError> {
    let res = db.execute(Stmt::new(LAST)).await?;
    if let Some(last) = stamp(&res, 0, "at") {
        // Jam yang mundur (last > now) tidak boleh mengunci lebih dari batasnya.
        let wait = (last + MIN_MANUAL_SECS - now_secs).min(MIN_MANUAL_SECS);
        if wait > 0 {
            return Err(RadioError::TooSoon {
                retry_after_secs: wait.unsigned_abs(),
            });
        }
    }
    check(db, probe, now_secs).await
}

/// Jalur cron: bangun klien dari konfigurasi lalu [`check`].
pub async fn run(config: &Config, now_secs: i64) -> Result<Report, RadioError> {
    let Some(cfg) = &config.db else {
        return Err(RadioError::NotConfigured("database"));
    };
    let db = crate::db::client(cfg)?;
    check(&db, &crate::radio_probe::WorkerProbe, now_secs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{client, execute_reply, result, tx_reply, Script};
    use serde_json::{json, Value as Json};
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn now() -> i64 {
        time::days_from_civil(2026, 10, 24) * 86_400 + 12 * 3600
    }

    // ----------------------------------------------------------- probe palsu

    #[derive(Default)]
    struct Fake {
        replies: RefCell<HashMap<String, Vec<Result<Reply, ProbeError>>>>,
        seen: RefCell<Vec<String>>,
    }

    impl Fake {
        fn on(self, url: &str, replies: Vec<Result<Reply, ProbeError>>) -> Self {
            self.replies.borrow_mut().insert(url.to_owned(), replies);
            self
        }
        fn seen(&self) -> Vec<String> {
            self.seen.borrow().clone()
        }
    }

    impl Probe for Fake {
        fn get(&self, url: &str) -> impl Future<Output = Result<Reply, ProbeError>> {
            self.seen.borrow_mut().push(url.to_owned());
            let reply = {
                let mut all = self.replies.borrow_mut();
                let queue = all.get_mut(url).expect("unscripted probe");
                assert!(!queue.is_empty(), "probe scripted too few replies");
                queue.remove(0)
            };
            async move { reply }
        }
    }

    fn status(s: u16) -> Result<Reply, ProbeError> {
        Ok(Reply {
            status: s,
            location: None,
        })
    }

    fn redirect(s: u16, to: &str) -> Result<Reply, ProbeError> {
        Ok(Reply {
            status: s,
            location: Some(to.to_owned()),
        })
    }

    fn probe_one(fake: &Fake, url: &str) -> Outcome {
        futures_executor::block_on(probe_url(fake, url))
    }

    // ------------------------------------------------------------- probeable

    #[test]
    fn only_plain_https_hosts_on_the_standard_port_are_probed() {
        for ok in [
            "https://stream.radiojar.com/7csmg90fuqruv?1780884834=",
            "https://n05-us.rcs.revma.com/k02rmq48kxcwv?rj-ttl=5&rj-tok=AAAB&age=18+",
            "https://a.example:443/x",
            "https://Radio.Example.ID/stream",
            "https://xn--bcher-kva.example/s",
        ] {
            assert!(probeable(ok), "{ok}");
        }
        for bad in [
            "http://a.example/x",
            "https://wz.mari.co.id:1936/noice_jakfm/jakfm/chunklist_w644295007.m3u8",
            "https://a.example:80/x",
            "https://a.example:/x",
            "https://127.0.0.1/x",
            "https://10.0.0.8/x",
            "https://0x7f.0.0.1/x",
            "https://2130706433/x",
            "https://[::1]/x",
            "https://localhost/x",
            "https://printer.local/x",
            "https://db.internal/x",
            "https://user:pw@a.example/x",
            "https://a.example@evil.example/x",
            "https://nodot/x",
            "https://-a.example/x",
            "https://a_b.example/x",
            "https://a.example/x y",
            "https://",
            "ftp://a.example",
            "HTTPS://a.example/x",
            "/relative",
            "",
        ] {
            assert!(!probeable(bad), "{bad}");
        }
    }

    #[test]
    fn redirect_targets_are_absolute_https_or_same_host_paths() {
        let cur = "https://a.example/s?tok=1";
        assert_eq!(
            next_hop(cur, "https://b.example/next").as_deref(),
            Some("https://b.example/next")
        );
        assert_eq!(
            next_hop(cur, " /live/2 ").as_deref(),
            Some("https://a.example/live/2")
        );
        for bad in [
            "",
            "http://b.example/x",
            "//evil.example/x",
            "/\\evil.example",
            "relative/path",
            "https://b.example:8000/x",
            "https://127.0.0.1/x",
            "ftp://b.example/x",
            "javascript:alert(1)",
        ] {
            assert_eq!(next_hop(cur, bad), None, "{bad:?}");
        }
    }

    // ------------------------------------------------------------- probe_url

    #[test]
    fn a_2xx_status_is_ok_and_other_statuses_are_judged_by_what_they_say() {
        let url = "https://a.example/s";
        for (s, kind) in [
            (200, Kind::Ok),
            (206, Kind::Ok),
            (401, Kind::Http),
            (403, Kind::Http),
            (404, Kind::Http),
            (410, Kind::Http),
            (500, Kind::Http),
            (503, Kind::Http),
            (304, Kind::Http),
            (429, Kind::Inconclusive),
        ] {
            let fake = Fake::default().on(url, vec![status(s)]);
            let got = probe_one(&fake, url);
            assert_eq!(got, Outcome::of(kind, Some(s)), "{s}");
            assert_eq!(fake.seen().len(), 1);
        }
    }

    #[test]
    fn network_failures_are_failures_without_a_status() {
        let url = "https://a.example/s";
        let fake = Fake::default().on(url, vec![Err(ProbeError::Timeout)]);
        assert_eq!(probe_one(&fake, url), Outcome::of(Kind::Timeout, None));
        let fake = Fake::default().on(url, vec![Err(ProbeError::Network)]);
        assert_eq!(probe_one(&fake, url), Outcome::of(Kind::Network, None));
    }

    #[test]
    fn redirects_are_followed_one_at_a_time_up_to_the_limit() {
        let a = "https://a.example/s";
        let b = "https://b.example/s2";
        let c = "https://c.example/s3";
        let fake = Fake::default()
            .on(a, vec![redirect(302, b)])
            .on(b, vec![redirect(301, c)])
            .on(c, vec![status(200)]);
        assert_eq!(probe_one(&fake, a), Outcome::of(Kind::Ok, Some(200)));
        assert_eq!(fake.seen(), [a, b, c], "MAX_REDIRECTS hops = 3 requests");

        // Satu lompatan lagi dari batas: gagal, dan tidak ada permintaan ke-4.
        let d = "https://d.example/s4";
        let fake = Fake::default()
            .on(a, vec![redirect(302, b)])
            .on(b, vec![redirect(302, c)])
            .on(c, vec![redirect(302, d)]);
        assert_eq!(probe_one(&fake, a), Outcome::of(Kind::Redirect, Some(302)));
        assert_eq!(fake.seen().len(), 3);
    }

    #[test]
    fn a_redirect_loop_is_cut_off() {
        let a = "https://a.example/s";
        let fake = Fake::default().on(
            a,
            vec![redirect(302, a), redirect(302, a), redirect(302, a)],
        );
        assert_eq!(probe_one(&fake, a), Outcome::of(Kind::Redirect, Some(302)));
        assert_eq!(fake.seen().len(), 3);
    }

    #[test]
    fn a_redirect_without_location_fails_but_one_we_may_not_follow_is_skipped() {
        let a = "https://a.example/s";
        let fake = Fake::default().on(a, vec![status(302)]);
        assert_eq!(probe_one(&fake, a), Outcome::of(Kind::Redirect, Some(302)));

        for to in [
            "http://b.example/s",
            "https://b.example:8000/s",
            "https://127.0.0.1/s",
        ] {
            let fake = Fake::default().on(a, vec![redirect(302, to)]);
            assert_eq!(probe_one(&fake, a), Outcome::of(Kind::Skipped, Some(302)));
            assert_eq!(fake.seen().len(), 1, "never connects to {to}");
        }
    }

    #[test]
    fn a_url_we_may_not_probe_is_skipped_without_any_request() {
        let fake = Fake::default();
        for url in [
            "http://a.example/s",
            "https://wz.mari.co.id:1936/x.m3u8",
            "https://127.0.0.1/s",
        ] {
            assert_eq!(probe_one(&fake, url), Outcome::of(Kind::Skipped, None));
        }
        assert!(fake.seen().is_empty());
    }

    // --------------------------------------------------------------- advance

    const HOUR: i64 = 3600;

    #[test]
    fn success_clears_the_failure_streak() {
        let prior = State {
            failures: 2,
            failing_since: Some(now() - 5 * HOUR),
            last_ok: Some(now() - 9 * HOUR),
            disabled_at: None,
        };
        let (next, disable) = advance(&prior, Kind::Ok, now());
        assert!(!disable);
        assert_eq!(
            next,
            State {
                failures: 0,
                failing_since: None,
                last_ok: Some(now()),
                disabled_at: None,
            }
        );
    }

    #[test]
    fn one_failure_never_disables() {
        let (next, disable) = advance(&State::default(), Kind::Http, now());
        assert!(!disable);
        assert_eq!(next.failures, 1);
        assert_eq!(next.failing_since, Some(now()));
    }

    #[test]
    fn enough_failures_in_a_short_time_do_not_disable() {
        let mut state = State::default();
        for i in 0..5 {
            let (next, disable) = advance(&state, Kind::Http, now() + i * 60);
            assert!(!disable, "5 failures within 5 minutes");
            state = next;
        }
        assert_eq!(state.failures, 5);
        assert_eq!(
            state.failing_since,
            Some(now()),
            "the streak keeps its start"
        );
    }

    #[test]
    fn enough_failures_over_enough_time_disable() {
        let since = now() - MIN_FAIL_SPAN_SECS;
        let two = State {
            failures: FAIL_THRESHOLD - 1,
            failing_since: Some(since),
            ..State::default()
        };
        let (next, disable) = advance(&two, Kind::Timeout, now());
        assert!(disable);
        assert_eq!(next.disabled_at, Some(now()));
        assert_eq!(next.failures, FAIL_THRESHOLD);

        // Waktunya cukup tetapi hitungannya belum: belum.
        let one = State {
            failures: 1,
            failing_since: Some(since),
            ..State::default()
        };
        assert!(!advance(&one, Kind::Timeout, now()).1);

        // Hitungannya cukup tetapi waktunya satu detik kurang: belum.
        let early = State {
            failures: FAIL_THRESHOLD - 1,
            failing_since: Some(since + 1),
            ..State::default()
        };
        assert!(!advance(&early, Kind::Timeout, now()).1);
    }

    #[test]
    fn skipped_and_inconclusive_leave_the_streak_alone() {
        let prior = State {
            failures: 2,
            failing_since: Some(now() - 7 * HOUR),
            last_ok: Some(now() - 20 * HOUR),
            disabled_at: None,
        };
        for kind in [Kind::Skipped, Kind::Inconclusive] {
            let (next, disable) = advance(&prior, kind, now());
            assert!(!disable);
            assert_eq!(next, prior);
        }
    }

    #[test]
    fn a_station_re_enabled_by_the_admin_starts_from_zero() {
        let prior = State {
            failures: 3,
            failing_since: Some(now() - 30 * HOUR),
            last_ok: Some(now() - 40 * HOUR),
            disabled_at: Some(now() - 24 * HOUR),
        };
        let (next, disable) = advance(&prior, Kind::Http, now());
        assert!(!disable, "one failure after re-enabling must not disable");
        assert_eq!(next.failures, 1);
        assert_eq!(next.failing_since, Some(now()));
        assert_eq!(next.last_ok, prior.last_ok);
        assert_eq!(next.disabled_at, None);
        // Dan tetap bersih kalau hasilnya bukan kegagalan.
        assert_eq!(advance(&prior, Kind::Skipped, now()).0.failures, 0);
    }

    #[test]
    fn the_failure_counter_stops_at_the_column_limit() {
        let prior = State {
            failures: MAX_FAILURES,
            failing_since: Some(now() - 100 * HOUR),
            ..State::default()
        };
        assert_eq!(advance(&prior, Kind::Http, now()).0.failures, MAX_FAILURES);
    }

    // ----------------------------------------------------------------- check

    const DUE_COLS: [&str; 7] = [
        "id",
        "stream_url",
        "checked_url",
        "failures",
        "failing_since",
        "last_ok_at",
        "disabled_at",
    ];

    fn due_row(id: i64, url: &str, state: Option<(&str, i64, Option<&str>)>) -> Vec<Json> {
        match state {
            None => vec![
                json!(id),
                json!(url),
                Json::Null,
                Json::Null,
                Json::Null,
                Json::Null,
                Json::Null,
            ],
            Some((checked, failures, since)) => vec![
                json!(id),
                json!(url),
                json!(checked),
                json!(failures),
                since.map_or(Json::Null, |s| json!(s)),
                Json::Null,
                Json::Null,
            ],
        }
    }

    fn due_reply(rows: &[Vec<Json>]) -> Result<turso_http::HttpResponse, String> {
        execute_reply(result(&DUE_COLS, rows))
    }

    fn written(n: usize) -> Result<turso_http::HttpResponse, String> {
        tx_reply((0..n).map(|_| result(&[], &[])).collect())
    }

    fn arg_text(script: &Script, stmt: usize, i: usize) -> String {
        script.args(1, stmt)[i]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    fn arg_null(script: &Script, stmt: usize, i: usize) -> bool {
        script.args(1, stmt)[i]["type"] == "null"
    }

    fn arg_int(script: &Script, stmt: usize, i: usize) -> String {
        script.args(1, stmt)[i]["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    fn run_check(script: &Script, fake: &Fake) -> Result<Report, RadioError> {
        futures_executor::block_on(check(&client(script), fake, now()))
    }

    #[test]
    fn nothing_due_costs_one_database_request_and_no_probes() {
        let script = Script::new(vec![due_reply(&[])]);
        let fake = Fake::default();
        let report = run_check(&script, &fake).unwrap();
        assert_eq!(report, Report::default());
        assert_eq!(script.requests(), 1);
        assert!(fake.seen().is_empty());
    }

    #[test]
    fn a_healthy_station_is_recorded_as_ok_and_nothing_is_disabled() {
        let url = "https://a.example/s?rj-tok=SECRET";
        let script = Script::new(vec![due_reply(&[due_row(7, url, None)]), written(2)]);
        let fake = Fake::default().on(url, vec![status(200)]);
        let report = run_check(&script, &fake).unwrap();
        assert_eq!((report.checked, report.ok, report.failed), (1, 1, 0));
        assert!(report.disabled.is_empty());
        assert_eq!(script.requests(), 2);

        // Transaksi: BEGIN, upsert, cleanup.
        let body = script.body(1);
        assert!(body.contains("INSERT INTO radio_checks"));
        assert!(body.contains("DELETE FROM radio_checks"));
        assert!(!body.contains("UPDATE radio_stations"));
        assert_eq!(arg_text(&script, 1, 6), "ok");
        assert_eq!(arg_text(&script, 1, 2), time::format_utc(now()));
        assert_eq!(arg_text(&script, 1, 3), time::format_utc(now()), "last_ok");
        assert!(arg_null(&script, 1, 4), "no failing_since");
        assert_eq!(arg_int(&script, 1, 5), "0");
        assert!(arg_null(&script, 1, 8), "not disabled");
    }

    #[test]
    fn the_limit_of_the_read_is_the_batch_size() {
        let script = Script::new(vec![due_reply(&[])]);
        run_check(&script, &Fake::default()).unwrap();
        assert!(script.body(0).contains(&BATCH.to_string()));
        assert_eq!(script.args(0, 0).len(), 1);
    }

    #[test]
    fn a_long_failing_station_is_disabled_only_for_the_url_that_was_checked() {
        let url = "https://a.example/s?rj-tok=OLD";
        let since = time::format_utc(now() - 7 * HOUR);
        let healthy = "https://b.example/live";
        let script = Script::new(vec![
            due_reply(&[
                due_row(1, url, Some((url, 2, Some(&since)))),
                due_row(2, healthy, None),
            ]),
            written(4),
        ]);
        let fake = Fake::default()
            .on(url, vec![status(403)])
            .on(healthy, vec![status(200)]);
        let report = run_check(&script, &fake).unwrap();
        assert_eq!(report.disabled, [1]);
        assert_eq!((report.ok, report.failed), (1, 1));

        let body = script.body(1);
        assert!(body.contains("UPDATE radio_stations"));
        // Urutan: BEGIN, upsert 1, upsert 2, disable, cleanup.
        assert_eq!(arg_text(&script, 1, 6), "http");
        assert_eq!(arg_int(&script, 1, 7), "403");
        assert_eq!(arg_int(&script, 1, 5), "3");
        assert_eq!(arg_text(&script, 1, 8), time::format_utc(now()));
        assert_eq!(arg_int(&script, 3, 0), "1");
        assert_eq!(
            arg_text(&script, 3, 1),
            url,
            "the disable is bound to the URL checked"
        );
    }

    #[test]
    fn a_station_that_failed_only_recently_is_not_disabled() {
        let url = "https://a.example/s";
        let since = time::format_utc(now() - HOUR);
        let script = Script::new(vec![
            due_reply(&[due_row(1, url, Some((url, 2, Some(&since))))]),
            written(2),
        ]);
        let fake = Fake::default().on(url, vec![status(404)]);
        let report = run_check(&script, &fake).unwrap();
        assert!(report.disabled.is_empty());
        assert!(!script.body(1).contains("UPDATE radio_stations"));
        assert_eq!(arg_int(&script, 1, 5), "3");
    }

    #[test]
    fn a_changed_link_starts_a_new_history() {
        let old = "https://a.example/s?rj-tok=OLD";
        let new = "https://a.example/s?rj-tok=NEW";
        let since = time::format_utc(now() - 30 * HOUR);
        let script = Script::new(vec![
            due_reply(&[due_row(1, new, Some((old, 9, Some(&since))))]),
            written(2),
        ]);
        let fake = Fake::default().on(new, vec![status(500)]);
        let report = run_check(&script, &fake).unwrap();
        assert!(report.disabled.is_empty(), "the old streak must not count");
        assert_eq!(arg_int(&script, 1, 5), "1");
        assert_eq!(arg_text(&script, 1, 4), time::format_utc(now()));
        assert_eq!(
            arg_text(&script, 1, 1),
            new,
            "checked_url follows the stream URL"
        );
    }

    #[test]
    fn if_every_judged_station_fails_the_round_is_inconclusive() {
        let urls = [
            "https://a.example/1",
            "https://b.example/2",
            "https://c.example/3",
        ];
        let since = time::format_utc(now() - 12 * HOUR);
        let rows: Vec<Vec<Json>> = urls
            .iter()
            .zip(1..)
            .map(|(u, id)| due_row(id, u, Some((u, 2, Some(&since)))))
            .collect();
        let script = Script::new(vec![due_reply(&rows), written(4)]);
        let mut fake = Fake::default();
        for u in urls {
            fake = fake.on(u, vec![status(403)]);
        }
        let report = run_check(&script, &fake).unwrap();
        assert!(
            report.disabled.is_empty(),
            "no station may be disabled by this round"
        );
        assert_eq!((report.failed, report.inconclusive), (0, 3));
        assert!(!script.body(1).contains("UPDATE radio_stations"));
        assert_eq!(arg_text(&script, 1, 6), "inconclusive");
        assert_eq!(arg_int(&script, 1, 5), "2", "the streak is left as it was");
    }

    #[test]
    fn the_guard_needs_enough_stations_and_counts_only_judged_ones() {
        let since = time::format_utc(now() - 12 * HOUR);
        // Dua stasiun gagal semua: terlalu sedikit untuk menyimpulkan gangguan.
        let (a, b) = ("https://a.example/1", "https://b.example/2");
        let script = Script::new(vec![
            due_reply(&[
                due_row(1, a, Some((a, 2, Some(&since)))),
                due_row(2, b, Some((b, 2, Some(&since)))),
            ]),
            written(5),
        ]);
        let fake = Fake::default()
            .on(a, vec![status(404)])
            .on(b, vec![status(404)]);
        assert_eq!(run_check(&script, &fake).unwrap().disabled, [1, 2]);

        // Tiga stasiun, satu dilewati (port 1936), satu sehat: yang gagal tetap dihitung.
        let (dead, ok, skipped) = (
            "https://a.example/1",
            "https://b.example/2",
            "https://wz.mari.co.id:1936/x.m3u8",
        );
        let script = Script::new(vec![
            due_reply(&[
                due_row(1, dead, Some((dead, 2, Some(&since)))),
                due_row(2, ok, None),
                due_row(3, skipped, None),
            ]),
            written(5),
        ]);
        let fake = Fake::default()
            .on(dead, vec![status(404)])
            .on(ok, vec![status(200)]);
        let report = run_check(&script, &fake).unwrap();
        assert_eq!(report.disabled, [1]);
        assert_eq!((report.ok, report.failed, report.skipped), (1, 1, 1));
        assert_eq!(fake.seen().len(), 2, "the skipped one is never contacted");
    }

    #[test]
    fn a_skipped_station_is_recorded_so_the_rotation_moves_on_but_never_disabled() {
        let url = "https://wz.mari.co.id:1936/x.m3u8";
        let since = time::format_utc(now() - 40 * HOUR);
        let script = Script::new(vec![
            due_reply(&[due_row(5, url, Some((url, 2, Some(&since))))]),
            written(2),
        ]);
        let report = run_check(&script, &Fake::default()).unwrap();
        assert_eq!((report.skipped, report.failed), (1, 0));
        assert_eq!(arg_text(&script, 1, 6), "skipped");
        assert!(arg_null(&script, 1, 7), "no HTTP status");
        assert!(!script.body(1).contains("UPDATE radio_stations"));
    }

    #[test]
    fn database_errors_surface_without_urls() {
        let script = Script::new(vec![Err("connection refused".to_owned())]);
        let err = run_check(&script, &Fake::default()).unwrap_err();
        assert!(matches!(err, RadioError::Db(_)));

        let url = "https://a.example/s?rj-tok=SECRET";
        let script = Script::new(vec![
            due_reply(&[due_row(1, url, None)]),
            Err("connection refused".to_owned()),
        ]);
        let fake = Fake::default().on(url, vec![status(200)]);
        let err = run_check(&script, &fake).unwrap_err();
        assert!(!err.to_string().contains("SECRET"));
    }

    #[test]
    fn a_row_without_an_id_is_bad_data() {
        let script = Script::new(vec![execute_reply(result(
            &DUE_COLS,
            &[vec![
                Json::Null,
                json!("https://a.example/s"),
                Json::Null,
                Json::Null,
                Json::Null,
                Json::Null,
                Json::Null,
            ]],
        ))]);
        assert_eq!(
            run_check(&script, &Fake::default()).unwrap_err(),
            RadioError::BadData("id")
        );
    }

    #[test]
    fn the_report_line_has_counts_and_ids_but_no_urls() {
        let r = Report {
            checked: 3,
            ok: 1,
            failed: 1,
            skipped: 1,
            inconclusive: 0,
            disabled: vec![4],
        };
        assert_eq!(
            r.to_string(),
            "checked=3 ok=1 failed=1 skipped=1 inconclusive=0 disabled=[4]"
        );
    }

    // ------------------------------------------------------------ dashboard

    const LIST_COLS: [&str; 12] = [
        "id",
        "label",
        "stream_url",
        "is_active",
        "checked_url",
        "last_checked_at",
        "last_ok_at",
        "failing_since",
        "failures",
        "last_result",
        "last_status",
        "disabled_at",
    ];

    #[allow(clippy::too_many_arguments)]
    fn list_row(
        id: i64,
        label: &str,
        url: &str,
        active: i64,
        checked: Option<&str>,
        result: Option<&str>,
        status: Option<i64>,
        failures: Option<i64>,
        disabled: Option<&str>,
    ) -> Vec<Json> {
        let t = |v: Option<&str>| v.map_or(Json::Null, |s| json!(s));
        vec![
            json!(id),
            json!(label),
            json!(url),
            json!(active),
            t(checked),
            t(result.map(|_| "2026-10-24T11:17:00Z")),
            t(result
                .filter(|r| *r == "ok")
                .map(|_| "2026-10-24T11:17:00Z")),
            t(failures.filter(|f| *f > 0).map(|_| "2026-10-24T03:17:00Z")),
            failures.map_or(Json::Null, |f| json!(f)),
            t(result),
            status.map_or(Json::Null, |s| json!(s)),
            t(disabled),
        ]
    }

    fn run_status(rows: &[Vec<Json>]) -> RadioStatus {
        let script = Script::new(vec![execute_reply(result(&LIST_COLS, rows))]);
        futures_executor::block_on(super::status(&client(&script), now())).unwrap()
    }

    #[test]
    fn the_status_covers_every_station_and_never_contains_a_url() {
        let secret = "https://a.example/s?rj-tok=SECRET";
        let rows = [
            list_row(
                1,
                "Healthy",
                secret,
                1,
                Some(secret),
                Some("ok"),
                Some(200),
                Some(0),
                None,
            ),
            list_row(
                2,
                "Failing",
                "https://b.example/s",
                1,
                Some("https://b.example/s"),
                Some("http"),
                Some(403),
                Some(2),
                None,
            ),
            list_row(
                3,
                "Never checked",
                "https://c.example/s",
                1,
                None,
                None,
                None,
                None,
                None,
            ),
            list_row(
                4,
                "Auto off",
                "https://d.example/s",
                0,
                Some("https://d.example/s"),
                Some("http"),
                Some(404),
                Some(3),
                Some("2026-10-24T09:17:00Z"),
            ),
            list_row(
                5,
                "Off by hand",
                "https://e.example/s",
                0,
                None,
                None,
                None,
                None,
                None,
            ),
            list_row(
                6,
                "Odd port",
                "https://wz.mari.co.id:1936/x.m3u8",
                1,
                Some("https://wz.mari.co.id:1936/x.m3u8"),
                Some("skipped"),
                None,
                Some(0),
                None,
            ),
            list_row(
                7,
                "New link",
                "https://f.example/new",
                1,
                Some("https://f.example/old"),
                Some("http"),
                Some(500),
                Some(2),
                None,
            ),
        ];
        let st = run_status(&rows);
        let json = serde_json::to_string(&st).unwrap();
        assert!(
            !json.contains("SECRET") && !json.contains("https://"),
            "{json}"
        );
        assert!(!json.contains("stream_url") && !json.contains("checked_url"));

        let by = |id| st.items.iter().find(|i| i.id == id).unwrap();
        assert_eq!(st.items.len(), 7);
        assert_eq!(
            st.items.iter().map(|i| i.id).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6, 7],
            "order of the query"
        );

        let ok = by(1);
        assert!(ok.is_active && ok.checkable && !ok.link_changed && !ok.disabled_by_check);
        assert_eq!(ok.last_result.as_deref(), Some("ok"));
        assert_eq!(ok.last_status, Some(200));
        assert_eq!(ok.last_ok_at.as_deref(), Some("2026-10-24T11:17:00Z"));
        assert_eq!(ok.failing_since, None);

        let bad = by(2);
        assert_eq!((bad.consecutive_failures, bad.last_status), (2, Some(403)));
        assert_eq!(bad.failing_since.as_deref(), Some("2026-10-24T03:17:00Z"));

        let fresh = by(3);
        assert_eq!(fresh.last_result, None);
        assert_eq!(fresh.last_checked_at, None);
        assert_eq!(fresh.consecutive_failures, 0);

        assert!(by(4).disabled_by_check && !by(4).is_active);
        assert!(
            !by(5).disabled_by_check && !by(5).is_active,
            "off by hand is not the checker's doing"
        );
        assert!(!by(6).checkable, "port 1936 is skipped");
        assert!(by(7).link_changed);
    }

    #[test]
    fn the_status_carries_the_rules_and_the_clock() {
        let st = run_status(&[]);
        assert_eq!(st.now, "2026-10-24T12:00:00Z");
        assert_eq!(st.next_run_at, "2026-10-24T12:17:00Z");
        assert_eq!(st.rules.fail_threshold, FAIL_THRESHOLD);
        assert_eq!(st.rules.min_fail_hours, 6);
        assert_eq!(st.rules.batch, BATCH);
        assert_eq!(st.rules.min_manual_secs, MIN_MANUAL_SECS);
        assert!(st.items.is_empty());
    }

    #[test]
    fn unknown_results_and_unreadable_times_are_dropped_not_forwarded() {
        let mut row = list_row(
            1,
            "Odd",
            "https://a.example/s",
            1,
            Some("https://a.example/s"),
            Some("weird"),
            Some(999_999),
            Some(-4),
            None,
        );
        row[5] = json!("yesterday");
        let st = run_status(&[row]);
        let item = &st.items[0];
        assert_eq!(item.last_result, None, "only known result codes pass");
        assert_eq!(item.last_status, None);
        assert_eq!(item.last_checked_at, None);
        assert_eq!(item.consecutive_failures, 0);
    }

    #[test]
    fn a_station_row_without_an_id_makes_the_status_bad_data() {
        let mut row = list_row(
            1,
            "X",
            "https://a.example/s",
            1,
            None,
            None,
            None,
            None,
            None,
        );
        row[0] = Json::Null;
        let script = Script::new(vec![execute_reply(result(&LIST_COLS, &[row]))]);
        let err = futures_executor::block_on(super::status(&client(&script), now())).unwrap_err();
        assert_eq!(err, RadioError::BadData("id"));
    }

    #[test]
    fn the_next_cron_run_is_minute_17_of_this_or_the_next_hour() {
        let h = time::days_from_civil(2026, 10, 24) * 86_400 + 12 * 3600;
        assert_eq!(next_run(h), h + 17 * 60);
        assert_eq!(next_run(h + 16 * 60 + 59), h + 17 * 60);
        assert_eq!(
            next_run(h + 17 * 60),
            h + 3600 + 17 * 60,
            "at the minute itself: the next one"
        );
        assert_eq!(next_run(h + 59 * 60), h + 3600 + 17 * 60);
        assert_eq!(CRON, format!("{CRON_MINUTE} * * * *"));
    }

    #[test]
    fn result_codes_round_trip() {
        for k in [
            Kind::Ok,
            Kind::Http,
            Kind::Network,
            Kind::Timeout,
            Kind::Redirect,
            Kind::Skipped,
            Kind::Inconclusive,
        ] {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
        assert_eq!(Kind::parse("OK"), None);
        assert_eq!(Kind::parse(""), None);
    }

    fn last_reply(at: Option<&str>) -> Result<turso_http::HttpResponse, String> {
        execute_reply(result(
            &["at"],
            &[vec![at.map_or(Json::Null, |a| json!(a))]],
        ))
    }

    fn manual(script: &Script, fake: &Fake) -> Result<Report, RadioError> {
        futures_executor::block_on(check_now(&client(script), fake, now()))
    }

    #[test]
    fn a_manual_check_right_after_another_is_refused_with_the_wait_time() {
        let recent = time::format_utc(now() - 45);
        let script = Script::new(vec![last_reply(Some(&recent))]);
        let fake = Fake::default();
        assert_eq!(
            manual(&script, &fake).unwrap_err(),
            RadioError::TooSoon {
                retry_after_secs: 15
            }
        );
        assert_eq!(script.requests(), 1, "nothing else is touched");
        assert!(fake.seen().is_empty());
    }

    #[test]
    fn a_manual_check_runs_when_the_last_one_is_old_or_missing() {
        let url = "https://a.example/s";
        for last in [None, Some(time::format_utc(now() - 60))] {
            let script = Script::new(vec![
                last_reply(last.as_deref()),
                due_reply(&[due_row(1, url, None)]),
                written(2),
            ]);
            let fake = Fake::default().on(url, vec![status(200)]);
            let report = manual(&script, &fake).unwrap();
            assert_eq!((report.checked, report.ok), (1, 1), "{last:?}");
        }
    }

    #[test]
    fn a_clock_running_backwards_never_locks_the_button_for_long() {
        let future = time::format_utc(now() + 10 * HOUR);
        let script = Script::new(vec![last_reply(Some(&future))]);
        assert_eq!(
            manual(&script, &Fake::default()).unwrap_err(),
            RadioError::TooSoon {
                retry_after_secs: 60_u64
            }
        );
    }

    #[test]
    fn the_cron_expression_is_registered_in_wrangler_toml() {
        let toml = include_str!("../wrangler.toml");
        let crons = toml
            .lines()
            .find(|l| l.trim_start().starts_with("crons"))
            .expect("crons line");
        assert!(crons.contains(&format!("\"{CRON}\"")), "{crons}");
        assert!(
            crons.contains("\"0 */6 * * *\""),
            "the journal cron must stay: {crons}"
        );
    }

    #[test]
    fn the_queries_use_bound_parameters_and_an_explicit_order() {
        assert!(DUE.contains("ORDER BY"));
        assert!(DUE.contains("LIMIT ?1"));
        for sql in [UPSERT, DISABLE] {
            assert!(sql.contains("?1"));
        }
        assert!(DISABLE.contains("stream_url = ?2"));
    }
}
