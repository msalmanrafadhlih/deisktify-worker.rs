//! Data GitHub Journal (halaman `/profile`): bentuk yang disimpan di
//! `journal_snapshots.payload` dan dikirim `GET /api/v1/journal`.
//!
//! Aturan utama: **data, bukan tampilan.** Tidak ada HTML, tidak ada kalimat
//! berbahasa tertentu, tidak ada tanggal yang sudah diformat. Klien menyusun
//! kalimat dari `Strings` dan menggambar dengan komponen, jadi isi payload
//! tidak pernah menjadi markup. Karena payload bisa diubah lewat database
//! (panel admin D6), [`Data::validate`] dijalankan server sebelum menyimpan
//! **dan** sebelum mengirim, dan klien boleh memanggilnya lagi.

use serde::{Deserialize, Serialize};

use crate::chat::hidden;

/// Batas isi payload. Dipakai [`Data::validate`].
pub mod limits {
    pub const MAX_PINNED: usize = 12;
    pub const MAX_LANGUAGES: usize = 10;
    pub const MAX_STACK: usize = 12;
    /// Satu tahun penuh ditambah ruang untuk tahun kabisat dan hari ini.
    pub const MAX_TIMELINE: usize = 400;
    pub const MAX_TOP_ACTIVITIES: usize = 5;
    /// Nama repositori GitHub paling panjang 100 karakter.
    pub const MAX_REPO_NAME_CHARS: usize = 100;
    pub const MAX_TOPIC_CHARS: usize = 60;
    pub const MAX_DESCRIPTION_CHARS: usize = 400;
    pub const MAX_NAME_CHARS: usize = 40;
    /// Batas hitungan kontribusi per hari yang masih masuk akal.
    pub const MAX_DAY_COUNT: u32 = 100_000;
}

/// Asal sebuah snapshot. `Seed` = contoh dari berkas seed lokal, **bukan**
/// angka GitHub sungguhan; klien harus menandainya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Github,
    Seed,
}

impl Source {
    /// Nilai kolom `journal_snapshots.source`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Source::Github),
            "seed" => Some(Source::Seed),
            _ => None,
        }
    }
}

/// Respons `GET /api/v1/journal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`: kapan data diambil dari GitHub.
    pub fetched_at: String,
    pub source: Source,
    pub data: Data,
}

