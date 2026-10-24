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
  "table.insert": "Tambah data",
  "table.update": "Ubah data",
  "table.delete": "Hapus data",
  "table.export": "Ekspor data",
  "preset.items": "Ubah isi preset suara latar",
  "message.purge": "Hapus permanen entri Guestbook",
  "radio.check": "Cek stasiun radio sekarang",
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
  if (m) {
    const kind = { message: "Entri #", repo: "Repo ", stack: "Stack ", github: "Akun " }[m[1]];
    return kind + m[2];
  }
  // `tabel:kunci` (mis. `bookmarks:5`) atau nama tabel saja (ekspor).
  const t = /^([a-z_]+):(\d+)$/.exec(target);
  if (t && t[1] in TABLE_LABELS) return `${TABLE_LABELS[t[1]]} #${t[2]}`;
  if (target in TABLE_LABELS) return TABLE_LABELS[target];
  return String(target);
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

// -------------------------------------------------------- Data (D6c)

const KINDS = ["text", "url", "route", "int", "bool", "date", "timestamp"];

/** Daftar tabel dari server -> bentuk tampil, atau `null` jika rusak. */
export function normalizeTables(raw) {
  if (!isObj(raw)) return null;
  return list(raw.items, (t) => {
    if (!isObj(t) || typeof t.name !== "string" || !/^[a-z_]{1,40}$/.test(t.name)) return null;
    if (text(t.label) === null || text(t.key) === null || text(t.key_label) === null) return null;
    if (typeof t.editable !== "boolean" || typeof t.key_given !== "boolean" || typeof t.paged !== "boolean") return null;
    if (count(t.max_rows) === null || count(t.rows) === null) return null;
    const columns = list(t.columns, (c) =>
      isObj(c) &&
      text(c.name) !== null &&
      text(c.label) !== null &&
      KINDS.includes(c.kind) &&
      typeof c.nullable === "boolean" &&
      (c.min === undefined || Number.isSafeInteger(c.min)) &&
      (c.max === undefined || Number.isSafeInteger(c.max))
        ? { name: c.name, label: c.label, kind: c.kind, nullable: c.nullable, min: c.min ?? null, max: c.max ?? null }
        : null,
    );
    if (!columns) return null;
    return {
      name: t.name,
      label: t.label,
      editable: t.editable,
      key: t.key,
      keyLabel: t.key_label,
      keyGiven: t.key_given,
      paged: t.paged,
      maxRows: t.max_rows,
      rows: t.rows,
      columns,
    };
  });
}

function cellOk(kind, nullable, v) {
  if (v === null) return nullable;
  if (kind === "int") return Number.isSafeInteger(v);
  if (kind === "bool") return typeof v === "boolean";
  return typeof v === "string";
}

/** Baris sebuah tabel dari server, dicek menurut skemanya. `null` jika ada yang rusak. */
export function normalizeRows(raw, table) {
  if (!isObj(raw)) return null;
  return list(raw.items, (r) => {
    if (!isObj(r) || !isId(r[table.key])) return null;
    const row = { [table.key]: r[table.key] };
    for (const c of table.columns) {
      if (!cellOk(c.kind, c.nullable, r[c.name])) return null;
      row[c.name] = r[c.name];
    }
    return row;
  });
}

/** Teks sel untuk layar. */
export function displayValue(col, value) {
  if (value === null || value === undefined) return "—";
  if (col.kind === "bool") return value ? "Ya" : "Tidak";
  if (col.kind === "timestamp") return formatLocal(value);
  return String(value);
}

const URL_RE = /^https?:\/\/[^\s/]\S*$/;

/** Pesan masalah untuk satu isian ("" jika baik). Aturan sama dengan server. */
export function validateField(col, raw) {
  const label = col.label;
  switch (col.kind) {
    case "text": {
      const v = squash(raw);
      if (v === "" && col.nullable) return "";
      const n = [...v].length;
      if (n === 0 && (col.min ?? 0) > 0) return `${label} wajib diisi.`;
      if (n < (col.min ?? 0)) return `${label} paling sedikit ${col.min} karakter.`;
      if (n > (col.max ?? Infinity)) return `${label} paling banyak ${col.max} karakter.`;
      return "";
    }
    case "url":
    case "route": {
      const v = String(raw ?? "").trim();
      if (v === "" && col.nullable) return "";
      if (v === "") return `${label} wajib diisi.`;
      if (v.length > 2048) return `${label} terlalu panjang.`;
      if (URL_RE.test(v)) return "";
      if (col.kind === "route" && /^\/(?![/\\])\S*$/.test(v)) return "";
      return col.kind === "route"
        ? `${label} harus berupa rute (/halaman) atau alamat http(s)://.`
        : `${label} harus berupa alamat http:// atau https://.`;
    }
    case "int": {
      const v = String(raw ?? "").trim();
      if (!/^-?\d{1,15}$/.test(v)) return `${label} harus berupa bilangan bulat.`;
      const n = Number(v);
      if (n < (col.min ?? -Infinity) || n > (col.max ?? Infinity)) return `${label} harus antara ${col.min} dan ${col.max}.`;
      return "";
    }
    case "date": {
      const v = String(raw ?? "").trim();
      const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
      if (!m || Number(m[2]) < 1 || Number(m[2]) > 12 || Number(m[3]) < 1 || Number(m[3]) > 31) return `${label} harus berformat TTTT-BB-HH.`;
      return "";
    }
    default:
      return "";
  }
}

