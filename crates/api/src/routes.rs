//! Handler HTTP untuk endpoint baca konten (D3). Query dan validasinya ada di
//! `content.rs` (dites native); di sini hanya menyambungkan konfigurasi,
//! klien database, dan respons.

use std::future::Future;
use std::sync::Arc;

use api_types::{ChatMessage, Items, NewMessage};
use axum::{
    body::Bytes,
    extract::{rejection::BytesRejection, Path, RawQuery, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    admin,
    chat::{self, ChatError},
    config::Config,
    content::{self, ContentError, Loaded},
    db::{self, WorkerTransport},
    error::AppError,
    github::GithubClient,
    journal::{self, JournalError},
    journal_admin::{self, List, OrderBody, ProfileBody, RepoBody, StackBody},
    moderation,
    panel::{self, Access, PanelError},
    radio_check::{self, RadioError},
    radio_probe::WorkerProbe,
    tables::{self, Table},
};

/// Konten jarang berubah: browser boleh memakai salinan 1 menit dan menyajikan
/// yang basi sambil memperbarui selama 5 menit. Hanya untuk respons sukses;
/// error tidak pernah di-cache (`no-store` dari middleware).
const CONTENT_CACHE: &str = "public, max-age=60, stale-while-revalidate=300";

/// Pesan error umum; detail hanya ke log Worker.
pub fn app_error(e: &ContentError) -> AppError {
    use turso_http::Error;
    match e {
        // Database tidak terjangkau / menolak koneksi: sementara.
        ContentError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE
        }
        // SQL gagal, protokol aneh, atau data berbentuk salah: bug/skema.
        ContentError::Db(_) | ContentError::BadData(_) => AppError::INTERNAL,
    }
}

fn ok<T: Serialize>(items: Vec<T>) -> Response {
    let mut res = Json(Items { items }).into_response();
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CONTENT_CACHE),
    );
    res
}

async fn respond<T, Fut>(
    config: &Config,
    name: &'static str,
    run: impl FnOnce(turso_http::Client<WorkerTransport>) -> Fut,
) -> Result<Response, AppError>
where
    T: Serialize,
    Fut: Future<Output = Result<Loaded<T>, ContentError>>,
{
    let Some(cfg) = &config.db else {
        return Err(AppError::UNAVAILABLE);
    };
    let client = db::client(cfg).map_err(|e| {
        worker::console_error!("{name}: invalid database config: {e}");
        AppError::UNAVAILABLE
    })?;
    match run(client).await {
        Ok(loaded) => {
            if loaded.skipped > 0 {
                worker::console_error!("{name}: skipped {} invalid row(s)", loaded.skipped);
            }
            Ok(ok(loaded.items))
        }
        Err(e) => {
            worker::console_error!("{name}: {e}");
            Err(app_error(&e))
        }
    }
}

// `#[worker::send]`: future di wasm32 tidak `Send`, sedangkan axum memintanya.

#[worker::send]
pub async fn generations(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(&config, "generations", |c| async move {
        content::generations(&c).await
    })
    .await
}

#[worker::send]
pub async fn bookmarks(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(&config, "bookmarks", |c| async move {
        content::bookmarks(&c).await
    })
    .await
}

#[worker::send]
pub async fn radio(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(
        &config,
        "radio",
        |c| async move { content::radio(&c).await },
    )
    .await
}

#[worker::send]
pub async fn embiences(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(&config, "embiences", |c| async move {
        content::embiences(&c).await
    })
    .await
}

#[worker::send]
pub async fn presets(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(
        &config,
        "presets",
        |c| async move { content::presets(&c).await },
    )
    .await
}

// ------------------------------------------------------------ Guestbook (D4)

/// Pesan error umum; detail hanya ke log Worker.
pub fn chat_error(e: &ChatError) -> Response {
    use turso_http::Error;
    match e {
        ChatError::RateLimited { retry_after_secs } => {
            let mut res = AppError::RATE_LIMITED.into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
            res
        }
        ChatError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE.into_response()
        }
        ChatError::Db(_) | ChatError::BadData(_) => AppError::INTERNAL.into_response(),
    }
}