/// Respons `POST /api/v1/admin/journal/refresh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refreshed {
    pub fetched_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Data {
    pub hero: Hero,
    pub pinned: Vec<Repo>,
    pub chronicle: Chronicle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hero {
    pub total_repos: u32,
    pub total_stars: u32,
    pub total_followers: u32,
}

/// Repositori yang disematkan di halaman.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    pub title: String,
    /// Teks polos satu baris (bukan Markdown, bukan HTML); boleh kosong.
    pub description: String,
    pub stars: u32,
    pub topic: Option<String>,
    /// Bahasa utama; `None` jika GitHub tidak mendeteksinya.
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chronicle {
    /// Kontribusi 12 bulan terakhir.
    pub total_contribution_volume: u32,
    /// Perubahan dibanding 12 bulan sebelumnya, persen (satu desimal);
    /// `0.0` jika periode sebelumnya kosong.
    pub growth_percent: f64,
    /// Hari berturut-turut dengan kontribusi sampai hari ini (hari ini yang
    /// masih kosong tidak memutus rangkaian).
    pub current_streak: u32,
    /// Hari tersibuk, terbanyak dulu (seri: yang lebih awal dulu). Isi pertama
    /// adalah hari puncak. Paling banyak tiga.
    pub top_activities: Vec<Day>,
    /// Repositori dengan commit terbanyak bulan ini (UTC); `None` jika belum
    /// ada commit bulan ini.
    pub monthly_focus: Option<Focus>,
    /// Hari dalam seminggu dengan kontribusi terbanyak bulan ini: 0 = Minggu
    /// ... 6 = Sabtu. Klien yang menerjemahkan nama harinya.
    pub most_productive_weekday: u8,
    /// Lima bahasa teratas menurut ukuran kode.
    pub languages: Vec<Language>,
    /// Daftar teknologi pilihan pemilik (dari konfigurasi server).
    pub stack: Vec<String>,
    /// Satu titik per hari, urut menaik.
    pub timeline: Vec<Day>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Focus {
    pub repo: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Language {
    pub name: String,
    /// 0..=100, satu desimal.
    pub percent: f64,
    /// `#rrggbb` jika GitHub memberinya dan lolos [`is_hex_color`].
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    /// `YYYY-MM-DD` (UTC).
    pub date: String,
    pub count: u32,
}

// ------------------------------------------------------------------ pembersih

/// Teks bebas dari luar (GitHub) -> teks polos satu baris: spasi berurutan
/// (juga baris baru dan tab) jadi satu spasi, karakter kontrol dan pembalik
/// arah dibuang, dipotong ke `max` karakter.
pub fn clean_text(raw: &str, max: usize) -> String {
    // Spasi putih (termasuk baris baru dan tab, yang juga karakter kontrol)
    // jadi spasi DULU; kalau tidak, "a\nb" terbuang menjadi "ab".
    let visible: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !hidden(*c))
        .collect();
    let joined = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(max).collect()
}

/// `#` diikuti tepat enam digit heksadesimal.
pub fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// `YYYY-MM-DD` dengan bulan 1..=12 dan hari 1..=31 (bukan pemeriksaan kalender
/// penuh; cukup untuk memastikan hanya angka dan tanda hubung).
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// Nama repositori GitHub: huruf, angka, `.`, `-`, `_`.
pub fn is_repo_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= limits::MAX_REPO_NAME_CHARS
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

// ------------------------------------------------------------------ validasi

/// Alasan payload ditolak. Hanya untuk log; tidak dikirim ke klien.
pub type Invalid = &'static str;

/// Teks yang sudah "bersih": tidak ada karakter kontrol/tak terlihat, tidak ada
/// spasi di ujung atau ganda, dan tidak lebih dari `max` karakter.
fn tidy(s: &str, max: usize) -> bool {
    s.chars().count() <= max && clean_text(s, usize::MAX) == s
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && tidy(s, limits::MAX_NAME_CHARS)
}

fn opt_name_ok(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(name_ok)
}

fn day_ok(d: &Day) -> bool {
    is_iso_date(&d.date) && d.count <= limits::MAX_DAY_COUNT
}

impl Data {
    /// Memeriksa batas dan isi. Dipakai server sebelum menyimpan dan sebelum
    /// mengirim (payload bisa diubah lewat database), dan boleh dipakai klien.
    pub fn validate(&self) -> Result<(), Invalid> {
        use limits::*;

        if self.pinned.len() > MAX_PINNED {
            return Err("too many pinned repositories");
        }
        for r in &self.pinned {
            if !is_repo_name(&r.title) {
                return Err("pinned repository name");
            }
            if !tidy(&r.description, MAX_DESCRIPTION_CHARS) {
                return Err("pinned repository description");
            }
            if !r
                .topic
                .as_deref()
                .is_none_or(|t| !t.is_empty() && tidy(t, MAX_TOPIC_CHARS))
            {
                return Err("pinned repository topic");
            }
            if !opt_name_ok(&r.language) {
                return Err("pinned repository language");
            }
        }

        let c = &self.chronicle;
        if !c.growth_percent.is_finite() || c.growth_percent.abs() > 1_000_000.0 {
            return Err("growth percent");
        }
        if c.most_productive_weekday > 6 {
            return Err("weekday");
        }
        if c.top_activities.len() > MAX_TOP_ACTIVITIES || !c.top_activities.iter().all(day_ok) {
            return Err("top activities");
        }
        if c.timeline.len() > MAX_TIMELINE || !c.timeline.iter().all(day_ok) {
            return Err("timeline");
        }
        if let Some(f) = &c.monthly_focus {
            if !is_repo_name(&f.repo) || !opt_name_ok(&f.language) {
                return Err("monthly focus");
            }
        }
        if c.languages.len() > MAX_LANGUAGES {
            return Err("too many languages");
        }
        for l in &c.languages {
            if !name_ok(&l.name) {
                return Err("language name");
            }
            if !(l.percent.is_finite() && (0.0..=100.0).contains(&l.percent)) {
                return Err("language percent");
            }
            if l.color.as_deref().is_some_and(|col| !is_hex_color(col)) {
                return Err("language color");
            }
        }
        if c.stack.len() > MAX_STACK || !c.stack.iter().all(|s| name_ok(s)) {
            return Err("stack");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Data {
        Data {
            hero: Hero {
                total_repos: 3,
                total_stars: 5,
                total_followers: 2,
            },
            pinned: vec![Repo {
                title: "racooonfig".into(),
                description: "Linux dotfiles".into(),
                stars: 1,
                topic: Some("Linux Dotfiles".into()),
                language: Some("Nix".into()),
            }],
            chronicle: Chronicle {
                total_contribution_volume: 120,
                growth_percent: 12.5,
                current_streak: 3,
                top_activities: vec![Day {
                    date: "2026-10-05".into(),
                    count: 14,
                }],
                monthly_focus: Some(Focus {
                    repo: "flexinix".into(),
                    language: Some("Nix".into()),
                }),
                most_productive_weekday: 2,
                languages: vec![Language {
                    name: "C++".into(),
                    percent: 60.5,
                    color: Some("#f34b7d".into()),
                }],
                stack: vec!["NixOS".into(), "Rust".into()],
                timeline: vec![Day {
                    date: "2026-10-19".into(),
                    count: 0,
                }],
            },
        }
    }

    #[test]
    fn a_normal_payload_is_valid_and_survives_json() {
        let d = sample();
        assert_eq!(d.validate(), Ok(()));
        let back: Data = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn json_shape_is_stable_and_has_no_markup_fields() {
        let v = serde_json::to_value(sample()).unwrap();
        let c = &v["chronicle"];
        assert_eq!(c["most_productive_weekday"], 2);
        assert_eq!(c["monthly_focus"]["repo"], "flexinix");
        assert_eq!(c["languages"][0]["color"], "#f34b7d");
        assert!(c.get("monthly_focus_html").is_none());
        assert!(v["hero"].is_object() && v["pinned"].is_array());
        let snap = Snapshot {
            fetched_at: "2026-10-20T12:00:00Z".into(),
            source: Source::Github,
            data: sample(),
        };
        let s = serde_json::to_value(&snap).unwrap();
        assert_eq!(s["source"], "github");
        assert_eq!(s["fetched_at"], "2026-10-20T12:00:00Z");
    }

    #[test]
    fn sources_match_the_database_column() {
        assert_eq!(Source::parse("github"), Some(Source::Github));
        assert_eq!(Source::parse("seed"), Some(Source::Seed));
        assert_eq!(Source::parse("Github"), None);
        assert_eq!(Source::parse(""), None);
    }

    #[test]
    fn clean_text_flattens_strips_hidden_characters_and_truncates() {
        assert_eq!(clean_text("  a \n\t b  ", 50), "a b");
        assert_eq!(clean_text("a\nb\tc\r\nd", 50), "a b c d");
        assert_eq!(clean_text("a\u{2028}b\u{A0}c", 50), "a b c");
        assert_eq!(clean_text("x\u{202E}y\u{200B}z\u{0}", 50), "xyz");
        assert_eq!(clean_text("héllo wörld", 5), "héllo");
        assert_eq!(clean_text("<b>hi</b> & co", 50), "<b>hi</b> & co");
        assert_eq!(clean_text("", 5), "");
    }

    #[test]
    fn colors_are_exactly_hash_plus_six_hex_digits() {
        for ok in ["#000000", "#F34B7d", "#a1B2c3"] {
            assert!(is_hex_color(ok), "{ok}");
        }
        for bad in [
            "",
            "#fff",
            "f34b7d",
            "#f34b7",
            "#f34b7dd",
            "#ggggggg",
            "red",
            "#f34b7d;background:url(x)",
            "#f34b7d\"",
            "#ééé",
        ] {
            assert!(!is_hex_color(bad), "{bad:?}");
        }
    }

    #[test]
    fn dates_are_plain_iso_days() {
        for ok in ["2026-10-20", "2024-02-29", "1999-12-31"] {
            assert!(is_iso_date(ok), "{ok}");
        }
        for bad in [
            "",
            "2026-1-2",
            "2026-13-01",
            "2026-00-10",
            "2026-10-00",
            "2026-10-32",
            "2026/10/20",
            "2026-10-20T00:00:00Z",
            "2026-10-2<",
            "٢٠٢٦-١٠-٢٠",
            "20x6-10-20",
        ] {
            assert!(!is_iso_date(bad), "{bad:?}");
        }
    }

    #[test]
    fn repo_names_use_the_github_alphabet() {
        for ok in ["flexinix", "fastfetch.md", "tquilla.is-a.bot", "a_b-c.1"] {
            assert!(is_repo_name(ok), "{ok}");
        }
        for bad in ["", "a b", "a/b", "<b>", "a\"b", "é", &"x".repeat(101)] {
            assert!(!is_repo_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn validation_rejects_each_kind_of_bad_payload() {
        type Case = (&'static str, Box<dyn Fn(&mut Data)>);
        let cases: Vec<Case> = vec![
            ("repo name", Box::new(|d| d.pinned[0].title = "a b".into())),
            (
                "description control",
                Box::new(|d| d.pinned[0].description = "a\nb".into()),
            ),
            (
                "description too long",
                Box::new(|d| d.pinned[0].description = "x".repeat(401)),
            ),
            (
                "description spaces",
                Box::new(|d| d.pinned[0].description = " padded ".into()),
            ),
            (
                "empty topic",
                Box::new(|d| d.pinned[0].topic = Some(String::new())),
            ),
            (
                "bidi language",
                Box::new(|d| d.pinned[0].language = Some("N\u{202E}ix".into())),
            ),
            (
                "growth nan",
                Box::new(|d| d.chronicle.growth_percent = f64::NAN),
            ),
            (
                "growth huge",
                Box::new(|d| d.chronicle.growth_percent = 1e12),
            ),
            (
                "weekday",
                Box::new(|d| d.chronicle.most_productive_weekday = 7),
            ),
            (
                "bad date in timeline",
                Box::new(|d| d.chronicle.timeline[0].date = "2026-10-20\"><".into()),
            ),
            (
                "bad date in top",
                Box::new(|d| d.chronicle.top_activities[0].date = "x".into()),
            ),
            (
                "count too big",
                Box::new(|d| d.chronicle.timeline[0].count = 100_001),
            ),
            (
                "focus repo",
                Box::new(|d| d.chronicle.monthly_focus.as_mut().unwrap().repo = "<x>".into()),
            ),
            (
                "color injection",
                Box::new(|d| {
                    d.chronicle.languages[0].color = Some("red;background:url(//x)".into());
                }),
            ),
            (
                "percent over 100",
                Box::new(|d| d.chronicle.languages[0].percent = 100.1),
            ),
            (
                "percent nan",
                Box::new(|d| d.chronicle.languages[0].percent = f64::NAN),
            ),
            (
                "empty stack item",
                Box::new(|d| d.chronicle.stack.push(String::new())),
            ),
        ];
        for (name, mutate) in cases {
            let mut d = sample();
            mutate(&mut d);
            assert!(d.validate().is_err(), "{name} must be rejected");
        }
    }

    #[test]
    fn validation_bounds_every_list() {
        let mut d = sample();
        d.pinned = vec![d.pinned[0].clone(); limits::MAX_PINNED + 1];
        assert!(d.validate().is_err());

        let mut d = sample();
        d.chronicle.timeline = vec![d.chronicle.timeline[0].clone(); limits::MAX_TIMELINE + 1];
        assert!(d.validate().is_err());

        let mut d = sample();
        d.chronicle.languages = vec![d.chronicle.languages[0].clone(); limits::MAX_LANGUAGES + 1];
        assert!(d.validate().is_err());

        let mut d = sample();
        d.chronicle.timeline = vec![d.chronicle.timeline[0].clone(); limits::MAX_TIMELINE];
        d.pinned = vec![d.pinned[0].clone(); limits::MAX_PINNED];
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn missing_optional_parts_are_fine() {
        let mut d = sample();
        d.chronicle.monthly_focus = None;
        d.chronicle.languages[0].color = None;
        d.pinned[0].topic = None;
        d.pinned[0].language = None;
        d.pinned[0].description = String::new();
        d.chronicle.top_activities.clear();
        d.chronicle.timeline.clear();
        assert_eq!(d.validate(), Ok(()));
    }
}
