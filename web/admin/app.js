// Dashboard admin (D6a, D6b, D6c, D7b). JavaScript biasa tanpa build step: Preact + htm
// (vendor/standalone.module.js). Semua teks ke layar lewat Preact (teks biasa,
// tidak pernah innerHTML), jadi isi dari server tidak bisa menyisipkan markup.

import { html, render, useState, useEffect, useRef } from "./vendor/standalone.module.js";
import { createApi } from "./api.js";
import { Data } from "./data.js";
import { Guestbook } from "./guestbook.js";
import { Journal } from "./journal.js";
import { Radio } from "./radio.js";
import {
  IDLE_LIMIT_MS,
  actionLabel,
  describeError,
  formatLocal,
  isIdle,
  normalizeAudit,
  normalizeOverview,
  tableLabel,
  targetLabel,
  timeAgo,
  whoLabel,
} from "./logic.js";

const api = createApi((...args) => fetch(...args));
const AUDIT_PAGE = 30;

const SOURCE_LABELS = { github: "GitHub", seed: "Data awal (seed)" };

// ---------------------------------------------------------------- Masuk

function Login({ onDone, notice }) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function submit(event) {
    event.preventDefault();
    if (busy || !value.trim()) return;
    setBusy(true);
    setError("");
    try {
      const overview = normalizeOverview(await api.login(value));
      if (!overview) {
        api.lock();
        setError("Respons server tidak dikenali.");
        setBusy(false);
        return;
      }
      onDone(overview); // halaman masuk dilepas; tidak ada lagi yang perlu diubah di sini
    } catch (err) {
      setError(describeError(err));
      setBusy(false);
    }
  }

  return html`
    <main class="login">
      <h1>Dashboard Admin</h1>
      <p class="muted">Masukkan token admin. Token hanya disimpan di memori halaman ini dan hilang saat halaman ditutup atau dikunci.</p>
      ${notice && html`<p class="notice" role="status">${notice}</p>`}
      <form onSubmit=${submit}>
        <label for="token">Token admin</label>
        <input
          id="token"
          type="password"
          name="admin-token"
          autocomplete="current-password"
          autocapitalize="off"
          autocorrect="off"
          spellcheck=${false}
          value=${value}
          onInput=${(e) => setValue(e.currentTarget.value)}
          disabled=${busy}
          autofocus
        />
        <button type="submit" class="primary" disabled=${busy || !value.trim()}>${busy ? "Memeriksa…" : "Masuk"}</button>
        ${error && html`<p class="error" role="alert">${error}</p>`}
      </form>
    </main>
  `;
}

// ------------------------------------------------------------- Ringkasan

function Stat({ label, value, hint }) {
  return html`
    <div class="stat">
      <dt>${label}</dt>
      <dd>${value}${hint && html`<span class="hint">${hint}</span>`}</dd>
    </div>
  `;
}

function Overview({ overview, busy, error, onRefresh }) {
  const { guestbook: g, journal: j, migrations: m, tables, now } = overview;
  return html`
    <section aria-label="Ringkasan">
      <div class="bar">
        <p class="muted">Waktu server: ${formatLocal(now)}</p>
        <button onClick=${onRefresh} disabled=${busy}>${busy ? "Memuat…" : "Perbarui"}</button>
      </div>
      ${error && html`<p class="error" role="alert">${error}</p>`}
      <div class="grid">
        <article class="card">
          <h2>Guestbook</h2>
          <dl>
            <${Stat} label="Entri" value=${g.total} />
            <${Stat} label="Disembunyikan" value=${g.hidden} />
            <${Stat}
              label="Dilaporkan (masih tampil)"
              value=${g.reported}
              hint=${g.reported > 0 ? "perlu ditinjau" : ""}
            />
          </dl>
        </article>
        <article class="card">
          <h2>Journal GitHub</h2>
          <dl>
            <${Stat}
              label="Terakhir diperbarui"
              value=${timeAgo(j.fetchedAt, now)}
              hint=${j.fetchedAt ? formatLocal(j.fetchedAt) : ""}
            />
            <${Stat} label="Sumber" value=${j.source ? (SOURCE_LABELS[j.source] ?? j.source) : "belum ada"} />
            <${Stat} label="Snapshot tersimpan" value=${j.snapshots} />
            <${Stat}
              label="Token GitHub"
              value=${j.githubConfigured ? "Terpasang" : "Belum dipasang"}
              hint=${j.githubConfigured ? "" : "cron dan refresh tidak berjalan"}
            />
          </dl>
        </article>
        <article class="card">
          <h2>Skema database</h2>
          <dl>
            <${Stat} label="Migrasi terakhir" value=${m.latest ?? "belum ada"} />
            <${Stat} label="Migrasi diterapkan" value=${m.applied} />
          </dl>
        </article>
      </div>
      <article class="card">
        <h2>Isi tabel</h2>
        <table>
          <thead><tr><th>Tabel</th><th class="num">Baris</th></tr></thead>
          <tbody>
            ${tables.map(
              (t) => html`<tr key=${t.name}><td>${tableLabel(t.name)} <code>${t.name}</code></td><td class="num">${t.rows}</td></tr>`,
            )}
          </tbody>
        </table>
      </article>
    </section>
  `;
}

// --------------------------------------------------------------- Riwayat