fn db_client(
    config: &Config,
    name: &'static str,
) -> Result<turso_http::Client<WorkerTransport>, AppError> {
    let Some(cfg) = &config.db else {
        return Err(AppError::UNAVAILABLE);
    };
    db::client(cfg).map_err(|e| {
        worker::console_error!("{name}: invalid database config: {e}");
        AppError::UNAVAILABLE
    })
}

/// `GET /api/v1/messages?after=<id>|before=<id>&limit=<n>`. Tidak pernah di-cache:
/// entri yang baru dihapus admin harus segera hilang dari pemuatan berikutnya.
#[worker::send]
pub async fn messages_get(
    State(config): State<Arc<Config>>,
    RawQuery(query): RawQuery,
) -> Result<Response, AppError> {
    let q = chat::parse_list_query(query.as_deref())
        .ok_or(AppError::bad_request("Invalid query parameters."))?;
    let client = db_client(&config, "messages")?;
    match chat::list(&client, &q).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => {
            worker::console_error!("messages: {e}");
            Ok(chat_error(&e))
        }
    }
}

/// `true` untuk `application/json` (boleh diikuti `; charset=...`).
fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|m| m == "application/json")
}

/// Browser yang menyebut `Origin` harus berasal dari daftar yang diizinkan.
/// Tanpa `Origin` (curl, server lain) diteruskan: pembatas kecepatan berlaku
/// sama untuk semua.
fn origin_ok(allowed: &[String], headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN) {
        None => true,
        Some(v) => v
            .to_str()
            .map(|o| allowed.iter().any(|a| a == o))
            .unwrap_or(false),
    }
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get(HeaderName::from_static("cf-connecting-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty())
        // Saat `wrangler dev` header ini tidak ada: satu ember bersama.
        .unwrap_or_else(|| "local".to_owned())
}

/// `POST /api/v1/messages`. Urutan sengaja murah dulu, database terakhir:
/// origin, tipe konten, isi body, validasi, baru pembatas kecepatan dan simpan.
#[worker::send]
pub async fn messages_post(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    if !origin_ok(&config.allowed_origins, &headers) {
        return Err(AppError::FORBIDDEN);
    }
    if !is_json(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    ) {
        return Err(AppError::bad_request(
            "Content-Type must be application/json.",
        ));
    }
    let body = body.map_err(|e| {
        if e.into_response().status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PAYLOAD_TOO_LARGE
        } else {
            AppError::bad_request("Could not read the request body.")
        }
    })?;
    let new: NewMessage = serde_json::from_slice(&body)
        .map_err(|_| AppError::bad_request("The request body is not valid."))?;
    let clean = chat::prepare(&new, &config.banned_words)
        .map_err(|reject| AppError::bad_request(reject.message()))?;

    let client = db_client(&config, "messages")?;
    let ip = client_ip(&headers);
    let sender = chat::Sender {
        ip: &ip,
        salt: config.rate_salt.expose(),
        now_secs: now_secs(),
    };
    match chat::submit(&client, &sender, &clean).await {
        Ok(message) => Ok(created(message)),
        Err(e) => {
            if !matches!(e, ChatError::RateLimited { .. }) {
                worker::console_error!("messages: {e}");
            }
            Ok(chat_error(&e))
        }
    }
}

fn created(message: ChatMessage) -> Response {
    (StatusCode::CREATED, Json(message)).into_response()
}

/// Id di path -> `u32` >= 1. Diambil sebagai teks lalu diperiksa sendiri supaya
/// yang salah tetap berupa error JSON kita (bukan penolakan teks polos Axum).
fn parse_id(raw: &str) -> Result<u32, AppError> {
    let digits = !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit());
    raw.parse::<u32>()
        .ok()
        .filter(|n| digits && *n >= 1)
        .ok_or(AppError::bad_request("Invalid entry id."))
}

fn now_secs() -> i64 {
    i64::try_from(worker::Date::now().as_millis() / 1000).unwrap_or(0)
}

