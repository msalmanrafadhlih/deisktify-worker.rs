// Tab Journal (D6b): refresh, riwayat snapshot, akun GitHub, repo sematan, dan stack.
// Server memeriksa ulang semua isian; pemeriksaan di sini hanya agar pesannya jelas.

import { html, useEffect, useRef, useState } from "./vendor/standalone.module.js";
import {
  describeError,
  formatLocal,
  formatWait,
  LIMITS,
  moveId,
  normalizeJournal,
  squash,
  timeAgo,
  validateRepo,
  validateStack,
  validateUsername,
} from "./logic.js";
import { ConfirmButton, Messages } from "./parts.js";

const SOURCE_LABELS = { github: "GitHub", seed: "Data awal (seed)" };

// ------------------------------------------------------------- Refresh

function RefreshCard({ data, busy, wait, onRefresh }) {
  const latest = data.snapshots[0] ?? null;
  return html`
    <article class="card">
      <h2>Data dari GitHub</h2>
      <dl>
        <div class="stat">
          <dt>Terakhir diperbarui</dt>
          <dd>
            ${latest ? timeAgo(latest.fetchedAt, data.now) : "belum pernah"}
            ${latest && html`<span class="hint">${formatLocal(latest.fetchedAt)} · ${SOURCE_LABELS[latest.source] ?? latest.source}</span>`}
          </dd>
        </div>
      </dl>
      <div class="actions">
        <button type="button" class="primary" disabled=${busy || wait > 0} onClick=${onRefresh}>
          ${wait > 0 ? `Tunggu ${formatWait(wait)}` : busy ? "Memproses…" : "Refresh sekarang"}
        </button>
      </div>
      <p class="muted">
        Perubahan akun, repo, atau stack di bawah baru terlihat di situs setelah refresh (atau saat jadwal otomatis berikutnya).
      </p>
    </article>
  `;
}

// -------------------------------------------------------------- Akun

function UsernameCard({ data, busy, run }) {
  const [value, setValue] = useState(data.username);
  const [problem, setProblem] = useState("");
  const changed = value.trim() !== data.username;

  function submit(event) {
    event.preventDefault();
    const why = validateUsername(value);
    setProblem(why);
    if (why || !changed) return;
    run((api) => api.setUsername(value.trim()), "Akun GitHub disimpan.");
  }

  return html`
    <article class="card">
      <h2>Akun GitHub</h2>
      <form class="inline" onSubmit=${submit}>
        <label for="gh-user" class="sr">Nama akun GitHub</label>
        <input id="gh-user" value=${value} maxlength=${LIMITS.username} autocomplete="off" spellcheck=${false} disabled=${busy} onInput=${(e) => setValue(e.currentTarget.value)} />
        <button type="submit" class="primary" disabled=${busy || !changed}>Simpan</button>
      </form>
      ${problem && html`<p class="error" role="alert">${problem}</p>`}
    </article>
  `;
}

// ------------------------------------------------------------ Daftar

/** Satu baris yang bisa diubah di tempat. `fields` = [{key,label,max}]. */
function Row({ item, index, total, busy, title, subtitle, fields, validate, onSave, onDelete, onMove }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState({});
  const [problem, setProblem] = useState("");

  function start() {
    const next = {};
    for (const f of fields) next[f.key] = item[f.key] ?? "";
    setDraft(next);
    setProblem("");
    setEditing(true);
  }

  function submit(event) {
    event.preventDefault();
    const why = validate(draft);
    setProblem(why);
    if (why) return;
    setEditing(false);
    onSave(draft);
  }

  if (editing) {
    return html`
      <li class="row">
        <form class="edit" onSubmit=${submit}>
          ${fields.map(
            (f) => html`
              <label key=${f.key}>${f.label}
                <input value=${draft[f.key] ?? ""} maxlength=${f.max * 2} autocomplete="off" spellcheck=${false} onInput=${(e) => { const v = e.currentTarget.value; setDraft((d) => ({ ...d, [f.key]: v })); }} />
              </label>
            `,
          )}
          ${problem && html`<p class="error" role="alert">${problem}</p>`}
          <div class="actions">
            <button type="submit" class="small primary" disabled=${busy}>Simpan</button>
            <button type="button" class="small" onClick=${() => setEditing(false)}>Batal</button>
          </div>
        </form>
      </li>
    `;
  }

  return html`
    <li class="row">
      <div class="grow">
        <strong>${title}</strong>
        ${subtitle && html`<div class="muted">${subtitle}</div>`}
      </div>
      <div class="actions">
        <button type="button" class="small" aria-label=${`Naikkan ${title}`} disabled=${busy || index === 0} onClick=${() => onMove(-1)}>↑</button>
        <button type="button" class="small" aria-label=${`Turunkan ${title}`} disabled=${busy || index === total - 1} onClick=${() => onMove(1)}>↓</button>
        <button type="button" class="small" disabled=${busy} onClick=${start}>Ubah</button>
        <${ConfirmButton} label="Hapus" confirmLabel="Ya, hapus" disabled=${busy} onConfirm=${onDelete} />
      </div>
    </li>
  `;
}

