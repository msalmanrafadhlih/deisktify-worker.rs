//! Pengaturan Journal dari dashboard admin (D6b): akun GitHub, repo sematan,
//! dan stack, beserta urutannya, plus daftar snapshot.
//!
//! Server tidak memercayai klien: semua masukan diperiksa ulang di sini dengan
//! aturan yang sama seperti `journal::load_config` dan CHECK di migrasi `0009`.
//! Pesan galat bersifat tetap (tidak memuat masukan). Logika generik terhadap
//! [`Transport`] supaya dites dengan transport palsu; SQL-nya juga dijalankan di
//! SQLite sungguhan oleh `scripts/migrations.test.mjs`.

use api_types::journal::{clean_text, is_repo_name, limits};
use serde::{Deserialize, Serialize};
use turso_http::{Client, Stmt, Transport, Value};

use crate::content;
use crate::journal::is_github_username;
use crate::panel::{write_error, PanelError};
use crate::time::format_utc;

const PROFILE: &str = include_str!("queries/journal_profile.sql");
const PROFILE_SET: &str = include_str!("queries/admin_journal_profile_set.sql");
const SNAPSHOTS: &str = include_str!("queries/admin_journal_snapshots.sql");
const REPOS: &str = include_str!("queries/admin_journal_repos.sql");
const REPO_IDS: &str = include_str!("queries/admin_journal_repo_ids.sql");
const REPO_INSERT: &str = include_str!("queries/admin_journal_repo_insert.sql");
const REPO_UPDATE: &str = include_str!("queries/admin_journal_repo_update.sql");
const REPO_DELETE: &str = include_str!("queries/admin_journal_repo_delete.sql");
const REPO_POSITION: &str = include_str!("queries/admin_journal_repo_position.sql");
const STACK: &str = include_str!("queries/admin_journal_stack.sql");
const STACK_IDS: &str = include_str!("queries/admin_journal_stack_ids.sql");
const STACK_INSERT: &str = include_str!("queries/admin_journal_stack_insert.sql");
const STACK_UPDATE: &str = include_str!("queries/admin_journal_stack_update.sql");
const STACK_DELETE: &str = include_str!("queries/admin_journal_stack_delete.sql");
const STACK_POSITION: &str = include_str!("queries/admin_journal_stack_position.sql");

/// Berapa snapshot terbaru yang ditampilkan.
pub const SNAPSHOT_LIMIT: u32 = 20;