/** Masalah pertama dari seluruh isian (termasuk kunci yang ditentukan pengisi), atau "". */
export function validateDraft(table, draft, creating) {
  if (creating && table.keyGiven) {
    const n = String(draft[table.key] ?? "").trim();
    if (!/^\d{1,9}$/.test(n) || Number(n) < 1) return `${table.keyLabel} harus bilangan bulat 1 atau lebih.`;
  }
  for (const c of table.columns) {
    if (c.kind === "bool") continue;
    const why = validateField(c, draft[c.name]);
    if (why) return why;
  }
  return "";
}

/** Draf awal untuk baris baru. */
export function emptyDraft(table) {
  const draft = {};
  if (table.keyGiven) draft[table.key] = "";
  for (const c of table.columns) draft[c.name] = c.kind === "bool" ? true : c.kind === "int" ? "0" : "";
  return draft;
}

/** Draf dari baris yang ada (semua dijadikan teks, kecuali bool). */
export function draftFromRow(table, row) {
  const draft = {};
  for (const c of table.columns) {
    const v = row[c.name];
    draft[c.name] = c.kind === "bool" ? v === true : v === null || v === undefined ? "" : String(v);
  }
  return draft;
}

/** Badan permintaan tambah/ubah dari draf yang sudah divalidasi. */
export function buildBody(table, draft, creating) {
  const body = {};
  if (creating && table.keyGiven) body[table.key] = Number(String(draft[table.key]).trim());
  for (const c of table.columns) {
    const v = draft[c.name];
    switch (c.kind) {
      case "text": {
        const t = squash(v);
        body[c.name] = t === "" && c.nullable ? null : t;
        break;
      }
      case "url":
      case "route":
      case "date": {
        const t = String(v ?? "").trim();
        body[c.name] = t === "" && c.nullable ? null : t;
        break;
      }
      case "int":
        body[c.name] = Number(String(v).trim());
        break;
      case "bool":
        body[c.name] = v === true;
        break;
      default:
        break;
    }
  }
  return body;
}

/** Isi satu preset: id embience urut menurut posisi. */
export function presetIds(items, presetId) {
  return items
    .filter((i) => i.presetId === presetId)
    .sort((a, b) => a.position - b.position)
    .map((i) => i.embienceId);
}

/** Isi semua preset dari server, atau `null` jika rusak. */
export function normalizePresetItems(raw) {
  if (!isObj(raw)) return null;
  return list(raw.items, (i) =>
    isObj(i) && isId(i.preset_id) && isId(i.embience_id) && count(i.position) !== null
      ? { presetId: i.preset_id, embienceId: i.embience_id, position: i.position }
      : null,
  );
}

/** Menyimpan teks sebagai berkas unduhan (dipakai tombol Ekspor). */
export function downloadName(table, format) {
  return `${table}.${format === "csv" ? "csv" : "json"}`;
}

// ------------------------------------------------------- Radio (D7b)

const RESULTS = ["ok", "http", "network", "timeout", "redirect", "skipped", "inconclusive"];

/** Status pemeriksaan radio dari server -> bentuk tampil, atau `null` jika rusak. */
export function normalizeRadio(raw) {
  if (!isObj(raw) || !isObj(raw.rules) || !Array.isArray(raw.items)) return null;
  const rules = {
    failThreshold: count(raw.rules.fail_threshold),
    minFailHours: count(raw.rules.min_fail_hours),
    batch: count(raw.rules.batch),
    minManualSecs: count(raw.rules.min_manual_secs),
  };
  const items = raw.items.map((r) => {
    if (!isObj(r) || !isId(r.id) || text(r.label) === null) return null;
    const optText = (v) => (v == null ? null : text(v));
    const result = r.last_result == null ? null : text(r.last_result);
    if (result !== null && !RESULTS.includes(result)) return null;
    const failures = count(r.consecutive_failures);
    if (failures === null) return null;
    return {
      id: r.id,
      label: r.label,
      isActive: r.is_active === true,
      checkable: r.checkable === true,
      linkChanged: r.link_changed === true,
      result,
      status: count(r.last_status),
      checkedAt: optText(r.last_checked_at),
      okAt: optText(r.last_ok_at),
      failingSince: optText(r.failing_since),
      failures,
      disabledByCheck: r.disabled_by_check === true,
    };
  });
  if (items.includes(null) || Object.values(rules).includes(null)) return null;
  if (text(raw.now) === null || text(raw.next_run_at) === null) return null;
  return { now: raw.now, nextRunAt: raw.next_run_at, rules, items };
}

