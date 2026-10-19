//! GitHub Journal (D5): mengambil statistik dari GitHub GraphQL, menyimpannya
//! sebagai snapshot di `journal_snapshots`, dan menyajikannya.
//!
//! Pengganti Worker lama (`github-journal-stats`, JavaScript). Perbedaan yang
//! disengaja:
//!
//! - **Cron Trigger**, bukan "ambil saat ada pengunjung + cache edge". Halaman
//!   hanya membaca database.
//! - **Akun, repositori sematan, dan stack dari tabel** (`journal_profile`,
//!   `journal_pinned_repos`, `journal_stack`), bukan konstanta, supaya bisa
//!   diubah dari dashboard admin (D6).
//! - **Data, bukan HTML.** Kalimat "fokus bulan ini" tidak lagi dibuat server
//!   (dulu menempelkan nama repo mentah ke string HTML); server hanya
//!   mengirim nama repo dan bahasanya.
//! - **Nilai dari database tidak dipercaya.** Nama repo masuk ke query sebagai
//!   *variabel* GraphQL (tidak pernah disisipkan ke teks query), dan setiap nilai
//!   diperiksa ulang saat dibaca.
//! - Query lebih ringan: commit per repositori hanya diminta untuk bulan ini
//!   (yang dipakai halaman), bukan untuk 12 bulan lalu disaring belakangan.
//!   Penting karena plan gratis Workers membatasi CPU 10 ms per eksekusi.
//! - Periode "tahun lalu" berakhir tepat sebelum periode sekarang mulai (kode
//!   lama kehilangan hari terakhirnya).
//!
//! Semua logika di sini murni terhadap [`Client`] dan [`GraphQl`], jadi dites
//! native dengan transport palsu.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;

use api_types::journal::{
    self, limits, Chronicle, Data, Day, Focus, Hero, Language, Refreshed, Repo, Snapshot, Source,
};
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::Deserialize;
use serde_json::json;
use turso_http::{Client, Error, HttpResponse, Stmt, Transport};

use crate::config::Config;
use crate::content::{opt_text, text};
use crate::github::GithubClient;
use crate::time;

const PROFILE: &str = include_str!("queries/journal_profile.sql");
const PINNED: &str = include_str!("queries/journal_pinned.sql");
const STACK: &str = include_str!("queries/journal_stack.sql");
const LATEST: &str = include_str!("queries/journal_latest.sql");
const LATEST_TIME: &str = include_str!("queries/journal_latest_time.sql");
const INSERT: &str = include_str!("queries/journal_insert.sql");
const PRUNE: &str = include_str!("queries/journal_prune.sql");

/// Snapshot GitHub yang disimpan (sisanya dihapus tiap refresh): 28 kali
/// refresh 6 jam = 7 hari.
pub const KEEP_SNAPSHOTS: u32 = 28;
/// Jarak minimum antar refresh manual dari endpoint admin.
pub const MIN_REFRESH_SECS: i64 = 60;

// -------------------------------------------------------------------- error

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// Database tidak terjangkau atau menolak query.
    Db(Error),
    /// Isi database tidak berbentuk yang diharapkan.
    BadData(&'static str),
    /// Pengaturan Worker belum lengkap (nama bagian yang kurang).
    NotConfigured(&'static str),
    /// GitHub gagal atau menjawab hal yang tidak dimengerti (hanya untuk log).
    Upstream(String),
    /// Refresh manual terlalu cepat setelah yang sebelumnya.
    TooSoon { retry_after_secs: u64 },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::Db(e) => write!(f, "{e}"),
            JournalError::BadData(what) => write!(f, "unexpected data: {what}"),
            JournalError::NotConfigured(what) => write!(f, "not configured: {what}"),
            JournalError::Upstream(why) => write!(f, "GitHub: {why}"),
            JournalError::TooSoon { retry_after_secs } => {
                write!(f, "refreshed too recently, retry in {retry_after_secs}s")
            }
        }
    }
}

impl std::error::Error for JournalError {}

impl From<Error> for JournalError {
    fn from(e: Error) -> Self {
        JournalError::Db(e)
    }
}

// ----------------------------------------------------------------- pengaturan

/// Repositori sematan dari `journal_pinned_repos`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedRepo {
    pub name: String,
    pub topic: Option<String>,
}

/// Pengaturan jurnal yang sudah diperiksa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalConfig {
    pub username: String,
    pub pinned: Vec<PinnedRepo>,
    pub stack: Vec<String>,
}

/// Nama akun GitHub: huruf, angka, `-`, 1..=39 karakter.
pub fn is_github_username(s: &str) -> bool {
    (1..=39).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Membaca pengaturan dari database. Nilai buruk pada daftar (repo, stack)
/// dibuang satu per satu; akun yang buruk atau hilang menggagalkan semuanya
/// (tanpa akun tidak ada yang bisa diambil).
pub async fn load_config<T: Transport>(client: &Client<T>) -> Result<JournalConfig, JournalError> {
    let res = client.execute(Stmt::new(PROFILE)).await?;
    let username =
        text(&res, 0, "github_username").map_err(|_| JournalError::BadData("profile"))?;
    if !is_github_username(&username) {
        return Err(JournalError::BadData("github_username"));
    }

    let res = client.execute(Stmt::new(PINNED)).await?;
    let mut pinned = Vec::new();
    for row in 0..res.rows.len() {
        let name = text(&res, row, "repo_name").map_err(|_| JournalError::BadData("repo_name"))?;
        let topic = opt_text(&res, row, "topic").map_err(|_| JournalError::BadData("topic"))?;
        if !journal::is_repo_name(&name) || pinned.iter().any(|p: &PinnedRepo| p.name == name) {
            continue;
        }
        let topic = topic
            .map(|t| journal::clean_text(&t, limits::MAX_TOPIC_CHARS))
            .filter(|t| !t.is_empty());
        pinned.push(PinnedRepo { name, topic });
        if pinned.len() == limits::MAX_PINNED {
            break;
        }
    }

    let res = client.execute(Stmt::new(STACK)).await?;
    let mut stack: Vec<String> = Vec::new();
    for row in 0..res.rows.len() {
        let name = text(&res, row, "name").map_err(|_| JournalError::BadData("stack"))?;
        let name = journal::clean_text(&name, limits::MAX_NAME_CHARS);
        if !name.is_empty() && !stack.contains(&name) {
            stack.push(name);
        }
        if stack.len() == limits::MAX_STACK {
            break;
        }
    }

    Ok(JournalConfig {
        username,
        pinned,
        stack,
    })
}

// ------------------------------------------------------------- query GitHub

/// Awal dan akhir periode yang diminta (semua UTC, `...T00:00:00Z` kecuali
/// akhir periode sekarang yang berupa saat ini).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Windows {
    /// 12 bulan penuh sebelum bulan ini, ditambah bulan ini sampai sekarang.
    pub current_start: String,
    pub current_end: String,
    /// 12 bulan sebelum itu; berakhir sedetik sebelum `current_start`.
    pub last_start: String,
    pub last_end: String,
    pub month_start: String,
}

