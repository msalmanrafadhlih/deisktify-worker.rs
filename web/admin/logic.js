// Logika murni dashboard admin: tanpa DOM, tanpa fetch, supaya bisa dites di Node
// (scripts/admin-ui.test.mjs). Semua teks yang tampil ke admin ada di sini atau
// di app.js, dalam bahasa Indonesia biasa.

/** Batas diam sebelum dashboard terkunci sendiri (milidetik). */
export const IDLE_LIMIT_MS = 10 * 60 * 1000;

const TABLE_LABELS = {
  generations: "Generasi boot",
  bookmarks: "Bookmark",
  radio_stations: "Stasiun radio",
  embiences: "Suara latar",
  embience_presets: "Preset suara latar",
  embience_preset_items: "Isi preset suara latar",
  chat_messages: "Entri Guestbook",
  message_reports: "Laporan Guestbook",
  journal_snapshots: "Snapshot journal",
  journal_pinned_repos: "Repo sematan journal",
  journal_stack: "Stack journal",
  admin_audit: "Riwayat admin",
};

const ACTION_LABELS = {
  login: "Masuk",
  "journal.refresh": "Refresh journal dari GitHub",
  "message.hide": "Sembunyikan entri Guestbook",
  "message.restore": "Tampilkan lagi entri Guestbook",
  "message.dismiss": "Tolak laporan entri Guestbook",
  "journal.profile": "Ganti akun GitHub journal",
  "journal.repo.add": "Tambah repo sematan",
  "journal.repo.edit": "Ubah repo sematan",
  "journal.repo.delete": "Hapus repo sematan",
  "journal.repo.order": "Ubah urutan repo sematan",
  "journal.stack.add": "Tambah item stack",
  "journal.stack.edit": "Ubah item stack",
  "journal.stack.delete": "Hapus item stack",
  "journal.stack.order": "Ubah urutan stack",
};

export function tableLabel(name) {
  return TABLE_LABELS[name] ?? String(name);
}

export function actionLabel(action) {
  return ACTION_LABELS[action] ?? String(action);
}

/** `target` dari server berbentuk `jenis:id`, mis. `message:12`. */
export function targetLabel(target) {
  if (target == null || target === "") return "";
  const m = /^(message|repo|stack|github):(.+)$/.exec(target);
  if (!m) return String(target);
  const kind = { message: "Entri #", repo: "Repo ", stack: "Stack ", github: "Akun " }[m[1]];
  return kind + m[2];
}

export function whoLabel(who, me) {
  return who === me ? "Perangkat ini" : `Lainnya (${who})`;
}

/** "Menunggu" yang enak dibaca untuk hitungan detik dari `Retry-After`. */
export function formatWait(secs) {
  if (!Number.isFinite(secs) || secs <= 0) return "";
  if (secs < 60) return `${Math.ceil(secs)} detik`;
  return `${Math.ceil(secs / 60)} menit`;
}

/** Nilai header `Retry-After` (detik, bilangan bulat) atau `null`. */
export function parseRetryAfter(value) {
  if (value == null) return null;
  const s = String(value).trim();
  return /^\d{1,6}$/.test(s) ? Number(s) : null;
}

/**
 * Pesan untuk admin dari error API. Sengaja tidak menampilkan teks dari server
 * (selalu Inggris dan umum); yang dipakai hanya status.
 */
export function describeError(err) {
  const status = err && typeof err.status === "number" ? err.status : 0;
  switch (status) {
    case 0:
      return "Tidak bisa menghubungi server. Periksa koneksi internet.";
    case 400:
      return "Permintaan ditolak server. Periksa isiannya.";
    case 404:
      return "Datanya tidak ditemukan (mungkin sudah berubah). Muat ulang daftar.";
    case 409:
      return "Nilai itu sudah ada.";
    case 413:
      return "Isiannya terlalu besar.";
    case 401:
      return "Token salah, atau sesi sudah berakhir.";
    case 429: {
      const wait = formatWait(err.retryAfter);
      return wait
        ? `Terlalu banyak percobaan. Coba lagi dalam ${wait}.`
        : "Terlalu banyak percobaan. Coba lagi nanti.";
    }
    case 502:
      return "Gagal menghubungi layanan luar (GitHub).";
    case 503:
      return "Server atau database belum bisa dipakai (atau token GitHub belum dipasang). Coba lagi sebentar.";
    default:
      return "Terjadi kesalahan di server.";
  }
}