// ------------------------------------------------------------------ masukan

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileBody {
    pub github_username: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoBody {
    pub repo_name: String,
    #[serde(default)]
    pub topic: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackBody {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderBody {
    pub ids: Vec<u32>,
}

// ------------------------------------------------------------------- keluaran

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoRow {
    pub id: u32,
    pub repo_name: String,
    pub topic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackRow {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotRow {
    pub id: u32,
    pub fetched_at: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Caps {
    pub max_pinned: usize,
    pub max_stack: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JournalSettings {
    /// Jam server (UTC), supaya dashboard tidak bergantung pada jam perangkat.
    pub now: String,
    pub github_username: String,
    pub repos: Vec<RepoRow>,
    pub stack: Vec<StackRow>,
    pub snapshots: Vec<SnapshotRow>,
    pub caps: Caps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Repos,
    Stack,
}

// ------------------------------------------------------------------ pemeriksaan

pub fn check_username(raw: &str) -> Result<String, PanelError> {
    let name = raw.trim();
    if is_github_username(name) {
        Ok(name.to_owned())
    } else {
        Err(PanelError::Invalid("Invalid GitHub username."))
    }
}

pub fn check_repo(
    repo_name: &str,
    topic: Option<&str>,
) -> Result<(String, Option<String>), PanelError> {
    let name = repo_name.trim();
    if !is_repo_name(name) {
        return Err(PanelError::Invalid("Invalid repository name."));
    }
    let topic = topic
        .map(|t| clean_text(t, usize::MAX))
        .filter(|t| !t.is_empty());
    if let Some(t) = &topic {
        if t.chars().count() > limits::MAX_TOPIC_CHARS {
            return Err(PanelError::Invalid("The topic is too long."));
        }
    }
    Ok((name.to_owned(), topic))
}

pub fn check_stack_name(raw: &str) -> Result<String, PanelError> {
    let name = clean_text(raw, usize::MAX);
    if name.is_empty() || name.chars().count() > limits::MAX_NAME_CHARS {
        return Err(PanelError::Invalid("Invalid name."));
    }
    Ok(name)
}

// -------------------------------------------------------------------- membaca

/// Membaca akun, repo, stack, dan snapshot dalam satu permintaan ke database.
pub async fn load<T: Transport>(
    client: &Client<T>,
    now_secs: i64,
) -> Result<JournalSettings, PanelError> {
    let res = client
        .transaction(vec![
            Stmt::new(PROFILE),
            Stmt::new(REPOS),
            Stmt::new(STACK),
            Stmt::new(SNAPSHOTS).bind(SNAPSHOT_LIMIT),
        ])
        .await?;
    let [profile, repos, stack, snaps] =
        <[_; 4]>::try_from(res).map_err(|_| PanelError::BadData("journal settings"))?;

    if profile.rows.is_empty() {
        return Err(PanelError::BadData("github_username"));
    }
    let github_username = content::text(&profile, 0, "github_username")?;

    let repos = (0..repos.rows.len())
        .map(|i| {
            Ok(RepoRow {
                id: content::id(&repos, i, "id")?,
                repo_name: content::text(&repos, i, "repo_name")?,
                topic: content::opt_text(&repos, i, "topic")?,
            })
        })
        .collect::<Result<Vec<_>, content::ContentError>>()?;
    let stack = (0..stack.rows.len())
        .map(|i| {
            Ok(StackRow {
                id: content::id(&stack, i, "id")?,
                name: content::text(&stack, i, "name")?,
            })
        })
        .collect::<Result<Vec<_>, content::ContentError>>()?;
    let snapshots = (0..snaps.rows.len())
        .map(|i| {
            Ok(SnapshotRow {
                id: content::id(&snaps, i, "id")?,
                fetched_at: content::text(&snaps, i, "fetched_at")?,
                source: content::text(&snaps, i, "source")?,
            })
        })
        .collect::<Result<Vec<_>, content::ContentError>>()?;

    Ok(JournalSettings {
        now: format_utc(now_secs),
        github_username,
        repos,
        stack,
        snapshots,
        caps: Caps {
            max_pinned: limits::MAX_PINNED,
            max_stack: limits::MAX_STACK,
        },
    })
}

// ---------------------------------------------------------------------- akun

/// Mengganti akun GitHub; mengembalikan nama yang tersimpan.
pub async fn set_username<T: Transport>(
    client: &Client<T>,
    raw: &str,
) -> Result<String, PanelError> {
    let name = check_username(raw)?;
    let res = client
        .execute(Stmt::new(PROFILE_SET).bind(name.as_str()))
        .await
        .map_err(write_error)?;
    if res.rows.is_empty() {
        return Err(PanelError::BadData("github_username"));
    }
    Ok(name)
}

// ----------------------------------------------------------------------- repo

fn inserted_id(res: &turso_http::StmtResult) -> Result<u32, PanelError> {
    if res.rows.is_empty() {
        // Tidak ada baris: batas jumlah tercapai (satu-satunya syarat INSERT).
        return Err(PanelError::Invalid("The list is full."));
    }
    Ok(content::id(res, 0, "id")?)
}

/// Menambah repo di urutan terakhir. Mengembalikan id baru.
pub async fn add_repo<T: Transport>(
    client: &Client<T>,
    body: &RepoBody,
) -> Result<(u32, String), PanelError> {
    let (name, topic) = check_repo(&body.repo_name, body.topic.as_deref())?;
    let res = client
        .execute(
            Stmt::new(REPO_INSERT)
                .bind(name.as_str())
                .bind(topic.map_or(Value::Null, Value::from))
                .bind(u32::try_from(limits::MAX_PINNED).unwrap_or(u32::MAX)),
        )
        .await
        .map_err(write_error)?;
    Ok((inserted_id(&res)?, name))
}

/// Mengganti nama/topik repo. Mengembalikan nama yang tersimpan.
pub async fn edit_repo<T: Transport>(
    client: &Client<T>,
    id: u32,
    body: &RepoBody,
) -> Result<String, PanelError> {
    let (name, topic) = check_repo(&body.repo_name, body.topic.as_deref())?;
    let res = client
        .execute(
            Stmt::new(REPO_UPDATE)
                .bind(id)
                .bind(name.as_str())
                .bind(topic.map_or(Value::Null, Value::from)),
        )
        .await
        .map_err(write_error)?;
    if res.rows.is_empty() {
        return Err(PanelError::NotFound);
    }
    Ok(name)
}

/// Menghapus repo. Mengembalikan nama yang dihapus (untuk riwayat).
pub async fn delete_repo<T: Transport>(client: &Client<T>, id: u32) -> Result<String, PanelError> {
    let res = client.execute(Stmt::new(REPO_DELETE).bind(id)).await?;
    if res.rows.is_empty() {
        return Err(PanelError::NotFound);
    }
    Ok(content::text(&res, 0, "repo_name")?)
}

// ---------------------------------------------------------------------- stack

pub async fn add_stack<T: Transport>(
    client: &Client<T>,
    body: &StackBody,
) -> Result<(u32, String), PanelError> {
    let name = check_stack_name(&body.name)?;
    let res = client
        .execute(
            Stmt::new(STACK_INSERT)
                .bind(name.as_str())
                .bind(u32::try_from(limits::MAX_STACK).unwrap_or(u32::MAX)),
        )
        .await
        .map_err(write_error)?;
    Ok((inserted_id(&res)?, name))
}

pub async fn edit_stack<T: Transport>(
    client: &Client<T>,
    id: u32,
    body: &StackBody,
) -> Result<String, PanelError> {
    let name = check_stack_name(&body.name)?;
    let res = client
        .execute(Stmt::new(STACK_UPDATE).bind(id).bind(name.as_str()))
        .await
        .map_err(write_error)?;
    if res.rows.is_empty() {
        return Err(PanelError::NotFound);
    }
    Ok(name)
}

pub async fn delete_stack<T: Transport>(client: &Client<T>, id: u32) -> Result<String, PanelError> {
    let res = client.execute(Stmt::new(STACK_DELETE).bind(id)).await?;
    if res.rows.is_empty() {
        return Err(PanelError::NotFound);
    }
    Ok(content::text(&res, 0, "name")?)
}

// --------------------------------------------------------------------- urutan

/// Menyimpan urutan baru. `ids` harus memuat semua id yang ada, tepat sekali
/// masing-masing; selain itu ditolak dan tidak ada yang berubah.
pub async fn reorder<T: Transport>(
    client: &Client<T>,
    list: List,
    ids: &[u32],
) -> Result<(), PanelError> {
    let (select, update, cap) = match list {
        List::Repos => (REPO_IDS, REPO_POSITION, limits::MAX_PINNED),
        List::Stack => (STACK_IDS, STACK_POSITION, limits::MAX_STACK),
    };
    // Tolak yang jelas salah sebelum menyentuh database.
    if ids.len() > cap || has_duplicates(ids) {
        return Err(PanelError::Invalid("The order is not valid."));
    }
    let res = client.execute(Stmt::new(select)).await?;
    let mut current = (0..res.rows.len())
        .map(|i| content::id(&res, i, "id"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut wanted = ids.to_vec();
    current.sort_unstable();
    wanted.sort_unstable();
    if current != wanted {
        return Err(PanelError::Invalid("The order is not valid."));
    }
    if ids.is_empty() {
        return Ok(());
    }
    let stmts = ids
        .iter()
        .zip(1u32..)
        .map(|(id, position)| Stmt::new(update).bind(position).bind(*id).no_rows())
        .collect();
    client.transaction(stmts).await?;
    Ok(())
}

fn has_duplicates(ids: &[u32]) -> bool {
    let mut seen = ids.to_vec();
    seen.sort_unstable();
    seen.windows(2).any(|w| w[0] == w[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{
        client, execute_error, execute_reply, result, tx_reply, tx_step_error, Script,
    };
    use serde_json::json;

    fn repo(name: &str, topic: Option<&str>) -> RepoBody {
        RepoBody {
            repo_name: name.into(),
            topic: topic.map(Into::into),
        }
    }

    fn ids_reply(ids: &[i64]) -> Result<turso_http::HttpResponse, String> {
        let rows: Vec<_> = ids.iter().map(|i| vec![json!(i)]).collect();
        execute_reply(result(&["id"], &rows))
    }

    #[test]
    fn usernames_follow_the_github_alphabet_and_are_trimmed() {
        assert_eq!(check_username("  moch-1 ").unwrap(), "moch-1");
        for bad in [
            "",
            "   ",
            "a b",
            "a_b",
            "x/y",
            "ünï",
            &"a".repeat(40),
            "a;b",
        ] {
            assert!(check_username(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn repos_are_validated_and_topics_are_cleaned() {
        assert_eq!(
            check_repo(" flexinix ", Some("  Nix \n flakes ")).unwrap(),
            ("flexinix".to_owned(), Some("Nix flakes".to_owned()))
        );
        // Topik kosong atau hanya spasi dianggap tidak ada.
        assert_eq!(check_repo("a", Some("  \t")).unwrap().1, None);
        assert_eq!(check_repo("a", None).unwrap().1, None);
        // Karakter tak terlihat dibuang, bukan disimpan.
        assert_eq!(
            check_repo("a", Some("x\u{202e}y")).unwrap().1.as_deref(),
            Some("xy")
        );
        for bad in ["", " ", "a b", "a/b", "a\"b", &"a".repeat(101), "é"] {
            assert!(check_repo(bad, None).is_err(), "{bad:?}");
        }
        assert!(check_repo("a", Some(&"t".repeat(61))).is_err());
        assert!(check_repo("a", Some(&"t".repeat(60))).is_ok());
    }

    #[test]
    fn stack_names_are_cleaned_and_bounded() {
        assert_eq!(check_stack_name("  Rust  lang ").unwrap(), "Rust lang");
        assert!(check_stack_name("").is_err());
        assert!(check_stack_name(" \n ").is_err());
        assert!(check_stack_name(&"n".repeat(41)).is_err());
        assert!(check_stack_name(&"n".repeat(40)).is_ok());
    }

    #[test]
    fn invalid_input_never_reaches_the_database() {
        futures_executor::block_on(async {
            let s = Script::new(vec![]);
            let c = client(&s);
            assert!(set_username(&c, "bad name").await.is_err());
            assert!(add_repo(&c, &repo("bad name", None)).await.is_err());
            assert!(edit_repo(&c, 1, &repo("", None)).await.is_err());
            assert!(add_stack(&c, &StackBody { name: " ".into() })
                .await
                .is_err());
            assert!(edit_stack(
                &c,
                1,
                &StackBody {
                    name: "x".repeat(41)
                }
            )
            .await
            .is_err());
            assert_eq!(s.requests(), 0);
        });
    }

    #[test]
    fn loading_reads_everything_in_one_transaction() {
        futures_executor::block_on(async {
            let s = Script::new(vec![tx_reply(vec![
                result(&["github_username"], &[vec![json!("moch")]]),
                result(
                    &["id", "repo_name", "topic"],
                    &[
                        vec![json!(2), json!("flexinix"), json!("Nix")],
                        vec![json!(3), json!("dots"), json!(null)],
                    ],
                ),
                result(&["id", "name"], &[vec![json!(1), json!("Rust")]]),
                result(
                    &["id", "fetched_at", "source"],
                    &[vec![
                        json!(9),
                        json!("2026-01-01T00:00:00Z"),
                        json!("github"),
                    ]],
                ),
            ])]);
            let got = load(&client(&s), 0).await.unwrap();
            assert_eq!(got.now, "1970-01-01T00:00:00Z");
            assert_eq!(s.requests(), 1);
            assert_eq!(got.github_username, "moch");
            assert_eq!(got.repos.len(), 2);
            assert_eq!(got.repos[1].topic, None);
            assert_eq!(got.stack[0].name, "Rust");
            assert_eq!(got.snapshots[0].source, "github");
            assert_eq!(got.caps.max_pinned, 12);
            assert_eq!(got.caps.max_stack, 12);
            assert_eq!(
                s.args(0, 4),
                vec![json!({"type": "integer", "value": "20"})]
            );
        });
    }

    #[test]
    fn a_missing_profile_row_is_bad_data() {
        futures_executor::block_on(async {
            let s = Script::new(vec![tx_reply(vec![
                result(&["github_username"], &[]),
                result(&["id", "repo_name", "topic"], &[]),
                result(&["id", "name"], &[]),
                result(&["id", "fetched_at", "source"], &[]),
            ])]);
            assert!(matches!(
                load(&client(&s), 0).await,
                Err(PanelError::BadData(_))
            ));
        });
    }

    #[test]
    fn adding_a_repo_binds_name_topic_and_the_cap() {
        futures_executor::block_on(async {
            let s = Script::new(vec![ids_reply(&[7])]);
            let (id, name) = add_repo(&client(&s), &repo("flexinix", Some("Nix")))
                .await
                .unwrap();
            assert_eq!((id, name.as_str()), (7, "flexinix"));
            let args = s.args(0, 0);
            assert_eq!(args[0], json!({"type": "text", "value": "flexinix"}));
            assert_eq!(args[1], json!({"type": "text", "value": "Nix"}));
            assert_eq!(args[2], json!({"type": "integer", "value": "12"}));

            let s = Script::new(vec![ids_reply(&[8])]);
            add_repo(&client(&s), &repo("x", None)).await.unwrap();
            assert_eq!(s.args(0, 0)[1], json!({"type": "null"}));
        });
    }

    #[test]
    fn a_full_list_and_a_duplicate_name_are_told_apart() {
        futures_executor::block_on(async {
            let s = Script::new(vec![ids_reply(&[])]);
            assert_eq!(
                add_repo(&client(&s), &repo("x", None)).await,
                Err(PanelError::Invalid("The list is full."))
            );
            let s = Script::new(vec![execute_error(
                "UNIQUE constraint failed: journal_pinned_repos.repo_name",
            )]);
            assert_eq!(
                add_repo(&client(&s), &repo("x", None)).await,
                Err(PanelError::Conflict)
            );
            let s = Script::new(vec![execute_error(
                "UNIQUE constraint failed: journal_stack.name",
            )]);
            assert_eq!(
                edit_stack(
                    &client(&s),
                    1,
                    &StackBody {
                        name: "Rust".into()
                    }
                )
                .await,
                Err(PanelError::Conflict)
            );
            // Pelanggaran lain (mis. CHECK) tetap galat database, bukan "sudah ada".
            let s = Script::new(vec![execute_error("CHECK constraint failed: x")]);
            assert!(matches!(
                add_repo(&client(&s), &repo("x", None)).await,
                Err(PanelError::Db(_))
            ));
        });
    }

    #[test]
    fn editing_and_deleting_a_missing_row_is_not_found() {
        futures_executor::block_on(async {
            let s = Script::new(vec![
                ids_reply(&[]),
                ids_reply(&[]),
                ids_reply(&[]),
                ids_reply(&[]),
            ]);
            let c = client(&s);
            assert_eq!(
                edit_repo(&c, 5, &repo("x", None)).await,
                Err(PanelError::NotFound)
            );
            assert_eq!(delete_repo(&c, 5).await, Err(PanelError::NotFound));
            assert_eq!(
                edit_stack(&c, 5, &StackBody { name: "x".into() }).await,
                Err(PanelError::NotFound)
            );
            assert_eq!(delete_stack(&c, 5).await, Err(PanelError::NotFound));
        });
    }

    #[test]
    fn deleting_returns_the_removed_name_for_the_history() {
        futures_executor::block_on(async {
            let s = Script::new(vec![
                execute_reply(result(&["repo_name"], &[vec![json!("flexinix")]])),
                execute_reply(result(&["name"], &[vec![json!("Rust")]])),
            ]);
            let c = client(&s);
            assert_eq!(delete_repo(&c, 2).await.unwrap(), "flexinix");
            assert_eq!(delete_stack(&c, 1).await.unwrap(), "Rust");
            assert_eq!(s.args(0, 0), vec![json!({"type": "integer", "value": "2"})]);
        });
    }

    #[test]
    fn the_username_is_upserted_and_trimmed() {
        futures_executor::block_on(async {
            let s = Script::new(vec![execute_reply(result(
                &["github_username"],
                &[vec![json!("moch")]],
            ))]);
            assert_eq!(set_username(&client(&s), " moch ").await.unwrap(), "moch");
            assert_eq!(s.args(0, 0), vec![json!({"type": "text", "value": "moch"})]);
        });
    }

    #[test]
    fn a_new_order_must_be_a_permutation_of_the_existing_ids() {
        futures_executor::block_on(async {
            for bad in [
                vec![1, 2],       // kurang
                vec![1, 2, 3, 4], // lebih
                vec![1, 1, 2],    // kembar
                vec![1, 2, 9],    // id asing
            ] {
                let s = Script::new(vec![ids_reply(&[1, 2, 3])]);
                assert_eq!(
                    reorder(&client(&s), List::Repos, &bad).await,
                    Err(PanelError::Invalid("The order is not valid.")),
                    "{bad:?}"
                );
                assert!(s.requests() <= 1, "no write for {bad:?}");
            }
            // Duplikat dan kelebihan batas ditolak tanpa membaca database.
            let s = Script::new(vec![]);
            assert!(reorder(&client(&s), List::Stack, &[1, 1]).await.is_err());
            assert!(
                reorder(&client(&s), List::Stack, &(1..=13).collect::<Vec<_>>())
                    .await
                    .is_err()
            );
            assert_eq!(s.requests(), 0);
        });
    }

    #[test]
    fn a_valid_order_writes_positions_one_to_n_in_one_transaction() {
        futures_executor::block_on(async {
            let s = Script::new(vec![
                ids_reply(&[1, 2, 3]),
                tx_reply(vec![result(&[], &[]), result(&[], &[]), result(&[], &[])]),
            ]);
            reorder(&client(&s), List::Repos, &[3, 1, 2]).await.unwrap();
            assert_eq!(s.requests(), 2);
            // Langkah 0 = BEGIN; langkah 1..=3 = UPDATE.
            let pos = |step| s.args(1, step);
            assert_eq!(pos(1)[0], json!({"type": "integer", "value": "1"}));
            assert_eq!(pos(1)[1], json!({"type": "integer", "value": "3"}));
            assert_eq!(pos(2)[0], json!({"type": "integer", "value": "2"}));
            assert_eq!(pos(2)[1], json!({"type": "integer", "value": "1"}));
            assert_eq!(pos(3)[1], json!({"type": "integer", "value": "2"}));
        });
    }

    #[test]
    fn an_empty_list_can_be_reordered_without_writing() {
        futures_executor::block_on(async {
            let s = Script::new(vec![ids_reply(&[])]);
            reorder(&client(&s), List::Stack, &[]).await.unwrap();
            assert_eq!(s.requests(), 1);
        });
    }

    #[test]
    fn a_failed_reorder_transaction_is_a_database_error() {
        futures_executor::block_on(async {
            let s = Script::new(vec![ids_reply(&[1]), tx_step_error(1, 0, "disk I/O error")]);
            assert!(matches!(
                reorder(&client(&s), List::Stack, &[1]).await,
                Err(PanelError::Db(_))
            ));
        });
    }

    #[test]
    fn requests_with_extra_fields_are_rejected() {
        assert!(serde_json::from_str::<RepoBody>(r#"{"repo_name":"a","id":1}"#).is_err());
        assert!(serde_json::from_str::<StackBody>(r#"{"name":"a","x":1}"#).is_err());
        assert!(serde_json::from_str::<OrderBody>(r#"{"ids":[1],"x":1}"#).is_err());
        assert!(serde_json::from_str::<OrderBody>(r#"{"ids":[-1]}"#).is_err());
        assert!(serde_json::from_str::<RepoBody>(r#"{"repo_name":"a"}"#).is_ok());
    }
}