function AddForm({ fields, validate, busy, full, label, onAdd }) {
  const empty = () => Object.fromEntries(fields.map((f) => [f.key, ""]));
  const [draft, setDraft] = useState(empty);
  const [problem, setProblem] = useState("");

  function submit(event) {
    event.preventDefault();
    const why = validate(draft);
    setProblem(why);
    if (why) return;
    onAdd(draft);
    setDraft(empty());
  }

  return html`
    <form class="add" onSubmit=${submit}>
      ${fields.map(
        (f) => html`
          <label key=${f.key}>${f.label}
            <input value=${draft[f.key]} maxlength=${f.max * 2} autocomplete="off" spellcheck=${false} disabled=${busy || full} onInput=${(e) => { const v = e.currentTarget.value; setDraft((d) => ({ ...d, [f.key]: v })); }} />
          </label>
        `,
      )}
      <button type="submit" class="primary" disabled=${busy || full}>${label}</button>
      ${full && html`<p class="muted">Daftar sudah penuh. Hapus satu item untuk menambah.</p>`}
      ${problem && html`<p class="error" role="alert">${problem}</p>`}
    </form>
  `;
}

const REPO_FIELDS = [
  { key: "name", label: "Nama repo", max: LIMITS.repoName },
  { key: "topic", label: "Topik (boleh kosong)", max: LIMITS.topic },
];
const STACK_FIELDS = [{ key: "name", label: "Nama", max: LIMITS.stackName }];

function ReposCard({ data, busy, run }) {
  const ids = data.repos.map((r) => r.id);
  const validate = (d) => validateRepo(d.name, d.topic);
  return html`
    <article class="card">
      <h2>Repo sematan <span class="muted">(${data.repos.length}/${data.caps.maxPinned})</span></h2>
      ${data.repos.length === 0 && html`<p class="muted">Belum ada repo sematan.</p>`}
      <ul class="rows">
        ${data.repos.map(
          (r, i) => html`
            <${Row}
              key=${r.id}
              item=${r}
              index=${i}
              total=${data.repos.length}
              busy=${busy}
              title=${r.name}
              subtitle=${r.topic}
              fields=${REPO_FIELDS}
              validate=${validate}
              onSave=${(d) => run((api) => api.editRepo(r.id, d.name.trim(), squash(d.topic)), "Repo disimpan.")}
              onDelete=${() => run((api) => api.deleteRepo(r.id), "Repo dihapus.")}
              onMove=${(delta) => {
                const next = moveId(ids, i, delta);
                if (next) run((api) => api.orderRepos(next), "Urutan disimpan.");
              }}
            />
          `,
        )}
      </ul>
      <${AddForm}
        fields=${REPO_FIELDS}
        validate=${validate}
        busy=${busy}
        full=${data.repos.length >= data.caps.maxPinned}
        label="Tambah repo"
        onAdd=${(d) => run((api) => api.addRepo(d.name.trim(), squash(d.topic)), "Repo ditambahkan.")}
      />
    </article>
  `;
}