/// `POST /api/v1/messages/{id}/report`. Aturan yang sama dengan menulis entri:
/// origin yang diizinkan dan `application/json` (memaksa preflight CORS, jadi
/// situs lain tidak bisa menyuruh browser pengunjung melapor diam-diam).
/// Jawabannya selalu `204` kalau permintaannya sah, apa pun yang terjadi pada
/// entrinya; lihat `chat::report`.
#[worker::send]
pub async fn message_report(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !origin_ok(&config.allowed_origins, &headers) {
        return Err(AppError::FORBIDDEN);
    }
    if !is_json(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    ) {
        return Err(AppError::bad_request(
            "Content-Type must be application/json.",
        ));
    }
    let id = parse_id(&id)?;
    let client = db_client(&config, "report")?;
    let ip = client_ip(&headers);
    let sender = chat::Sender {
        ip: &ip,
        salt: config.rate_salt.expose(),
        now_secs: now_secs(),
    };
    match chat::report(&client, &sender, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(e) => {
            if !matches!(e, ChatError::RateLimited { .. }) {
                worker::console_error!("report: {e}");
            }
            Ok(chat_error(&e))
        }
    }
}

// ------------------------------------------------------------ Admin (D4b, D6a)

/// Pesan error umum; detail hanya ke log Worker.
pub fn panel_error(e: &PanelError) -> AppError {
    use turso_http::Error;
    match e {
        PanelError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE
        }
        PanelError::Db(_) | PanelError::BadData(_) => AppError::INTERNAL,
        PanelError::Invalid(why) => AppError::bad_request(why),
        PanelError::NotFound => AppError::NOT_FOUND,
        PanelError::Conflict => AppError::CONFLICT,
    }
}

fn too_many(retry_after_secs: u32) -> Response {
    let mut res = AppError::RATE_LIMITED.into_response();
    if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        res.headers_mut().insert(header::RETRY_AFTER, v);
    }
    res
}

/// Permintaan admin yang sudah lolos gerbang.
struct Admin {
    client: turso_http::Client<WorkerTransport>,
    /// Hash IP pelaku (kunci pembatas dan baris riwayat).
    who: String,
    now: i64,
}

/// Gerbang semua endpoint admin: token (`admin::authorized`), lalu pembatas
/// percobaan gagal (`panel::check_access`). Urutannya sengaja begini:
///
/// - Admin mati (token kosong atau pendek): 401 tanpa menyentuh database.
/// - Token salah dan database tidak bisa dipakai: tetap 401 (gagal tertutup).
/// - Token benar tetapi database tidak bisa dipakai: 503.
/// - Terkunci: 429 + `Retry-After`, juga untuk token yang benar.
async fn admin_gate(
    config: &Config,
    headers: &HeaderMap,
    name: &'static str,
) -> Result<Admin, Response> {
    if !admin::enabled(&config.admin_token) {
        return Err(AppError::UNAUTHORIZED.into_response());
    }
    let authorized = admin::authorized(&config.admin_token, authorization(headers));
    let client = match db_client(config, name) {
        Ok(c) => c,
        Err(e) if authorized => return Err(e.into_response()),
        Err(_) => return Err(AppError::UNAUTHORIZED.into_response()),
    };
    let who = chat::ip_key(&client_ip(headers), config.rate_salt.expose());
    let now = now_secs();
    match panel::check_access(&client, &who, now, authorized).await {
        Ok(Access::Granted) => Ok(Admin { client, who, now }),
        Ok(Access::Denied) => Err(AppError::UNAUTHORIZED.into_response()),
        Ok(Access::Locked { retry_after_secs }) => Err(too_many(retry_after_secs)),
        Err(e) => {
            worker::console_error!("{name}: admin gate: {e}");
            Err(if authorized {
                panel_error(&e).into_response()
            } else {
                AppError::UNAUTHORIZED.into_response()
            })
        }
    }
}

/// Memasang gerbang; kalau ditolak, mengembalikan jawabannya dari handler.
macro_rules! gate {
    ($config:expr, $headers:expr, $name:expr) => {
        match admin_gate($config, $headers, $name).await {
            Ok(admin) => admin,
            Err(res) => return Ok(res),
        }
    };
}

/// Mencatat aksi yang sudah berhasil. Gagal mencatat tidak membatalkan aksinya;
/// hanya masuk log.
async fn audit(admin: &Admin, action: &str, target: Option<&str>) {
    if let Err(e) = panel::record(&admin.client, &admin.who, admin.now, action, target).await {
        worker::console_error!("audit {action}: {e}");
    }
}

/// Header `Authorization` mentah, kalau ada dan berupa teks.
fn authorization(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
}