const FAILURE_REASONS = {
  network: "tidak bisa tersambung ke server radio",
  timeout: "server radio terlalu lama menjawab",
  redirect: "pengalihan (redirect) tidak valid",
};

/**
 * Keadaan satu stasiun dalam kata-kata: `tone` (`ok`, `warn`, `bad`, `muted`),
 * `label` singkat, dan `detail` yang menjelaskan apa artinya bagi admin.
 */
export function describeStation(item, rules, nowIso) {
  if (!item.isActive) {
    return item.disabledByCheck
      ? {
          tone: "bad",
          label: "Dimatikan otomatis",
          detail: `Pemeriksa menonaktifkan stasiun ini setelah ${rules.failThreshold} kegagalan berturut-turut. Tempel tautan baru lewat tab Data, lalu aktifkan lagi.`,
        }
      : { tone: "muted", label: "Nonaktif", detail: "Dinonaktifkan dari tab Data; tidak diperiksa." };
  }
  if (item.linkChanged) {
    return { tone: "muted", label: "Tautan baru", detail: "Tautannya diganti sejak pemeriksaan terakhir; riwayat mulai dari nol di pemeriksaan berikutnya." };
  }
  if (!item.checkable) {
    return {
      tone: "muted",
      label: "Tidak diperiksa",
      detail: "Tautannya bukan https biasa (atau memakai port khusus), jadi tidak bisa diperiksa dari Cloudflare. Stasiun ini tidak akan dimatikan otomatis.",
    };
  }
  if (item.result === null) return { tone: "muted", label: "Belum diperiksa", detail: "Menunggu pemeriksaan pertama." };
  const ago = item.checkedAt ? timeAgo(item.checkedAt, nowIso) : "";
  if (item.result === "ok") return { tone: "ok", label: "Sehat", detail: `Dicek ${ago}.` };
  if (item.result === "skipped") {
    return { tone: "muted", label: "Tidak diperiksa", detail: "Tautannya dialihkan ke tujuan yang tidak boleh diperiksa. Stasiun ini tidak akan dimatikan otomatis." };
  }
  if (item.result === "inconclusive") {
    const earlier = item.failures > 0 ? ` Sebelumnya gagal ${item.failures} kali berturut-turut.` : "";
    return {
      tone: item.failures > 0 ? "warn" : "muted",
      label: "Belum meyakinkan",
      detail: `Hasil terakhir tidak dihitung (server radio membatasi permintaan, atau semua stasiun gagal sekaligus sehingga masalahnya diduga di sisi Cloudflare).${earlier}`,
    };
  }
  const reason = item.result === "http" ? (item.status ? `server radio menjawab ${item.status}` : "server radio menolak") : FAILURE_REASONS[item.result];
  const since = item.failingSince ? `, gagal sejak ${formatLocal(item.failingSince)}` : "";
  return {
    tone: "warn",
    label: `Gagal ${item.failures}×`,
    detail: `Dicek ${ago}: ${reason}${since}. Dimatikan otomatis setelah ${rules.failThreshold} kegagalan berturut-turut dan minimal ${rules.minFailHours} jam.`,
  };
}

/** Sisa detik sebelum "Cek sekarang" boleh dipakai lagi, menurut jam server. */
export function manualWait(radio) {
  const now = Date.parse(radio.now);
  const last = Math.max(0, ...radio.items.map((i) => Date.parse(i.checkedAt ?? "")).filter(Number.isFinite));
  if (!Number.isFinite(now) || last === 0) return 0;
  const wait = Math.ceil(radio.rules.minManualSecs - (now - last) / 1000);
  return Math.min(Math.max(wait, 0), radio.rules.minManualSecs);
}

/** Hasil "Cek sekarang" dari server -> bentuk tampil, atau `null` jika rusak. */
export function normalizeRadioReport(raw) {
  if (!isObj(raw) || !Array.isArray(raw.disabled)) return null;
  const out = {
    checked: count(raw.checked),
    ok: count(raw.ok),
    failed: count(raw.failed),
    skipped: count(raw.skipped),
    inconclusive: count(raw.inconclusive),
    disabled: raw.disabled,
  };
  if (Object.entries(out).some(([k, v]) => k !== "disabled" && v === null)) return null;
  return out.disabled.every(isId) ? out : null;
}

/** Ringkasan satu kalimat; nama stasiun dicari lewat `labelOf(id)`. */
export function describeRadioReport(report, labelOf) {
  if (report.checked === 0) return "Tidak ada stasiun aktif yang perlu diperiksa.";
  const parts = [`${report.checked} stasiun diperiksa`, `${report.ok} sehat`];
  if (report.failed) parts.push(`${report.failed} gagal`);
  if (report.skipped) parts.push(`${report.skipped} dilewati`);
  if (report.inconclusive) parts.push(`${report.inconclusive} belum meyakinkan`);
  let line = `${parts.join(", ")}.`;
  if (report.disabled.length) line += ` Dimatikan otomatis: ${report.disabled.map(labelOf).join(", ")}.`;
  return line;
}
