//! `migrate`: menjalankan migrasi dan seed SQL ke Turso/libSQL dari terminal.
//!
//! ```text
//! TURSO_DATABASE_URL=http://127.0.0.1:8081 cargo run -p migrate -- up
//! TURSO_DATABASE_URL=http://127.0.0.1:8081 cargo run -p migrate -- seed --samples
//! ```
//!
//! Rahasia hanya dibaca dari environment (`TURSO_AUTH_TOKEN`), tidak pernah
//! dicetak. Alat ini native dan bukan bagian dari Worker.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use turso_http::migrate::{self, Migration};
use turso_http::{Client, HttpResponse, Transport};

const USAGE: &str = "\
usage: migrate <command> [options]

commands:
  status          show applied and pending migrations (changes nothing)
  up              apply pending migrations, one transaction each
  seed            run seeds/0001_content.sql (site content; safe to repeat)
  seed --samples  also run seeds/0002_samples.sql (fake chat + journal; local only)

options:
  --migrations <dir>  default: migrations
  --seeds <dir>       default: seeds
  --yes               required for `up` and `seed` on a non-local database

environment:
  TURSO_DATABASE_URL  required (libsql://..., https://... or http://127.0.0.1:PORT)
  TURSO_AUTH_TOKEN    optional for a local database, required for Turso cloud
";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Status,
    Up,
    Seed { samples: bool },
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
    command: Command,
    migrations_dir: PathBuf,
    seeds_dir: PathBuf,
    yes: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut it = args.iter();
    let name = it.next().ok_or("missing command")?;
    let mut samples = false;
    let mut yes = false;
    let mut migrations_dir = PathBuf::from("migrations");
    let mut seeds_dir = PathBuf::from("seeds");
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--samples" => samples = true,
            "--yes" => yes = true,
            "--migrations" => {
                migrations_dir = it.next().ok_or("--migrations needs a folder")?.into();
            }
            "--seeds" => seeds_dir = it.next().ok_or("--seeds needs a folder")?.into(),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    let command = match name.as_str() {
        "status" => Command::Status,
        "up" => Command::Up,
        "seed" => Command::Seed { samples },
        other => return Err(format!("unknown command {other:?}")),
    };
    if samples && !matches!(command, Command::Seed { .. }) {
        return Err("--samples only applies to `seed`".into());
    }
    Ok(Args {
        command,
        migrations_dir,
        seeds_dir,
        yes,
    })
}

/// `true` untuk database di mesin sendiri (`turso dev`).
fn is_local(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if let Some(stripped) = host.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Membaca `*.sql` dari folder, diurutkan menurut nama. Berkas lain diabaikan.
fn read_sql_dir(dir: &Path) -> Result<Vec<Migration>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.extension().is_none_or(|e| e != "sql") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("bad file name: {}", path.display()))?
            .to_owned();
        let sql = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        out.push(Migration::new(name, sql));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Transport HTTP sinkron (ureq). Token hanya masuk ke header, tidak ke log
/// atau pesan error.
struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        Self { agent }
    }
}

impl Transport for UreqTransport {
    fn post(
        &self,
        url: &str,
        bearer: Option<&str>,
        body: String,
    ) -> impl Future<Output = Result<HttpResponse, String>> {
        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/json");
        if let Some(token) = bearer {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let result = request
            .send(body)
            .map_err(|e| format!("request failed: {e}"))
            .and_then(|mut response| {
                let status = response.status().as_u16();
                let body = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| format!("could not read response: {e}"))?;
                Ok(HttpResponse { status, body })
            });
        async move { result }
    }
}