async fn overview_response(config: &Config, admin: &Admin) -> Result<Response, AppError> {
    let ctx = panel::Context {
        who: &admin.who,
        now_secs: admin.now,
        github_configured: !config.github_token.expose().is_empty(),
    };
    match panel::overview(&admin.client, &ctx).await {
        Ok(o) => Ok(Json(o).into_response()),
        Err(e) => {
            worker::console_error!("admin overview: {e}");
            Ok(panel_error(&e).into_response())
        }
    }
}

/// `POST /api/v1/admin/login`: memeriksa token, mencatat "login", menjawab ringkasan.
#[worker::send]
pub async fn admin_login(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin login");
    audit(&admin, "login", None).await;
    overview_response(&config, &admin).await
}

/// `GET /api/v1/admin/overview`: ringkasan untuk dashboard.
#[worker::send]
pub async fn admin_overview(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin overview");
    overview_response(&config, &admin).await
}

/// `GET /api/v1/admin/audit?before=<id>&limit=<n>`: riwayat aksi admin.
#[worker::send]
pub async fn admin_audit(
    State(config): State<Arc<Config>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin audit");
    let q = panel::parse_audit_query(query.as_deref())
        .ok_or(AppError::bad_request("Invalid query parameters."))?;
    match panel::audit_list(&admin.client, &q).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => {
            worker::console_error!("admin audit: {e}");
            Ok(panel_error(&e).into_response())
        }
    }
}

/// `GET /api/v1/admin/reports`: entri yang dilaporkan, paling banyak dulu.
#[worker::send]
pub async fn admin_reports(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin reports");
    match chat::reported(&admin.client).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => {
            worker::console_error!("admin reports: {e}");
            Ok(chat_error(&e))
        }
    }
}