/** Selisih waktu dalam bahasa Indonesia. `nowIso` dari server (bebas selisih jam perangkat). */
export function timeAgo(iso, nowIso) {
  if (iso == null) return "belum pernah";
  const then = Date.parse(iso);
  const now = Date.parse(nowIso);
  if (Number.isNaN(then) || Number.isNaN(now)) return "tidak diketahui";
  const secs = Math.floor((now - then) / 1000);
  if (secs < 60) return "baru saja";
  if (secs < 3600) return `${Math.floor(secs / 60)} menit lalu`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} jam lalu`;
  return `${Math.floor(secs / 86400)} hari lalu`;
}

/** Tanggal dan jam setempat; teks aslinya kalau tidak bisa dibaca. */
export function formatLocal(iso, timeZone) {
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return String(iso);
  const options = { dateStyle: "medium", timeStyle: "short" };
  if (timeZone) options.timeZone = timeZone;
  return new Date(ms).toLocaleString("id-ID", options);
}

export function isIdle(lastActiveMs, nowMs, limitMs = IDLE_LIMIT_MS) {
  return nowMs - lastActiveMs >= limitMs;
}

const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const count = (v) => (Number.isSafeInteger(v) && v >= 0 ? v : null);
const text = (v) => (typeof v === "string" ? v : null);

/**
 * Memeriksa bentuk ringkasan dari server. Server dipercaya, tetapi halaman
 * tidak boleh rusak kalau bentuknya berubah: yang tidak dikenal menghasilkan `null`.
 */
export function normalizeOverview(raw) {
  if (!isObj(raw) || !Array.isArray(raw.tables)) return null;
  const { guestbook: g, journal: j, migrations: m } = raw;
  if (!isObj(g) || !isObj(j) || !isObj(m)) return null;
  const tables = raw.tables.map((t) =>
    isObj(t) && typeof t.name === "string" && count(t.rows) !== null ? { name: t.name, rows: t.rows } : null,
  );
  if (tables.includes(null)) return null;
  const out = {
    now: text(raw.now),
    me: text(raw.me),
    tables,
    guestbook: { total: count(g.total), hidden: count(g.hidden), reported: count(g.reported) },
    journal: {
      fetchedAt: j.fetched_at == null ? null : text(j.fetched_at),
      source: j.source == null ? null : text(j.source),
      snapshots: count(j.snapshots),
      githubConfigured: j.github_configured === true,
    },
    migrations: { applied: count(m.applied), latest: m.latest == null ? null : text(m.latest) },
  };
  const required = [out.now, out.me, out.guestbook.total, out.guestbook.hidden, out.guestbook.reported, out.journal.snapshots, out.migrations.applied];
  return required.includes(null) ? null : out;
}

/** Satu baris riwayat dari server -> bentuk tampil, atau `null` jika rusak. */
export function normalizeAudit(raw) {
  if (!isObj(raw) || !Array.isArray(raw.items)) return null;
  const items = raw.items.map((e) =>
    isObj(e) && count(e.id) !== null && text(e.at) !== null && text(e.action) !== null && text(e.who) !== null
      ? { id: e.id, at: e.at, action: e.action, target: e.target == null ? null : text(e.target), who: e.who }
      : null,
  );
  return items.includes(null) ? null : items;
}

// ------------------------------------------------------- Journal (D6b)

/** Batas yang sama dengan server; server tetap memeriksa ulang. */
export const LIMITS = { username: 39, repoName: 100, topic: 60, stackName: 40 };

/** Spasi berurutan (juga baris baru dan tab) jadi satu spasi, ujung dipangkas. */
export function squash(value) {
  return String(value ?? "").split(/\s+/).filter(Boolean).join(" ");
}

export function validateUsername(raw) {
  const v = String(raw ?? "").trim();
  if (!v) return "Isi nama akun GitHub.";
  if (v.length > LIMITS.username) return `Paling banyak ${LIMITS.username} karakter.`;
  if (!/^[A-Za-z0-9-]+$/.test(v)) return "Hanya huruf, angka, dan tanda hubung (-).";
  return "";
}

export function validateRepo(rawName, rawTopic) {
  const name = String(rawName ?? "").trim();
  if (!name) return "Isi nama repo.";
  if (name.length > LIMITS.repoName) return `Nama repo paling banyak ${LIMITS.repoName} karakter.`;
  if (!/^[A-Za-z0-9._-]+$/.test(name)) return "Nama repo hanya boleh huruf, angka, titik, garis bawah, dan tanda hubung.";
  if ([...squash(rawTopic)].length > LIMITS.topic) return `Topik paling banyak ${LIMITS.topic} karakter.`;
  return "";
}

export function validateStack(raw) {
  const name = squash(raw);
  if (!name) return "Isi nama item.";
  if ([...name].length > LIMITS.stackName) return `Paling banyak ${LIMITS.stackName} karakter.`;
  return "";
}

/** Memindahkan elemen ke atas (`-1`) atau ke bawah (`1`). `null` jika tidak mungkin. */
export function moveId(ids, index, delta) {
  const to = index + delta;
  if (!Number.isInteger(index) || index < 0 || index >= ids.length || to < 0 || to >= ids.length) return null;
  const next = [...ids];
  [next[index], next[to]] = [next[to], next[index]];
  return next;
}

/** Id yang dipakai di path: bilangan bulat positif, tidak pernah teks bebas. */
export function isId(value) {
  return Number.isSafeInteger(value) && value >= 1;
}

const list = (raw, row) => {
  if (!Array.isArray(raw)) return null;
  const out = raw.map(row);
  return out.includes(null) ? null : out;
};

/** Pengaturan journal dari server -> bentuk tampil, atau `null` jika rusak. */
export function normalizeJournal(raw) {
  if (!isObj(raw) || !isObj(raw.caps)) return null;
  const repos = list(raw.repos, (r) =>
    isObj(r) && isId(r.id) && text(r.repo_name) !== null && (r.topic == null || text(r.topic) !== null)
      ? { id: r.id, name: r.repo_name, topic: r.topic ?? null }
      : null,
  );
  const stack = list(raw.stack, (r) => (isObj(r) && isId(r.id) && text(r.name) !== null ? { id: r.id, name: r.name } : null));
  const snapshots = list(raw.snapshots, (r) =>
    isObj(r) && isId(r.id) && text(r.fetched_at) !== null && text(r.source) !== null
      ? { id: r.id, fetchedAt: r.fetched_at, source: r.source }
      : null,
  );
  const maxPinned = count(raw.caps.max_pinned);
  const maxStack = count(raw.caps.max_stack);
  if (!repos || !stack || !snapshots || text(raw.now) === null || text(raw.github_username) === null) return null;
  if (maxPinned === null || maxStack === null) return null;
  return { now: raw.now, username: raw.github_username, repos, stack, snapshots, caps: { maxPinned, maxStack } };
}

// ----------------------------------------------------- Guestbook (D6b)

export const MESSAGE_FILTERS = [
  { id: "reported", label: "Dilaporkan", empty: "Tidak ada entri yang dilaporkan." },
  { id: "hidden", label: "Disembunyikan", empty: "Tidak ada entri yang disembunyikan." },
  { id: "all", label: "Semua", empty: "Belum ada entri." },
];

/** Entri Guestbook untuk moderator -> bentuk tampil, atau `null` jika rusak. */
export function normalizeMessages(raw) {
  if (!isObj(raw)) return null;
  return list(raw.items, (m) =>
    isObj(m) &&
    isId(m.id) &&
    text(m.username) !== null &&
    text(m.message) !== null &&
    text(m.created_at) !== null &&
    count(m.reports) !== null &&
    (m.url == null || text(m.url) !== null) &&
    (m.hidden_at == null || text(m.hidden_at) !== null) &&
    (m.last_reported_at == null || text(m.last_reported_at) !== null)
      ? {
          id: m.id,
          username: m.username,
          url: m.url ?? null,
          message: m.message,
          createdAt: m.created_at,
          hiddenAt: m.hidden_at ?? null,
          reports: m.reports,
          lastReportedAt: m.last_reported_at ?? null,
        }
      : null,
  );
}