fn run(args: Args) -> Result<(), String> {
    let url = std::env::var("TURSO_DATABASE_URL")
        .map_err(|_| "TURSO_DATABASE_URL is not set".to_owned())?;
    let token = std::env::var("TURSO_AUTH_TOKEN").ok();
    let client =
        Client::new(&url, token.as_deref(), UreqTransport::new()).map_err(|e| e.to_string())?;
    let local = is_local(client.base_url());
    println!(
        "database: {} ({})",
        client.base_url(),
        if local { "local" } else { "remote" }
    );

    let mutating = !matches!(args.command, Command::Status);
    if mutating && !local && !args.yes {
        return Err("this is not a local database; repeat with --yes to confirm".into());
    }

    match args.command {
        Command::Status => {
            let migrations = read_sql_dir(&args.migrations_dir)?;
            let status = futures_executor::block_on(migrate::status(&client, &migrations))
                .map_err(|e| e.to_string())?;
            for name in &status.applied {
                println!("applied  {name}");
            }
            for name in &status.pending {
                println!("pending  {name}");
            }
            if status.pending.is_empty() {
                println!("up to date ({} applied)", status.applied.len());
            }
        }
        Command::Up => {
            let migrations = read_sql_dir(&args.migrations_dir)?;
            let ran = futures_executor::block_on(migrate::run(&client, &migrations))
                .map_err(|e| e.to_string())?;
            if ran.is_empty() {
                println!("nothing to do");
            }
            for name in ran {
                println!("applied  {name}");
            }
        }
        Command::Seed { samples } => {
            if samples && !local {
                return Err("--samples is for local databases only (fake data)".into());
            }
            let wanted: &[&str] = if samples {
                &["0001_content", "0002_samples"]
            } else {
                &["0001_content"]
            };
            let seeds = read_sql_dir(&args.seeds_dir)?;
            for name in wanted {
                let seed = seeds.iter().find(|s| s.name == *name).ok_or_else(|| {
                    format!("seed {name}.sql not found in {}", args.seeds_dir.display())
                })?;
                let count =
                    futures_executor::block_on(migrate::run_script(&client, &seed.name, &seed.sql))
                        .map_err(|e| e.to_string())?;
                println!("seeded   {name} ({count} statements)");
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("migrate: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("migrate: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pemecah pernyataan milik runner harus bisa membaca seed sungguhan (termasuk
    /// `INSERT ... SELECT ... FROM (VALUES ...)` dan tanda `%` di default kolom).
    #[test]
    fn the_real_content_seed_splits_into_whole_statements() {
        let sql = include_str!("../../../seeds/0001_content.sql");
        let stmts = migrate::split_statements(sql).expect("seed splits");
        // 1 CREATE + 6 tabel konten + 1 penanda.
        assert_eq!(stmts.len(), 8, "{stmts:#?}");
        assert!(stmts[0].starts_with("CREATE TABLE IF NOT EXISTS _seeded"));
        assert!(stmts[0].contains("%Y-%m-%dT%H:%M:%SZ"));
        for s in &stmts[1..7] {
            assert!(s.starts_with("INSERT OR IGNORE INTO"), "{s}");
            assert!(s.contains("WHERE NOT EXISTS (SELECT 1 FROM _seeded WHERE name = 'content')"));
        }
        assert!(stmts[7].contains("INTO _seeded"));
        let samples = include_str!("../../../seeds/0002_samples.sql");
        assert!(migrate::split_statements(samples).is_ok());
    }

    fn args(list: &[&str]) -> Result<Args, String> {
        parse_args(&list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_commands_and_options() {
        let a = args(&["up"]).unwrap();
        assert_eq!(a.command, Command::Up);
        assert_eq!(a.migrations_dir, PathBuf::from("migrations"));
        assert!(!a.yes);

        let a = args(&["seed", "--samples", "--seeds", "x", "--yes"]).unwrap();
        assert_eq!(a.command, Command::Seed { samples: true });
        assert_eq!(a.seeds_dir, PathBuf::from("x"));
        assert!(a.yes);

        assert_eq!(args(&["status"]).unwrap().command, Command::Status);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(args(&[]).is_err());
        assert!(args(&["nope"]).is_err());
        assert!(args(&["up", "--wat"]).is_err());
        assert!(args(&["up", "--samples"]).is_err());
        assert!(args(&["up", "--migrations"]).is_err());
    }

    #[test]
    fn recognises_local_databases() {
        for url in [
            "http://127.0.0.1:8081",
            "http://localhost:8081/",
            "http://[::1]:8081",
            "http://user@localhost:1",
        ] {
            assert!(is_local(url), "{url}");
        }
        for url in [
            "libsql://db-org.turso.io",
            "https://db-org.turso.io",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil.example",
            "https://evil.example/127.0.0.1",
            "https://127.0.0.1@evil.example",
        ] {
            assert!(!is_local(url), "{url}");
        }
    }

    #[test]
    fn reads_only_sql_files_in_name_order() {
        let dir = std::env::temp_dir().join(format!("migrate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0002_b.sql"), "SELECT 2;").unwrap();
        std::fs::write(dir.join("0001_a.sql"), "SELECT 1;").unwrap();
        std::fs::write(dir.join("README.md"), "ignored").unwrap();
        let list = read_sql_dir(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let names: Vec<_> = list.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["0001_a", "0002_b"]);
        assert!(read_sql_dir(&dir).is_err());
    }
}