fn month_start_secs(year: i64, month: u32) -> i64 {
    time::days_from_civil(year, month, 1) * 86_400
}

/// Bulan ke-`index` (`tahun * 12 + bulan - 1`, 0-based) -> awal bulan itu.
fn month_index_start(index: i64) -> i64 {
    let year = index.div_euclid(12);
    let month = u32::try_from(index.rem_euclid(12)).unwrap_or(0) + 1;
    month_start_secs(year, month)
}

pub fn windows(now_secs: i64) -> Windows {
    let (year, month, _) = time::civil_from_days(now_secs.div_euclid(86_400));
    // Awal bulan depan, setahun lalu: Nov tahun lalu kalau sekarang Oktober.
    let next_month = year * 12 + i64::from(month);
    let current_start = month_index_start(next_month - 12);
    let last_start = month_index_start(next_month - 24);
    Windows {
        current_start: time::format_utc(current_start),
        current_end: time::format_utc(now_secs),
        last_start: time::format_utc(last_start),
        last_end: time::format_utc(current_start - 1),
        month_start: time::format_utc(month_start_secs(year, month)),
    }
}

/// Teks query untuk `pinned` repositori. Hanya bergantung pada jumlahnya:
/// nama repositori dikirim sebagai variabel `$repo<i>`, tidak pernah disisipkan.
pub fn build_query(pinned: usize) -> String {
    let mut vars = String::new();
    let mut aliases = String::new();
    for i in 0..pinned {
        vars.push_str(&format!(", $repo{i}: String!"));
        aliases.push_str(&format!(
            "\n    repo{i}: repository(owner: $username, name: $repo{i}) {{ name description stargazerCount primaryLanguage {{ name }} }}"
        ));
    }
    format!(
        r"query($username: String!, $currentYearStart: DateTime!, $currentYearEnd: DateTime!, $lastYearStart: DateTime!, $lastYearEnd: DateTime!, $monthStart: DateTime!{vars}) {{
  user(login: $username) {{
    followers {{ totalCount }}
    repositories(first: 100, ownerAffiliations: OWNER, isFork: false, orderBy: {{field: STARGAZERS, direction: DESC}}) {{
      totalCount
      nodes {{
        stargazerCount
        languages(first: 10, orderBy: {{field: SIZE, direction: DESC}}) {{ edges {{ size node {{ name color }} }} }}
      }}
    }}
    currentYear: contributionsCollection(from: $currentYearStart, to: $currentYearEnd) {{
      contributionCalendar {{ totalContributions weeks {{ contributionDays {{ contributionCount date weekday }} }} }}
    }}
    thisMonth: contributionsCollection(from: $monthStart, to: $currentYearEnd) {{
      commitContributionsByRepository(maxRepositories: 100) {{
        repository {{ name primaryLanguage {{ name }} }}
        contributions(first: 100) {{ nodes {{ commitCount }} }}
      }}
    }}
    lastYear: contributionsCollection(from: $lastYearStart, to: $lastYearEnd) {{
      contributionCalendar {{ totalContributions }}
    }}
  }}{aliases}
}}
"
    )
}

/// Isi `POST https://api.github.com/graphql`.
pub fn request_body(cfg: &JournalConfig, now_secs: i64) -> String {
    let w = windows(now_secs);
    let mut vars = serde_json::Map::new();
    vars.insert("username".into(), json!(cfg.username));
    vars.insert("currentYearStart".into(), json!(w.current_start));
    vars.insert("currentYearEnd".into(), json!(w.current_end));
    vars.insert("lastYearStart".into(), json!(w.last_start));
    vars.insert("lastYearEnd".into(), json!(w.last_end));
    vars.insert("monthStart".into(), json!(w.month_start));
    for (i, p) in cfg.pinned.iter().enumerate() {
        vars.insert(format!("repo{i}"), json!(p.name));
    }
    json!({ "query": build_query(cfg.pinned.len()), "variables": vars }).to_string()
}

// ------------------------------------------------------------ respons mentah
//
// Hanya kolom yang dipakai (serde mengabaikan sisanya tanpa menyalinnya):
// batas CPU 10 ms di plan gratis membuat parse yang hemat itu penting.