/// `DELETE /api/v1/admin/messages/{id}`: menyembunyikan entri (baris tetap ada).
/// `204` jika disembunyikan, `404` jika tidak ada atau sudah tersembunyi.
#[worker::send]
pub async fn admin_delete(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin delete");
    let id = parse_id(&id)?;
    match chat::hide(&admin.client, id, admin.now).await {
        Ok(true) => {
            audit(&admin, "message.hide", Some(&format!("message:{id}"))).await;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Ok(false) => Err(AppError::NOT_FOUND),
        Err(e) => {
            worker::console_error!("admin delete: {e}");
            Ok(chat_error(&e))
        }
    }
}

// ------------------------------------------- Journal dan moderasi (D6b)

/// Isi JSON dari permintaan admin: tipe konten, batas ukuran, dan bentuk diperiksa
/// dengan pesan tetap. Dipanggil SETELAH gerbang, jadi yang belum masuk tidak
/// pernah sampai ke sini. `Origin` tidak diperiksa: dashboard satu origin dengan
/// API dan yang membuka pintunya adalah token, bukan asal permintaan.
fn admin_json<B: DeserializeOwned>(
    headers: &HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<B, AppError> {
    if !is_json(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    ) {
        return Err(AppError::bad_request(
            "Content-Type must be application/json.",
        ));
    }
    let body = body.map_err(|e| {
        if e.into_response().status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PAYLOAD_TOO_LARGE
        } else {
            AppError::bad_request("Could not read the request body.")
        }
    })?;
    serde_json::from_slice(&body)
        .map_err(|_| AppError::bad_request("The request body is not valid."))
}

/// Jawaban galat untuk operasi admin; hanya kegagalan sungguhan yang masuk log
/// (masukan salah, tidak ada, dan kembar adalah hal biasa).
fn panel_fail(what: &str, e: &PanelError) -> Response {
    if matches!(e, PanelError::Db(_) | PanelError::BadData(_)) {
        worker::console_error!("{what}: {e}");
    }
    panel_error(e).into_response()
}

#[derive(Serialize)]
struct Created {
    id: u32,
}

#[derive(Serialize)]
struct Username {
    github_username: String,
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// `GET /api/v1/admin/journal`: akun, repo, stack, dan snapshot terbaru.
#[worker::send]
pub async fn admin_journal(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin journal");
    match journal_admin::load(&admin.client, admin.now).await {
        Ok(settings) => Ok(Json(settings).into_response()),
        Err(e) => Ok(panel_fail("admin journal", &e)),
    }
}

/// `PUT /api/v1/admin/journal/profile`: mengganti akun GitHub.
#[worker::send]
pub async fn admin_journal_profile(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin journal profile");
    let body: ProfileBody = admin_json(&headers, body)?;
    match journal_admin::set_username(&admin.client, &body.github_username).await {
        Ok(name) => {
            audit(&admin, "journal.profile", Some(&format!("github:{name}"))).await;
            Ok(Json(Username {
                github_username: name,
            })
            .into_response())
        }
        Err(e) => Ok(panel_fail("admin journal profile", &e)),
    }
}

/// `POST /api/v1/admin/journal/repos`: menambah repo sematan.
#[worker::send]
pub async fn admin_journal_repo_add(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin repo add");
    let body: RepoBody = admin_json(&headers, body)?;
    match journal_admin::add_repo(&admin.client, &body).await {
        Ok((id, name)) => {
            audit(&admin, "journal.repo.add", Some(&format!("repo:{name}"))).await;
            Ok((StatusCode::CREATED, Json(Created { id })).into_response())
        }
        Err(e) => Ok(panel_fail("admin repo add", &e)),
    }
}

/// `PUT /api/v1/admin/journal/repos/{id}`: mengganti nama/topik repo.
#[worker::send]
pub async fn admin_journal_repo_edit(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin repo edit");
    let id = parse_id(&id)?;
    let body: RepoBody = admin_json(&headers, body)?;
    match journal_admin::edit_repo(&admin.client, id, &body).await {
        Ok(name) => {
            audit(&admin, "journal.repo.edit", Some(&format!("repo:{name}"))).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin repo edit", &e)),
    }
}

/// `DELETE /api/v1/admin/journal/repos/{id}`.
#[worker::send]
pub async fn admin_journal_repo_delete(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin repo delete");
    let id = parse_id(&id)?;
    match journal_admin::delete_repo(&admin.client, id).await {
        Ok(name) => {
            audit(&admin, "journal.repo.delete", Some(&format!("repo:{name}"))).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin repo delete", &e)),
    }
}

/// `PUT /api/v1/admin/journal/repos/order`.
#[worker::send]
pub async fn admin_journal_repos_order(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin repo order");
    let body: OrderBody = admin_json(&headers, body)?;
    match journal_admin::reorder(&admin.client, List::Repos, &body.ids).await {
        Ok(()) => {
            audit(&admin, "journal.repo.order", None).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin repo order", &e)),
    }
}

/// `POST /api/v1/admin/journal/stack`.
#[worker::send]
pub async fn admin_journal_stack_add(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin stack add");
    let body: StackBody = admin_json(&headers, body)?;
    match journal_admin::add_stack(&admin.client, &body).await {
        Ok((id, name)) => {
            audit(&admin, "journal.stack.add", Some(&format!("stack:{name}"))).await;
            Ok((StatusCode::CREATED, Json(Created { id })).into_response())
        }
        Err(e) => Ok(panel_fail("admin stack add", &e)),
    }
}

/// `PUT /api/v1/admin/journal/stack/{id}`.
#[worker::send]
pub async fn admin_journal_stack_edit(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin stack edit");
    let id = parse_id(&id)?;
    let body: StackBody = admin_json(&headers, body)?;
    match journal_admin::edit_stack(&admin.client, id, &body).await {
        Ok(name) => {
            audit(&admin, "journal.stack.edit", Some(&format!("stack:{name}"))).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin stack edit", &e)),
    }
}

/// `DELETE /api/v1/admin/journal/stack/{id}`.
#[worker::send]
pub async fn admin_journal_stack_delete(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin stack delete");
    let id = parse_id(&id)?;
    match journal_admin::delete_stack(&admin.client, id).await {
        Ok(name) => {
            audit(
                &admin,
                "journal.stack.delete",
                Some(&format!("stack:{name}")),
            )
            .await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin stack delete", &e)),
    }
}

/// `PUT /api/v1/admin/journal/stack/order`.
#[worker::send]
pub async fn admin_journal_stack_order(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin stack order");
    let body: OrderBody = admin_json(&headers, body)?;
    match journal_admin::reorder(&admin.client, List::Stack, &body.ids).await {
        Ok(()) => {
            audit(&admin, "journal.stack.order", None).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin stack order", &e)),
    }
}

/// `GET /api/v1/admin/messages?filter=&before=&limit=`: entri untuk moderator.
#[worker::send]
pub async fn admin_messages(
    State(config): State<Arc<Config>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin messages");
    let q = moderation::parse_query(query.as_deref())
        .ok_or(AppError::bad_request("Invalid query parameters."))?;
    match moderation::list(&admin.client, &q).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => Ok(panel_fail("admin messages", &e)),
    }
}

/// `POST /api/v1/admin/messages/{id}/restore`: menampilkan lagi entri.
#[worker::send]
pub async fn admin_message_restore(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin restore");
    let id = parse_id(&id)?;
    match moderation::restore(&admin.client, id).await {
        Ok(()) => {
            audit(&admin, "message.restore", Some(&format!("message:{id}"))).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin restore", &e)),
    }
}

/// `DELETE /api/v1/admin/messages/{id}/reports`: menolak semua laporan entri.
#[worker::send]
pub async fn admin_message_dismiss(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin dismiss");
    let id = parse_id(&id)?;
    match moderation::dismiss_reports(&admin.client, id).await {
        Ok(()) => {
            audit(&admin, "message.dismiss", Some(&format!("message:{id}"))).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin dismiss", &e)),
    }
}

// ------------------------------------------------------ Editor data (D6c)

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PurgeBody {
    confirm: String,
}

/// Tabel dari daftar putih; apa pun di luarnya 404 (tidak membocorkan nama tabel lain).
fn known_table(name: &str) -> Result<&'static Table, AppError> {
    tables::find(name).ok_or(AppError::NOT_FOUND)
}

/// `GET /api/v1/admin/tables`: tabel yang boleh dikelola, skema, dan jumlah baris.
#[worker::send]
pub async fn admin_tables(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin tables");
    match tables::overview(&admin.client).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => Ok(panel_fail("admin tables", &e)),
    }
}

/// `GET /api/v1/admin/tables/{table}/rows?before=<kunci>&limit=<n>`.
#[worker::send]
pub async fn admin_table_rows(
    State(config): State<Arc<Config>>,
    Path(table): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin rows");
    let table = known_table(&table)?;
    let q = panel::parse_audit_query(query.as_deref())
        .ok_or(AppError::bad_request("Invalid query parameters."))?;
    match tables::list(&admin.client, table, q.before, q.limit).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => Ok(panel_fail("admin rows", &e)),
    }
}

/// `POST /api/v1/admin/tables/{table}/rows`: menambah baris → `201 {"id"}`.
#[worker::send]
pub async fn admin_table_insert(
    State(config): State<Arc<Config>>,
    Path(table): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin insert");
    let table = known_table(&table)?;
    let body: serde_json::Map<String, serde_json::Value> = admin_json(&headers, body)?;
    match tables::insert(&admin.client, table, &body).await {
        Ok(id) => {
            audit(
                &admin,
                "table.insert",
                Some(&format!("{}:{id}", table.name)),
            )
            .await;
            Ok((StatusCode::CREATED, Json(Created { id })).into_response())
        }
        Err(e) => Ok(panel_fail("admin insert", &e)),
    }
}

/// `PUT /api/v1/admin/tables/{table}/rows/{id}`: mengubah baris → `204`.
#[worker::send]
pub async fn admin_table_update(
    State(config): State<Arc<Config>>,
    Path((table, id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin update");
    let table = known_table(&table)?;
    let id = parse_id(&id)?;
    let body: serde_json::Map<String, serde_json::Value> = admin_json(&headers, body)?;
    match tables::update(&admin.client, table, id, &body).await {
        Ok(()) => {
            audit(
                &admin,
                "table.update",
                Some(&format!("{}:{id}", table.name)),
            )
            .await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin update", &e)),
    }
}

/// `DELETE /api/v1/admin/tables/{table}/rows/{id}` → `204`.
#[worker::send]
pub async fn admin_table_delete(
    State(config): State<Arc<Config>>,
    Path((table, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin row delete");
    let table = known_table(&table)?;
    let id = parse_id(&id)?;
    match tables::delete(&admin.client, table, id).await {
        Ok(()) => {
            audit(
                &admin,
                "table.delete",
                Some(&format!("{}:{id}", table.name)),
            )
            .await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin row delete", &e)),
    }
}

/// `GET /api/v1/admin/tables/{table}/export?format=json|csv`: unduhan berkas.
#[worker::send]
pub async fn admin_table_export(
    State(config): State<Arc<Config>>,
    Path(table): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin export");
    let table = known_table(&table)?;
    let format = tables::parse_format(query.as_deref())
        .ok_or(AppError::bad_request("Invalid query parameters."))?;
    match tables::export(&admin.client, table, format, admin.now).await {
        Ok(file) => {
            audit(&admin, "table.export", Some(table.name)).await;
            let mut res = file.body.into_response();
            let h = res.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(file.content_type),
            );
            // Nama berkas berasal dari daftar putih (huruf kecil dan `_`), aman di header.
            if let Ok(v) =
                HeaderValue::from_str(&format!("attachment; filename=\"{}\"", file.filename))
            {
                h.insert(header::CONTENT_DISPOSITION, v);
            }
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            Ok(res)
        }
        Err(e) => Ok(panel_fail("admin export", &e)),
    }
}

/// `GET /api/v1/admin/presets/items`: isi semua preset.
#[worker::send]
pub async fn admin_preset_items(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin preset items");
    match tables::preset_items(&admin.client).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => Ok(panel_fail("admin preset items", &e)),
    }
}

/// `PUT /api/v1/admin/presets/{id}/items`: mengganti isi satu preset → `204`.
#[worker::send]
pub async fn admin_preset_items_set(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin preset set");
    let id = parse_id(&id)?;
    let body: OrderBody = admin_json(&headers, body)?;
    match tables::set_preset_items(&admin.client, id, &body.ids).await {
        Ok(()) => {
            audit(
                &admin,
                "preset.items",
                Some(&format!("embience_presets:{id}")),
            )
            .await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin preset set", &e)),
    }
}

/// `POST /api/v1/admin/messages/{id}/purge`: hapus permanen entri yang sudah
/// disembunyikan. Wajib `{"confirm":"purge:<id>"}` → `204`.
#[worker::send]
pub async fn admin_message_purge(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin purge");
    let id = parse_id(&id)?;
    let body: PurgeBody = admin_json(&headers, body)?;
    match tables::purge_message(&admin.client, id, &body.confirm).await {
        Ok(()) => {
            audit(&admin, "message.purge", Some(&format!("message:{id}"))).await;
            Ok(no_content())
        }
        Err(e) => Ok(panel_fail("admin purge", &e)),
    }
}

// ------------------------------------------------------------ Journal (D5)

/// Respons jurnal berubah paling cepat tiap refresh (cron 6 jam): browser
/// boleh memakai salinan 5 menit.
const JOURNAL_CACHE: &str = "public, max-age=300, stale-while-revalidate=600";

/// Pesan error umum; detail hanya ke log Worker.
pub fn journal_error(e: &JournalError) -> Response {
    use turso_http::Error;
    match e {
        JournalError::TooSoon { retry_after_secs } => {
            let mut res = AppError::RATE_LIMITED.into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
            res
        }
        JournalError::NotConfigured(_) => AppError::NOT_CONFIGURED.into_response(),
        JournalError::Upstream(_) => AppError::UPSTREAM.into_response(),
        JournalError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE.into_response()
        }
        JournalError::Db(_) | JournalError::BadData(_) => AppError::INTERNAL.into_response(),
    }
}

/// `GET /api/v1/journal`: snapshot terbaru ([`api_types::journal::Snapshot`]).
#[worker::send]
pub async fn journal_get(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    let client = db_client(&config, "journal")?;
    match journal::latest(&client).await {
        Ok(Some(snapshot)) => {
            let mut res = Json(snapshot).into_response();
            res.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(JOURNAL_CACHE),
            );
            Ok(res)
        }
        Ok(None) => Err(AppError::NOT_READY),
        Err(e) => {
            worker::console_error!("journal: {e}");
            Ok(journal_error(&e))
        }
    }
}

/// `POST /api/v1/admin/journal/refresh`: ambil dari GitHub sekarang, tanpa
/// menunggu cron. Hanya admin; paling cepat sekali per menit.
#[worker::send]
pub async fn admin_journal_refresh(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "journal refresh");
    if config.github_token.expose().is_empty() {
        return Err(AppError::NOT_CONFIGURED);
    }
    let github = GithubClient::new(config.github_token.expose());
    match journal::refresh_if_due(&admin.client, &github, admin.now).await {
        Ok(done) => {
            audit(&admin, "journal.refresh", None).await;
            Ok(Json(done).into_response())
        }
        Err(e) => {
            if !matches!(e, JournalError::TooSoon { .. }) {
                worker::console_error!("journal refresh: {e}");
            }
            Ok(journal_error(&e))
        }
    }
}

/// Pesan error umum; detail hanya ke log Worker.
pub fn radio_error(e: &RadioError) -> Response {
    use turso_http::Error;
    match e {
        RadioError::TooSoon { retry_after_secs } => {
            too_many(u32::try_from(*retry_after_secs).unwrap_or(u32::MAX))
        }
        RadioError::NotConfigured(_) => AppError::NOT_CONFIGURED.into_response(),
        RadioError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE.into_response()
        }
        RadioError::Db(_) | RadioError::BadData(_) => AppError::INTERNAL.into_response(),
    }
}

/// `GET /api/v1/admin/radio`: status pemeriksaan semua stasiun (tanpa URL).
#[worker::send]
pub async fn admin_radio(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin radio");
    match radio_check::status(&admin.client, admin.now).await {
        Ok(status) => Ok(Json(status).into_response()),
        Err(e) => {
            worker::console_error!("admin radio: {e}");
            Ok(radio_error(&e))
        }
    }
}

/// `POST /api/v1/admin/radio/check`: periksa stasiun aktif sekarang. Hanya
/// admin; paling cepat sekali per menit.
#[worker::send]
pub async fn admin_radio_check(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let admin = gate!(&config, &headers, "admin radio check");
    match radio_check::check_now(&admin.client, &WorkerProbe, admin.now).await {
        Ok(report) => {
            audit(&admin, "radio.check", None).await;
            worker::console_log!("radio (manual): {report}");
            Ok(Json(report).into_response())
        }
        Err(e) => {
            if !matches!(e, RadioError::TooSoon { .. }) {
                worker::console_error!("admin radio check: {e}");
            }
            Ok(radio_error(&e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use turso_http::Error;

    #[test]
    fn database_outages_are_503_and_bugs_are_500() {
        for e in [
            Error::Config("x".into()),
            Error::Transport("offline".into()),
            Error::Http {
                status: 401,
                message: "Unauthorized".into(),
            },
        ] {
            assert_eq!(app_error(&ContentError::Db(e)), AppError::UNAVAILABLE);
        }
        for e in [
            ContentError::Db(Error::Sql {
                message: "no such table".into(),
                code: None,
            }),
            ContentError::Db(Error::Protocol("x".into())),
            ContentError::BadData("id"),
        ] {
            assert_eq!(app_error(&e), AppError::INTERNAL);
        }
    }

    #[test]
    fn client_messages_never_echo_internal_details() {
        let e = app_error(&ContentError::Db(Error::Sql {
            message: "no such table: radio_stations".into(),
            code: None,
        }));
        assert!(!e.message.contains("radio_stations"));
        assert_eq!(e.message, "Something went wrong.");
    }

    #[test]
    fn entry_ids_in_paths_must_be_plain_positive_numbers() {
        assert_eq!(parse_id("42"), Ok(42));
        assert_eq!(parse_id("1"), Ok(1));
        for bad in [
            "0",
            "",
            "-1",
            "+1",
            "1.5",
            "x",
            "4294967296",
            " 7",
            "7 ",
            "0x10",
        ] {
            assert!(parse_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn journal_errors_map_to_safe_statuses() {
        let status = |e: JournalError| journal_error(&e).status();
        assert_eq!(
            status(JournalError::TooSoon {
                retry_after_secs: 5
            }),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            status(JournalError::NotConfigured("GITHUB_TOKEN")),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status(JournalError::Upstream("HTTP 401".into())),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            status(JournalError::Db(Error::Transport("x".into()))),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status(JournalError::BadData("payload")),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        let retry = journal_error(&JournalError::TooSoon {
            retry_after_secs: 42,
        });
        assert_eq!(retry.headers()[header::RETRY_AFTER], "42");
    }

    #[test]
    fn success_responses_are_cacheable_json_lists() {
        let res = ok(vec![1, 2]);
        assert_eq!(res.headers()[header::CACHE_CONTROL], CONTENT_CACHE);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    }
}