function History({ me, onFail }) {
  const [items, setItems] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [more, setMore] = useState(false);
  const started = useRef(false);

  async function load(before) {
    setBusy(true);
    setError("");
    try {
      const page = normalizeAudit(await api.audit(before, AUDIT_PAGE));
      if (!page) throw Object.assign(new Error("bad"), { status: 500 });
      setItems((old) => (before == null ? page : [...old, ...page]));
      setMore(page.length === AUDIT_PAGE);
    } catch (err) {
      if (!onFail(err)) setError(describeError(err));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    load(null);
  }, []);

  const last = items.length ? items[items.length - 1].id : null;
  return html`
    <section aria-label="Riwayat">
      <div class="bar">
        <p class="muted">Aksi admin yang berhasil. Paling baru 500 yang disimpan.</p>
        <button onClick=${() => load(null)} disabled=${busy}>Muat ulang</button>
      </div>
      ${error && html`<p class="error" role="alert">${error}</p>`}
      ${!busy && !error && items.length === 0 && html`<p class="muted">Belum ada riwayat.</p>`}
      <ul class="log">
        ${items.map(
          (e) => html`
            <li key=${e.id}>
              <strong>${actionLabel(e.action)}</strong>
              ${e.target && html`<span> · ${targetLabel(e.target)}</span>`}
              <div class="muted">${formatLocal(e.at)} · ${whoLabel(e.who, me)}</div>
            </li>
          `,
        )}
      </ul>
      ${more && html`<button onClick=${() => load(last)} disabled=${busy}>${busy ? "Memuat…" : "Muat yang lebih lama"}</button>`}
    </section>
  `;
}

// ------------------------------------------------------------------ Rangka

const TABS = [
  { id: "overview", label: "Ringkasan" },
  { id: "journal", label: "Journal" },
  { id: "guestbook", label: "Guestbook" },
  { id: "radio", label: "Radio" },
  { id: "data", label: "Data" },
  { id: "history", label: "Riwayat" },
];

function Shell({ overview, setOverview, onLock, onFail }) {
  const [tab, setTab] = useState("overview");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function refresh() {
    setBusy(true);
    setError("");
    try {
      const next = normalizeOverview(await api.overview());
      if (!next) throw Object.assign(new Error("bad"), { status: 500 });
      setOverview(next);
    } catch (err) {
      if (!onFail(err)) setError(describeError(err));
    } finally {
      setBusy(false);
    }
  }

  return html`
    <div class="shell">
      <header>
        <h1>Dashboard Admin</h1>
        <button onClick=${() => onLock("")}>Kunci</button>
      </header>
      <nav aria-label="Bagian">
        ${TABS.map(
          (t) => html`<button key=${t.id} class=${tab === t.id ? "tab active" : "tab"} aria-current=${tab === t.id ? "page" : undefined} onClick=${() => setTab(t.id)}>${t.label}</button>`,
        )}
      </nav>
      <main>
        ${tab === "overview" && html`<${Overview} overview=${overview} busy=${busy} error=${error} onRefresh=${refresh} />`}
        ${tab === "journal" && html`<${Journal} api=${api} onFail=${onFail} />`}
        ${tab === "guestbook" && html`<${Guestbook} api=${api} onFail=${onFail} />`}
        ${tab === "radio" && html`<${Radio} api=${api} onFail=${onFail} />`}
        ${tab === "data" && html`<${Data} api=${api} onFail=${onFail} />`}
        ${tab === "history" && html`<${History} me=${overview.me} onFail=${onFail} />`}
      </main>
      <footer class="muted">Terkunci otomatis setelah ${IDLE_LIMIT_MS / 60000} menit tidak aktif.</footer>
    </div>
  `;
}

function App() {
  const [overview, setOverview] = useState(null);
  const [notice, setNotice] = useState("");

  function lock(message) {
    api.lock();
    setOverview(null);
    setNotice(message);
  }

  /** `true` jika errornya mengakhiri sesi (token ditolak) dan sudah ditangani. */
  function onFail(err) {
    if (err && err.status === 401) {
      lock("Sesi berakhir. Masuk lagi.");
      return true;
    }
    return false;
  }

  const signedIn = overview !== null;
  useEffect(() => {
    if (!signedIn) return undefined;
    let last = Date.now();
    const touch = () => {
      last = Date.now();
    };
    const check = () => {
      if (isIdle(last, Date.now())) lock("Terkunci otomatis karena lama tidak aktif.");
    };
    const onVisible = () => {
      if (document.visibilityState === "visible") check();
    };
    const events = ["pointerdown", "keydown", "touchstart"];
    for (const name of events) window.addEventListener(name, touch, { passive: true });
    document.addEventListener("visibilitychange", onVisible);
    const timer = setInterval(check, 5000);
    return () => {
      for (const name of events) window.removeEventListener(name, touch);
      document.removeEventListener("visibilitychange", onVisible);
      clearInterval(timer);
    };
  }, [signedIn]);

  if (!signedIn) {
    return html`<${Login} notice=${notice} onDone=${(o) => { setNotice(""); setOverview(o); }} />`;
  }
  return html`<${Shell} overview=${overview} setOverview=${setOverview} onLock=${lock} onFail=${onFail} />`;
}

render(html`<${App} />`, document.getElementById("app"));