#[derive(Deserialize, Default)]
#[serde(default)]
struct Response {
    data: Option<RawData>,
    errors: Vec<GqlError>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GqlError {
    message: String,
}

/// `data`: `user` ditambah satu alias `repo<i>` per repositori sematan.
#[derive(Default)]
pub struct RawData {
    user: Option<User>,
    repos: HashMap<usize, RawRepo>,
}

impl<'de> Deserialize<'de> for RawData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RawData;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the GraphQL data object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawData, A::Error> {
                let mut out = RawData::default();
                while let Some(key) = map.next_key::<String>()? {
                    if key == "user" {
                        out.user = map.next_value::<Option<User>>()?;
                    } else if let Some(i) = key
                        .strip_prefix("repo")
                        .and_then(|n| n.parse::<usize>().ok())
                    {
                        if let Some(repo) = map.next_value::<Option<RawRepo>>()? {
                            out.repos.insert(i, repo);
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_map(V)
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct User {
    followers: Option<Count>,
    repositories: Option<RepoList>,
    current_year: Option<Collection>,
    last_year: Option<Collection>,
    this_month: Option<Collection>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Count {
    total_count: u64,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RepoList {
    total_count: u64,
    nodes: Vec<Option<RepoNode>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RepoNode {
    stargazer_count: u64,
    languages: Option<LangEdges>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LangEdges {
    edges: Vec<Option<LangEdge>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LangEdge {
    size: u64,
    node: Option<LangNode>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LangNode {
    name: Option<String>,
    color: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Collection {
    contribution_calendar: Option<Calendar>,
    commit_contributions_by_repository: Vec<Option<CommitRepo>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Calendar {
    total_contributions: u64,
    weeks: Vec<Option<Week>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Week {
    contribution_days: Vec<Option<RawDay>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RawDay {
    contribution_count: u64,
    date: String,
    weekday: u64,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CommitRepo {
    repository: Option<CommitRepoInfo>,
    contributions: Option<ContribNodes>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CommitRepoInfo {
    name: Option<String>,
    primary_language: Option<LangNode>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ContribNodes {
    nodes: Vec<Option<ContribNode>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ContribNode {
    commit_count: u64,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RawRepo {
    name: Option<String>,
    description: Option<String>,
    stargazer_count: u64,
    primary_language: Option<LangNode>,
}

// ---------------------------------------------------------------- transformasi

fn sat32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn day_count(n: u64) -> u32 {
    sat32(n.min(u64::from(limits::MAX_DAY_COUNT)))
}

fn lang_name(node: Option<&LangNode>) -> Option<String> {
    let name = journal::clean_text(node?.name.as_deref()?, limits::MAX_NAME_CHARS);
    (!name.is_empty()).then_some(name)
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn first_weekday(counts: &[u64; 7]) -> Option<u8> {
    let max = *counts.iter().max()?;
    if max == 0 {
        return None;
    }
    counts
        .iter()
        .position(|c| *c == max)
        .and_then(|i| u8::try_from(i).ok())
}

/// Respons GraphQL -> payload jurnal. Hasilnya sudah lolos [`Data::validate`].
pub fn transform(raw: &RawData, cfg: &JournalConfig, now_secs: i64) -> Result<Data, &'static str> {
    let user = raw.user.as_ref().ok_or("user not found")?;
    let today_days = now_secs.div_euclid(86_400);
    let today = time::format_date(today_days);

    // Hero
    let repos = user.repositories.as_ref();
    let nodes = repos.map(|r| r.nodes.as_slice()).unwrap_or_default();
    let hero = Hero {
        total_repos: sat32(repos.map_or(0, |r| r.total_count)),
        total_stars: sat32(
            nodes
                .iter()
                .flatten()
                .fold(0u64, |s, n| s.saturating_add(n.stargazer_count)),
        ),
        total_followers: sat32(user.followers.as_ref().map_or(0, |f| f.total_count)),
    };

    // Repositori sematan; yang tidak ada (privat, diganti nama, dihapus) dilewati.
    let mut pinned = Vec::new();
    for (i, cfgrepo) in cfg.pinned.iter().enumerate() {
        let Some(r) = raw.repos.get(&i) else { continue };
        let title = r
            .name
            .as_deref()
            .filter(|n| journal::is_repo_name(n))
            .unwrap_or(&cfgrepo.name)
            .to_owned();
        pinned.push(Repo {
            title,
            description: journal::clean_text(
                r.description.as_deref().unwrap_or_default(),
                limits::MAX_DESCRIPTION_CHARS,
            ),
            stars: sat32(r.stargazer_count),
            topic: cfgrepo.topic.clone(),
            language: lang_name(r.primary_language.as_ref()),
        });
    }

    // Bahasa: ukuran kode dijumlah di semua repositori milik sendiri.
    let mut langs: Vec<(String, u64, Option<String>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut total_size = 0u64;
    for node in nodes.iter().flatten() {
        for edge in node.languages.iter().flat_map(|l| l.edges.iter().flatten()) {
            let Some(name) = lang_name(edge.node.as_ref()) else {
                continue;
            };
            let color = edge
                .node
                .as_ref()
                .and_then(|n| n.color.clone())
                .filter(|c| journal::is_hex_color(c));
            total_size = total_size.saturating_add(edge.size);
            match index.get(&name) {
                Some(&i) => {
                    langs[i].1 = langs[i].1.saturating_add(edge.size);
                    if langs[i].2.is_none() {
                        langs[i].2 = color;
                    }
                }
                None => {
                    index.insert(name.clone(), langs.len());
                    langs.push((name, edge.size, color));
                }
            }
        }
    }
    let mut languages: Vec<Language> = langs
        .into_iter()
        .map(|(name, size, color)| Language {
            name,
            percent: if total_size > 0 {
                #[allow(clippy::cast_precision_loss)]
                let ratio = size as f64 / total_size as f64;
                round1(ratio * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            },
            color,
        })
        .collect();
    // Sort stabil: seri tetap dalam urutan pertama kali muncul.
    languages.sort_by(|a, b| b.percent.total_cmp(&a.percent));
    languages.truncate(5);

    // Kontribusi
    let cal_total = user
        .current_year
        .as_ref()
        .and_then(|c| c.contribution_calendar.as_ref())
        .map_or(0, |c| c.total_contributions);
    let last_total = user
        .last_year
        .as_ref()
        .and_then(|c| c.contribution_calendar.as_ref())
        .map_or(0, |c| c.total_contributions);
    let growth_percent = if last_total > 0 {
        #[allow(clippy::cast_precision_loss)]
        let g = (cal_total as f64 - last_total as f64) / last_total as f64 * 100.0;
        if g.is_finite() {
            round1(g).clamp(-1_000_000.0, 1_000_000.0)
        } else {
            0.0
        }
    } else {
        0.0
    };

    // Satu titik per hari, urut menaik; (tanggal, jumlah, hari-dalam-seminggu).
    let mut days: Vec<(String, u32, usize)> = user
        .current_year
        .iter()
        .filter_map(|c| c.contribution_calendar.as_ref())
        .flat_map(|c| c.weeks.iter().flatten())
        .flat_map(|w| w.contribution_days.iter().flatten())
        .filter(|d| journal::is_iso_date(&d.date))
        .map(|d| {
            (
                d.date.clone(),
                day_count(d.contribution_count),
                usize::try_from(d.weekday.min(6)).unwrap_or(0),
            )
        })
        .collect();
    days.sort_by(|a, b| a.0.cmp(&b.0));
    days.dedup_by(|a, b| a.0 == b.0);
    if days.len() > limits::MAX_TIMELINE {
        days.drain(..days.len() - limits::MAX_TIMELINE);
    }

    // Rangkaian hari berturut-turut sampai hari ini; hari ini yang masih
    // kosong tidak memutusnya.
    let today_index = days
        .iter()
        .position(|d| d.0 == today)
        .or_else(|| days.len().checked_sub(1));
    let mut current_streak = 0u32;
    if let Some(start) = today_index {
        for (idx, d) in days[..=start].iter().enumerate().rev() {
            if d.1 > 0 {
                current_streak += 1;
            } else if idx != start {
                break;
            }
        }
    }

    // Hari tersibuk: terbanyak dulu; seri -> yang lebih awal (sort stabil).
    let mut busiest: Vec<&(String, u32, usize)> = days.iter().filter(|d| d.1 > 0).collect();
    busiest.sort_by_key(|d| std::cmp::Reverse(d.1));
    let top_activities = busiest
        .iter()
        .take(3)
        .map(|d| Day {
            date: d.0.clone(),
            count: d.1,
        })
        .collect();

    // Hari dalam seminggu paling aktif bulan ini.
    let month_prefix = &today[..7];
    let mut by_weekday = [0u64; 7];
    for d in days.iter().filter(|d| d.0.starts_with(month_prefix)) {
        by_weekday[d.2] += u64::from(d.1);
    }
    let most_productive_weekday =
        first_weekday(&by_weekday).unwrap_or_else(|| time::weekday_from_days(today_days));

    // Fokus bulan ini: repositori dengan commit terbanyak.
    let mut focus: Vec<(String, u64, Option<String>)> = Vec::new();
    for rc in user
        .this_month
        .iter()
        .flat_map(|c| c.commit_contributions_by_repository.iter().flatten())
    {
        let Some(info) = rc.repository.as_ref() else {
            continue;
        };
        let Some(name) = info.name.as_deref().filter(|n| journal::is_repo_name(n)) else {
            continue;
        };
        let commits = rc
            .contributions
            .iter()
            .flat_map(|c| c.nodes.iter().flatten())
            .fold(0u64, |s, n| s.saturating_add(n.commit_count));
        let language = lang_name(info.primary_language.as_ref());
        match focus.iter_mut().find(|f| f.0 == name) {
            Some(f) => f.1 = f.1.saturating_add(commits),
            None => focus.push((name.to_owned(), commits, language)),
        }
    }
    focus.sort_by_key(|f| std::cmp::Reverse(f.1));
    let monthly_focus = focus
        .into_iter()
        .find(|f| f.1 > 0)
        .map(|(repo, _, language)| Focus { repo, language });

    let data = Data {
        hero,
        pinned,
        chronicle: Chronicle {
            total_contribution_volume: sat32(cal_total),
            growth_percent,
            current_streak,
            top_activities,
            monthly_focus,
            most_productive_weekday,
            languages,
            stack: cfg.stack.clone(),
            timeline: days
                .into_iter()
                .map(|(date, count, _)| Day { date, count })
                .collect(),
        },
    };
    data.validate()?;
    Ok(data)
}

/// Respons HTTP GitHub -> payload.
pub fn parse_and_transform(
    resp: &HttpResponse,
    cfg: &JournalConfig,
    now_secs: i64,
) -> Result<Data, JournalError> {
    if !(200..300).contains(&resp.status) {
        return Err(JournalError::Upstream(format!("HTTP {}", resp.status)));
    }
    let parsed: Response = serde_json::from_str(&resp.body)
        .map_err(|e| JournalError::Upstream(format!("unreadable response: {e}")))?;
    if let Some(first) = parsed.errors.first() {
        let msg: String = first.message.chars().take(200).collect();
        return Err(JournalError::Upstream(format!("GraphQL error: {msg}")));
    }
    let raw = parsed
        .data
        .ok_or_else(|| JournalError::Upstream("response has no data".into()))?;
    transform(&raw, cfg, now_secs).map_err(|why| JournalError::Upstream(why.to_string()))
}

// ---------------------------------------------------------------------- GitHub

/// Pengirim `POST` ke GitHub GraphQL. Diimplementasikan oleh `fetch` di Worker
/// (`github.rs`) dan oleh transport palsu di test.
pub trait GraphQl {
    /// `body` adalah JSON `{query, variables}`. `Err` = jaringan gagal (tanpa
    /// rahasia).
    fn post(&self, body: String) -> impl Future<Output = Result<HttpResponse, String>>;
}

// ------------------------------------------------------------- simpan dan baca

/// Menyimpan snapshot baru lalu memangkas yang lama, dalam satu transaksi.
pub async fn store<T: Transport>(
    client: &Client<T>,
    data: &Data,
    now_secs: i64,
) -> Result<Refreshed, JournalError> {
    let payload =
        serde_json::to_string(data).map_err(|_| JournalError::BadData("payload encode"))?;
    if payload.len() > 1_048_576 {
        return Err(JournalError::BadData("payload too large"));
    }
    let fetched_at = time::format_utc(now_secs);
    client
        .transaction(vec![
            Stmt::new(INSERT)
                .bind(payload)
                .bind(fetched_at.as_str())
                .no_rows(),
            Stmt::new(PRUNE).bind(KEEP_SNAPSHOTS).no_rows(),
        ])
        .await?;
    Ok(Refreshed { fetched_at })
}

/// Snapshot terbaru, atau `None` jika belum pernah ada. Payload diperiksa
/// ulang (bisa diubah lewat database); yang tidak lolos adalah `BadData`,
/// bukan dikirim ke klien.
pub async fn latest<T: Transport>(client: &Client<T>) -> Result<Option<Snapshot>, JournalError> {
    let res = client.execute(Stmt::new(LATEST)).await?;
    if res.rows.is_empty() {
        return Ok(None);
    }
    let payload = text(&res, 0, "payload").map_err(|_| JournalError::BadData("payload"))?;
    let source = text(&res, 0, "source").map_err(|_| JournalError::BadData("source"))?;
    let fetched_at =
        text(&res, 0, "fetched_at").map_err(|_| JournalError::BadData("fetched_at"))?;
    let source = Source::parse(&source).ok_or(JournalError::BadData("source"))?;
    if time::parse_utc(&fetched_at).is_none() {
        return Err(JournalError::BadData("fetched_at"));
    }
    let data: Data =
        serde_json::from_str(&payload).map_err(|_| JournalError::BadData("payload"))?;
    data.validate()
        .map_err(|_| JournalError::BadData("payload"))?;
    Ok(Some(Snapshot {
        fetched_at,
        source,
        data,
    }))
}

async fn latest_fetched_at<T: Transport>(client: &Client<T>) -> Result<Option<i64>, JournalError> {
    let res = client.execute(Stmt::new(LATEST_TIME)).await?;
    if res.rows.is_empty() {
        return Ok(None);
    }
    let at = text(&res, 0, "fetched_at").map_err(|_| JournalError::BadData("fetched_at"))?;
    Ok(time::parse_utc(&at))
}

// ---------------------------------------------------------------------- refresh

/// Ambil dari GitHub sekarang dan simpan. Dipakai cron.
pub async fn refresh<T: Transport, G: GraphQl>(
    db: &Client<T>,
    github: &G,
    now_secs: i64,
) -> Result<Refreshed, JournalError> {
    let cfg = load_config(db).await?;
    let resp = github
        .post(request_body(&cfg, now_secs))
        .await
        .map_err(|e| JournalError::Upstream(format!("request failed: {e}")))?;
    let data = parse_and_transform(&resp, &cfg, now_secs)?;
    store(db, &data, now_secs).await
}

/// Seperti [`refresh`], tetapi menolak kalau snapshot terakhir lebih baru dari
/// [`MIN_REFRESH_SECS`]. Dipakai endpoint admin.
pub async fn refresh_if_due<T: Transport, G: GraphQl>(
    db: &Client<T>,
    github: &G,
    now_secs: i64,
) -> Result<Refreshed, JournalError> {
    if let Some(last) = latest_fetched_at(db).await? {
        let wait = last + MIN_REFRESH_SECS - now_secs;
        if wait > 0 {
            return Err(JournalError::TooSoon {
                retry_after_secs: wait.unsigned_abs(),
            });
        }
    }
    refresh(db, github, now_secs).await
}

/// Jalur cron: bangun klien dari konfigurasi lalu [`refresh`].
pub async fn run(config: &Config, now_secs: i64) -> Result<Refreshed, JournalError> {
    let Some(cfg) = &config.db else {
        return Err(JournalError::NotConfigured("database"));
    };
    if config.github_token.expose().is_empty() {
        return Err(JournalError::NotConfigured("GITHUB_TOKEN"));
    }
    let db = crate::db::client(cfg)?;
    refresh(
        &db,
        &GithubClient::new(config.github_token.expose()),
        now_secs,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value as Json};
    use std::cell::RefCell;

    /// 2026-10-20 12:00:00 UTC (hari Selasa).
    fn now() -> i64 {
        time::days_from_civil(2026, 10, 20) * 86_400 + 12 * 3600
    }

    fn cfg() -> JournalConfig {
        JournalConfig {
            username: "moch".into(),
            pinned: vec![
                PinnedRepo {
                    name: "flexinix".into(),
                    topic: Some("Nixos Configuration Flakes".into()),
                },
                PinnedRepo {
                    name: "ghost-repo".into(),
                    topic: None,
                },
                PinnedRepo {
                    name: "racooonfig".into(),
                    topic: None,
                },
            ],
            stack: vec!["NixOS".into(), "Rust".into()],
        }
    }

    fn day(date: &str, count: u64, weekday: u64) -> Json {
        json!({"date": date, "contributionCount": count, "weekday": weekday})
    }

    fn github_response() -> Json {
        json!({"data": {
            "user": {
                "followers": {"totalCount": 7},
                "repositories": {"totalCount": 3, "nodes": [
                    {"stargazerCount": 5, "languages": {"edges": [
                        {"size": 600, "node": {"name": "Nix", "color": "#7e7eff"}},
                        {"size": 300, "node": {"name": "Rust", "color": "#dea584"}},
                        {"size": 100, "node": {"name": "C++", "color": "red;background:url(x)"}}
                    ]}},
                    {"stargazerCount": 3, "languages": {"edges": [
                        {"size": 100, "node": {"name": "Nix", "color": null}}
                    ]}},
                    {"stargazerCount": 2, "languages": null},
                    null
                ]},
                "currentYear": {"contributionCalendar": {"totalContributions": 120, "weeks": [
                    {"contributionDays": [
                        day("2026-09-28", 4, 1), day("2026-09-29", 0, 2), day("2026-09-30", 9, 3),
                        day("2026-10-05", 14, 1), day("2026-10-06", 7, 2), day("2026-10-07", 7, 3)
                    ]},
                    {"contributionDays": [
                        day("2026-10-17", 0, 6), day("2026-10-18", 2, 0),
                        day("2026-10-19", 3, 1), day("2026-10-20", 0, 2)
                    ]}
                ]}},
                "lastYear": {"contributionCalendar": {"totalContributions": 100}},
                "thisMonth": {"commitContributionsByRepository": [
                    {"repository": {"name": "flexinix", "primaryLanguage": {"name": "Nix"}},
                     "contributions": {"nodes": [{"commitCount": 5}, {"commitCount": 4}]}},
                    {"repository": {"name": "racooonfig", "primaryLanguage": null},
                     "contributions": {"nodes": [{"commitCount": 12}]}},
                    {"repository": {"name": "<b>x</b>", "primaryLanguage": {"name": "Evil"}},
                     "contributions": {"nodes": [{"commitCount": 100}]}}
                ]}
            },
            "repo0": {"name": "flexinix", "description": "  Nix\nflakes <b>x</b> ",
                      "stargazerCount": 3, "primaryLanguage": {"name": "Nix"}},
            "repo1": null,
            "repo2": {"name": "racooonfig", "description": null,
                      "stargazerCount": 1, "primaryLanguage": null}
        }})
    }

    fn parse(body: &Json) -> Result<Data, JournalError> {
        parse_and_transform(
            &HttpResponse {
                status: 200,
                body: body.to_string(),
            },
            &cfg(),
            now(),
        )
    }

    // ------------------------------------------------------------ jendela waktu

    #[test]
    fn windows_cover_twelve_months_and_the_year_before() {
        let w = windows(now());
        assert_eq!(w.current_start, "2025-11-01T00:00:00Z");
        assert_eq!(w.current_end, "2026-10-20T12:00:00Z");
        assert_eq!(w.last_start, "2024-11-01T00:00:00Z");
        assert_eq!(w.last_end, "2025-10-31T23:59:59Z");
        assert_eq!(w.month_start, "2026-10-01T00:00:00Z");
    }

    #[test]
    fn windows_roll_over_the_year_boundary() {
        // Desember: periode sekarang mulai 1 Januari tahun ini.
        let dec = time::days_from_civil(2026, 12, 31) * 86_400 + 86_399;
        let w = windows(dec);
        assert_eq!(w.current_start, "2026-01-01T00:00:00Z");
        assert_eq!(w.last_start, "2025-01-01T00:00:00Z");
        assert_eq!(w.last_end, "2025-12-31T23:59:59Z");
        assert_eq!(w.month_start, "2026-12-01T00:00:00Z");
        // Januari: periode sekarang mulai 1 Februari tahun lalu.
        let jan = time::days_from_civil(2027, 1, 1) * 86_400;
        let w = windows(jan);
        assert_eq!(w.current_start, "2026-02-01T00:00:00Z");
        assert_eq!(w.last_end, "2026-01-31T23:59:59Z");
    }

    // ------------------------------------------------------------- query GitHub

    #[test]
    fn repository_names_are_variables_never_part_of_the_query_text() {
        let q = build_query(2);
        assert!(q.contains("$repo0: String!") && q.contains("$repo1: String!"));
        assert!(q.contains("repo1: repository(owner: $username, name: $repo1)"));
        assert!(!q.contains("$repo2"));
        assert_eq!(q.matches('{').count(), q.matches('}').count());

        let body: Json = serde_json::from_str(&request_body(&cfg(), now())).unwrap();
        let q = body["query"].as_str().unwrap();
        for name in ["flexinix", "ghost-repo", "racooonfig", "moch"] {
            assert!(!q.contains(name), "{name} must not be interpolated");
        }
        assert_eq!(body["variables"]["repo0"], "flexinix");
        assert_eq!(body["variables"]["repo2"], "racooonfig");
        assert_eq!(body["variables"]["username"], "moch");
        assert_eq!(body["variables"]["lastYearEnd"], "2025-10-31T23:59:59Z");
    }

    #[test]
    fn a_query_without_pinned_repositories_is_still_well_formed() {
        let q = build_query(0);
        assert!(q.starts_with("query($username: String!"));
        assert!(!q.contains("repo0"));
        assert_eq!(q.matches('{').count(), q.matches('}').count());
    }

    // ---------------------------------------------------------------- transform

    #[test]
    fn a_github_response_becomes_a_valid_payload() {
        let d = parse(&github_response()).unwrap();
        assert_eq!(d.validate(), Ok(()));

        assert_eq!(
            d.hero,
            Hero {
                total_repos: 3,
                total_stars: 10,
                total_followers: 7
            }
        );

        // Repo yang null dilewati; deskripsi jadi teks polos satu baris.
        assert_eq!(d.pinned.len(), 2);
        assert_eq!(d.pinned[0].title, "flexinix");
        assert_eq!(d.pinned[0].description, "Nix flakes <b>x</b>");
        assert_eq!(
            d.pinned[0].topic.as_deref(),
            Some("Nixos Configuration Flakes")
        );
        assert_eq!(d.pinned[0].language.as_deref(), Some("Nix"));
        assert_eq!(d.pinned[1].title, "racooonfig");
        assert_eq!(d.pinned[1].description, "");
        assert_eq!(d.pinned[1].language, None);

        let c = &d.chronicle;
        assert_eq!(c.total_contribution_volume, 120);
        assert!((c.growth_percent - 20.0).abs() < f64::EPSILON);
        assert_eq!(c.stack, ["NixOS", "Rust"]);
        assert_eq!(c.timeline.len(), 10);
        assert_eq!(c.timeline[0].date, "2026-09-28");
        assert_eq!(c.timeline[9].date, "2026-10-20");
    }

    #[test]
    fn languages_are_summed_ranked_and_colors_are_validated() {
        let c = parse(&github_response()).unwrap().chronicle;
        let names: Vec<_> = c.languages.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Nix", "Rust", "C++"]);
        let pct: Vec<_> = c.languages.iter().map(|l| l.percent).collect();
        assert_eq!(pct, [63.6, 27.3, 9.1]);
        assert_eq!(c.languages[0].color.as_deref(), Some("#7e7eff"));
        assert_eq!(
            c.languages[2].color, None,
            "an injected color must be dropped"
        );
    }

    #[test]
    fn streak_ignores_an_empty_today_but_stops_at_the_first_empty_day() {
        let c = parse(&github_response()).unwrap().chronicle;
        // 20 Okt: 0 (hari ini, tidak memutus), 19 Okt: 3, 18 Okt: 2, 17 Okt: 0.
        assert_eq!(c.current_streak, 2);
    }

    #[test]
    fn busiest_days_come_first_and_ties_keep_the_earlier_day() {
        let c = parse(&github_response()).unwrap().chronicle;
        let top: Vec<_> = c
            .top_activities
            .iter()
            .map(|d| (d.date.as_str(), d.count))
            .collect();
        assert_eq!(
            top,
            [("2026-10-05", 14), ("2026-09-30", 9), ("2026-10-06", 7)]
        );
    }

    #[test]
    fn the_weekday_and_focus_are_about_this_month_only() {
        let c = parse(&github_response()).unwrap().chronicle;
        // Oktober: Senin 14 + 3 = 17 adalah yang terbanyak; 30 Sep tidak dihitung.
        assert_eq!(c.most_productive_weekday, 1);
        // racooonfig 12 commit > flexinix 9; repo bernama aneh dilewati.
        assert_eq!(
            c.monthly_focus,
            Some(Focus {
                repo: "racooonfig".into(),
                language: None
            })
        );
    }

    #[test]
    fn an_empty_account_still_produces_a_valid_payload() {
        let body = json!({"data": {"user": {}}});
        let d = parse(&body).unwrap();
        assert_eq!(d.validate(), Ok(()));
        assert!(d.pinned.is_empty());
        assert_eq!(d.chronicle.current_streak, 0);
        assert_eq!(d.chronicle.monthly_focus, None);
        assert!(d.chronicle.top_activities.is_empty());
        assert!(d.chronicle.languages.is_empty());
        // Tanpa aktivitas: hari dalam seminggu jatuh ke hari ini (Selasa).
        assert_eq!(d.chronicle.most_productive_weekday, 2);
        assert!((d.chronicle.growth_percent).abs() < f64::EPSILON);
    }

    #[test]
    fn hostile_github_text_is_flattened_and_never_becomes_markup_fields() {
        let mut body = github_response();
        body["data"]["repo0"]["description"] = json!("a\u{202E}b\u{0}\n<img src=x onerror=1>");
        body["data"]["repo0"]["primaryLanguage"] = json!({"name": "Ru\u{200B}st\nX"});
        let d = parse(&body).unwrap();
        assert_eq!(d.pinned[0].description, "ab <img src=x onerror=1>");
        assert_eq!(d.pinned[0].language.as_deref(), Some("Rust X"));
    }

    #[test]
    fn absurd_numbers_are_clamped_not_rejected() {
        let mut body = github_response();
        body["data"]["user"]["currentYear"]["contributionCalendar"]["weeks"][0]
            ["contributionDays"][0]["contributionCount"] = json!(u64::MAX);
        body["data"]["user"]["repositories"]["totalCount"] = json!(u64::MAX);
        let d = parse(&body).unwrap();
        assert_eq!(d.hero.total_repos, u32::MAX);
        assert_eq!(d.chronicle.timeline[0].count, limits::MAX_DAY_COUNT);
    }

    #[test]
    fn dates_that_are_not_plain_days_are_dropped() {
        let mut body = github_response();
        body["data"]["user"]["currentYear"]["contributionCalendar"]["weeks"][0]
            ["contributionDays"][0]["date"] = json!("2026-09-28\"><script>");
        let d = parse(&body).unwrap();
        assert_eq!(d.chronicle.timeline.len(), 9);
    }

    #[test]
    fn github_failures_are_upstream_errors() {
        let err = |status: u16, body: &str| {
            parse_and_transform(
                &HttpResponse {
                    status,
                    body: body.to_owned(),
                },
                &cfg(),
                now(),
            )
            .unwrap_err()
        };
        for e in [
            err(401, "{}"),
            err(502, "<html>"),
            err(200, "not json"),
            err(200, "{}"),
            err(200, r#"{"data": null}"#),
            err(200, r#"{"data": {"user": null}}"#),
            err(200, r#"{"errors":[{"message":"Bad credentials"}]}"#),
            err(200, r#"{"data":{"user":{}},"errors":[{"message":"x"}]}"#),
        ] {
            assert!(matches!(e, JournalError::Upstream(_)), "{e:?}");
        }
        // Pesan galat GitHub dipotong supaya log tidak membengkak.
        let long = format!(r#"{{"errors":[{{"message":"{}"}}]}}"#, "x".repeat(1000));
        let JournalError::Upstream(msg) = err(200, &long) else {
            panic!()
        };
        assert!(msg.len() < 260, "{}", msg.len());
    }

    // ------------------------------------------------------------ database palsu

    struct Script {
        replies: RefCell<Vec<Result<HttpResponse, String>>>,
        seen: RefCell<Vec<Json>>,
    }

    impl Script {
        fn new(mut replies: Vec<Result<HttpResponse, String>>) -> Self {
            replies.reverse();
            Self {
                replies: RefCell::new(replies),
                seen: RefCell::new(Vec::new()),
            }
        }
        fn requests(&self) -> usize {
            self.seen.borrow().len()
        }
        fn sql(&self, i: usize) -> String {
            self.seen.borrow()[i]["requests"][0]["stmt"]["sql"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        }
    }

    impl Transport for &Script {
        fn post(
            &self,
            _: &str,
            _: Option<&str>,
            body: String,
        ) -> impl Future<Output = Result<HttpResponse, String>> {
            self.seen
                .borrow_mut()
                .push(serde_json::from_str(&body).unwrap());
            let reply = self
                .replies
                .borrow_mut()
                .pop()
                .expect("more requests than scripted replies");
            async move { reply }
        }
    }

    fn db(s: &Script) -> Client<&Script> {
        Client::new("http://127.0.0.1:1", None, s).unwrap()
    }

    fn cell(v: &Json) -> Json {
        match v {
            Json::Null => json!({"type": "null"}),
            Json::Number(n) => json!({"type": "integer", "value": n.to_string()}),
            Json::String(s) => json!({"type": "text", "value": s}),
            other => panic!("unsupported {other}"),
        }
    }

    fn result(cols: &[&str], rows: &[Vec<Json>]) -> Json {
        json!({
            "cols": cols.iter().map(|c| json!({"name": c})).collect::<Vec<_>>(),
            "rows": rows.iter().map(|r| r.iter().map(cell).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "affected_row_count": 0
        })
    }

    fn reply(res: Json) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status: 200,
            body: json!({"results": [
                {"type": "ok", "response": {"type": "execute", "result": res}},
                {"type": "ok", "response": {"type": "close"}}
            ]})
            .to_string(),
        })
    }

    fn config_replies() -> Vec<Result<HttpResponse, String>> {
        vec![
            reply(result(&["github_username"], &[vec![json!("moch")]])),
            reply(result(
                &["repo_name", "topic"],
                &[
                    vec![json!("flexinix"), json!("Flakes")],
                    vec![json!("racooonfig"), Json::Null],
                ],
            )),
            reply(result(
                &["name"],
                &[vec![json!("NixOS")], vec![json!("Rust")]],
            )),
        ]
    }

    fn tx_ok() -> Result<HttpResponse, String> {
        let empty = result(&[], &[]);
        Ok(HttpResponse {
            status: 200,
            body: json!({"results": [
                {"type": "ok", "response": {"type": "batch", "result": {
                    "step_results": [empty, empty, empty, empty, null, null],
                    "step_errors": [null, null, null, null, null, null]
                }}},
                {"type": "ok", "response": {"type": "close"}}
            ]})
            .to_string(),
        })
    }

    #[test]
    fn config_comes_from_the_tables_and_bad_rows_are_dropped() {
        let s = Script::new(vec![
            reply(result(&["github_username"], &[vec![json!("moch")]])),
            reply(result(
                &["repo_name", "topic"],
                &[
                    vec![json!("ok-repo"), json!("  Topic \n here ")],
                    vec![json!("evil\") { x }"), Json::Null],
                    vec![json!("ok-repo"), json!("duplicate")],
                    vec![json!("second"), json!("")],
                ],
            )),
            reply(result(
                &["name"],
                &[vec![json!("Rust")], vec![json!("  ")], vec![json!("Rust")]],
            )),
        ]);
        let cfg = futures_executor::block_on(load_config(&db(&s))).unwrap();
        assert_eq!(cfg.username, "moch");
        assert_eq!(
            cfg.pinned,
            [
                PinnedRepo {
                    name: "ok-repo".into(),
                    topic: Some("Topic here".into())
                },
                PinnedRepo {
                    name: "second".into(),
                    topic: None
                },
            ]
        );
        assert_eq!(cfg.stack, ["Rust"]);
    }

    #[test]
    fn a_missing_or_malformed_account_name_stops_everything() {
        for rows in [
            vec![],
            vec![vec![json!("")]],
            vec![vec![json!("a b")]],
            vec![vec![json!("x\" ) { }")]],
            vec![vec![json!("a".repeat(40))]],
        ] {
            let s = Script::new(vec![reply(result(&["github_username"], &rows))]);
            let err = futures_executor::block_on(load_config(&db(&s))).unwrap_err();
            assert!(matches!(err, JournalError::BadData(_)), "{rows:?}: {err:?}");
            assert_eq!(s.requests(), 1, "nothing else is read");
        }
    }

    #[test]
    fn usernames_use_the_github_alphabet() {
        for ok in ["moch", "a-b", "A1", &"a".repeat(39)] {
            assert!(is_github_username(ok), "{ok}");
        }
        for bad in ["", "a b", "a_b", "é", "a/b", &"a".repeat(40)] {
            assert!(!is_github_username(bad), "{bad:?}");
        }
    }

    // ------------------------------------------------------------------ refresh

    struct Github {
        reply: RefCell<Option<Result<HttpResponse, String>>>,
        bodies: RefCell<Vec<String>>,
    }

    impl Github {
        fn new(reply: Result<HttpResponse, String>) -> Self {
            Self {
                reply: RefCell::new(Some(reply)),
                bodies: RefCell::new(Vec::new()),
            }
        }
        fn ok(body: &Json) -> Self {
            Self::new(Ok(HttpResponse {
                status: 200,
                body: body.to_string(),
            }))
        }
    }

    impl GraphQl for &Github {
        fn post(&self, body: String) -> impl Future<Output = Result<HttpResponse, String>> {
            self.bodies.borrow_mut().push(body);
            let reply = self.reply.borrow_mut().take().expect("one GitHub call");
            async move { reply }
        }
    }

    #[test]
    fn refresh_reads_config_calls_github_and_stores_one_snapshot() {
        let mut replies = config_replies();
        replies.push(tx_ok());
        let s = Script::new(replies);
        let gh = Github::ok(&github_response());
        let out = futures_executor::block_on(refresh(&db(&s), &&gh, now())).unwrap();
        assert_eq!(out.fetched_at, "2026-10-20T12:00:00Z");
        assert_eq!(s.requests(), 4);

        // Satu transaksi: INSERT lalu pemangkasan.
        let seen = s.seen.borrow();
        let steps = &seen[3]["requests"][0]["batch"]["steps"];
        let sqls: Vec<&str> = steps
            .as_array()
            .unwrap()
            .iter()
            .map(|st| st["stmt"]["sql"].as_str().unwrap())
            .collect();
        assert!(sqls[1].contains("INSERT INTO journal_snapshots"));
        assert!(sqls[2].contains("DELETE FROM journal_snapshots"));
        let args = steps[1]["stmt"]["args"].as_array().unwrap();
        let payload: Data = serde_json::from_str(args[0]["value"].as_str().unwrap()).unwrap();
        assert_eq!(payload.validate(), Ok(()));
        assert_eq!(args[1]["value"], "2026-10-20T12:00:00Z");
        assert_eq!(
            steps[2]["stmt"]["args"][0]["value"],
            KEEP_SNAPSHOTS.to_string()
        );

        // Hanya dua repo dari tabel yang diminta ke GitHub.
        let sent: Json = serde_json::from_str(&gh.bodies.borrow()[0]).unwrap();
        assert_eq!(sent["variables"]["repo1"], "racooonfig");
        assert!(sent["variables"].get("repo2").is_none());
    }

    #[test]
    fn a_failing_github_stores_nothing_and_keeps_the_old_snapshot() {
        for gh in [
            Github::new(Err("offline".into())),
            Github::new(Ok(HttpResponse {
                status: 502,
                body: "bad gateway".into(),
            })),
            Github::ok(&json!({"errors": [{"message": "Bad credentials"}]})),
        ] {
            let s = Script::new(config_replies());
            let err = futures_executor::block_on(refresh(&db(&s), &&gh, now())).unwrap_err();
            assert!(matches!(err, JournalError::Upstream(_)), "{err:?}");
            assert_eq!(s.requests(), 3, "only the config was read; no write");
        }
    }

    #[test]
    fn the_token_never_appears_in_the_request_body_or_errors() {
        // Token dikirim lewat header oleh `github.rs`, bukan di body atau pesan.
        let body = request_body(&cfg(), now());
        assert!(!body.to_lowercase().contains("token"));
    }

    #[test]
    fn manual_refresh_is_refused_right_after_a_snapshot() {
        let s = Script::new(vec![reply(result(
            &["fetched_at"],
            &[vec![json!("2026-10-20T11:59:30Z")]],
        ))]);
        let gh = Github::new(Err("must not be called".into()));
        let err = futures_executor::block_on(refresh_if_due(&db(&s), &&gh, now())).unwrap_err();
        assert_eq!(
            err,
            JournalError::TooSoon {
                retry_after_secs: 30
            }
        );
        assert!(gh.bodies.borrow().is_empty());
    }

    #[test]
    fn manual_refresh_goes_ahead_when_the_last_snapshot_is_old_or_missing() {
        for latest in [vec![], vec![vec![json!("2026-10-20T11:00:00Z")]]] {
            let mut replies = vec![reply(result(&["fetched_at"], &latest))];
            replies.extend(config_replies());
            replies.push(tx_ok());
            let s = Script::new(replies);
            let gh = Github::ok(&github_response());
            let out = futures_executor::block_on(refresh_if_due(&db(&s), &&gh, now())).unwrap();
            assert_eq!(out.fetched_at, "2026-10-20T12:00:00Z");
        }
    }

    // --------------------------------------------------------------------- baca

    fn latest_reply(payload: &str, source: &str, at: &str) -> Result<HttpResponse, String> {
        reply(result(
            &["payload", "source", "fetched_at"],
            &[vec![json!(payload), json!(source), json!(at)]],
        ))
    }

    fn valid_payload() -> String {
        serde_json::to_string(&parse(&github_response()).unwrap()).unwrap()
    }

    #[test]
    fn latest_returns_the_validated_snapshot() {
        let s = Script::new(vec![latest_reply(
            &valid_payload(),
            "github",
            "2026-10-20T06:00:00Z",
        )]);
        let snap = futures_executor::block_on(latest(&db(&s)))
            .unwrap()
            .unwrap();
        assert_eq!(snap.fetched_at, "2026-10-20T06:00:00Z");
        assert_eq!(snap.source, Source::Github);
        assert_eq!(snap.data.hero.total_followers, 7);
        assert!(s.sql(0).contains("ORDER BY fetched_at DESC, id DESC"));
    }

    #[test]
    fn latest_is_none_before_the_first_refresh() {
        let s = Script::new(vec![reply(result(
            &["payload", "source", "fetched_at"],
            &[],
        ))]);
        assert_eq!(futures_executor::block_on(latest(&db(&s))).unwrap(), None);
    }

    #[test]
    fn a_tampered_snapshot_is_a_server_error_not_something_sent_to_browsers() {
        let tampered = valid_payload().replace("flexinix", "<script>x</script>");
        for (payload, source, at) in [
            (tampered.as_str(), "github", "2026-10-20T06:00:00Z"),
            ("{nope", "github", "2026-10-20T06:00:00Z"),
            ("{}", "github", "2026-10-20T06:00:00Z"),
            (valid_payload().as_str(), "mystery", "2026-10-20T06:00:00Z"),
            (valid_payload().as_str(), "github", "yesterday"),
        ] {
            let s = Script::new(vec![latest_reply(payload, source, at)]);
            let err = futures_executor::block_on(latest(&db(&s))).unwrap_err();
            assert!(matches!(err, JournalError::BadData(_)), "{err:?}");
        }
    }

    #[test]
    fn database_failures_are_db_errors() {
        let s = Script::new(vec![Err("offline".into())]);
        let err = futures_executor::block_on(latest(&db(&s))).unwrap_err();
        assert!(matches!(err, JournalError::Db(Error::Transport(_))));
    }
}