function StackCard({ data, busy, run }) {
  const ids = data.stack.map((s) => s.id);
  const validate = (d) => validateStack(d.name);
  return html`
    <article class="card">
      <h2>Stack <span class="muted">(${data.stack.length}/${data.caps.maxStack})</span></h2>
      ${data.stack.length === 0 && html`<p class="muted">Belum ada item stack.</p>`}
      <ul class="rows">
        ${data.stack.map(
          (s, i) => html`
            <${Row}
              key=${s.id}
              item=${s}
              index=${i}
              total=${data.stack.length}
              busy=${busy}
              title=${s.name}
              subtitle=${""}
              fields=${STACK_FIELDS}
              validate=${validate}
              onSave=${(d) => run((api) => api.editStack(s.id, squash(d.name)), "Item disimpan.")}
              onDelete=${() => run((api) => api.deleteStack(s.id), "Item dihapus.")}
              onMove=${(delta) => {
                const next = moveId(ids, i, delta);
                if (next) run((api) => api.orderStack(next), "Urutan disimpan.");
              }}
            />
          `,
        )}
      </ul>
      <${AddForm}
        fields=${STACK_FIELDS}
        validate=${validate}
        busy=${busy}
        full=${data.stack.length >= data.caps.maxStack}
        label="Tambah item"
        onAdd=${(d) => run((api) => api.addStack(squash(d.name)), "Item ditambahkan.")}
      />
    </article>
  `;
}

function SnapshotsCard({ data }) {
  return html`
    <article class="card">
      <h2>Snapshot tersimpan</h2>
      ${data.snapshots.length === 0 && html`<p class="muted">Belum ada snapshot.</p>`}
      <ul class="rows compact">
        ${data.snapshots.map(
          (s) => html`<li key=${s.id} class="row"><span>${formatLocal(s.fetchedAt)}</span><span class="muted">${SOURCE_LABELS[s.source] ?? s.source}</span></li>`,
        )}
      </ul>
      <p class="muted">Menampilkan paling banyak 20 yang terbaru.</p>
    </article>
  `;
}

// ------------------------------------------------------------- Halaman

export function Journal({ api, onFail }) {
  const [data, setData] = useState(null);
  const [busy, setBusy] = useState(true); // memuat sejak tampil pertama
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [wait, setWait] = useState(0);
  const started = useRef(false);

  async function load() {
    const next = normalizeJournal(await api.journal());
    if (!next) throw Object.assign(new Error("bad"), { status: 500 });
    setData(next);
    return next;
  }

  /** Menjalankan satu aksi lalu memuat ulang; satu aksi pada satu waktu. */
  async function run(action, done) {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action(api);
      await load();
      setNotice(done);
    } catch (err) {
      if (onFail(err)) return;
      setError(describeError(err));
      // Setelah gagal (mis. daftar sudah berubah), tampilkan keadaan terbaru.
      try {
        await load();
      } catch (again) {
        onFail(again);
      }
    } finally {
      setBusy(false);
    }
  }

  async function refresh() {
    if (busy || wait > 0) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await api.journalRefresh();
      await load();
      setNotice("Data GitHub diperbarui.");
    } catch (err) {
      if (onFail(err)) return;
      if (err && err.status === 429 && err.retryAfter) setWait(err.retryAfter);
      setError(describeError(err));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    setBusy(true);
    load()
      .catch((err) => {
        if (!onFail(err)) setError(describeError(err));
      })
      .finally(() => setBusy(false));
  }, []);

  useEffect(() => {
    if (wait <= 0) return undefined;
    const timer = setTimeout(() => setWait((w) => Math.max(0, w - 1)), 1000);
    return () => clearTimeout(timer);
  }, [wait]);

  return html`
    <section aria-label="Journal">
      <${Messages} error=${error} notice=${notice} />
      ${!data && busy && html`<p class="muted">Memuat…</p>`}
      ${!data && !busy && html`<div class="actions"><button type="button" onClick=${() => { setBusy(true); setError(""); load().catch((e) => { if (!onFail(e)) setError(describeError(e)); }).finally(() => setBusy(false)); }}>Coba lagi</button></div>`}
      ${data && html`
        <div class="stack">
          <${RefreshCard} data=${data} busy=${busy} wait=${wait} onRefresh=${refresh} />
          <${UsernameCard} key=${data.username} data=${data} busy=${busy} run=${run} />
          <${ReposCard} data=${data} busy=${busy} run=${run} />
          <${StackCard} data=${data} busy=${busy} run=${run} />
          <${SnapshotsCard} data=${data} />
        </div>
      `}
    </section>
  `;
}
