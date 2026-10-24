// Tab Guestbook (D6b): moderasi entri. Teks entri ditampilkan sebagai teks biasa
// (tidak dirender sebagai Markdown, tidak ada tautan yang bisa diklik), jadi isi
// yang berbahaya tidak bisa menjalankan apa pun di halaman admin.

import { html, useEffect, useRef, useState } from "./vendor/standalone.module.js";
import { describeError, formatLocal, MESSAGE_FILTERS, normalizeMessages } from "./logic.js";
import { ConfirmButton, Messages } from "./parts.js";

const PAGE = 30;

/** Hapus permanen: harus mengetik id entri dulu (tidak ada tombol sekali klik). */
function PurgeButton({ id, busy, onPurge }) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  if (!open) {
    return html`<button type="button" class="small danger" disabled=${busy} onClick=${() => setOpen(true)}>Hapus permanen…</button>`;
  }
  const ready = typed.trim() === String(id);
  return html`
    <form class="purge" onSubmit=${(e) => { e.preventDefault(); if (ready) onPurge(); }}>
      <label for=${`purge-${id}`}>Entri dan laporannya dihapus selamanya dan tidak bisa dikembalikan. Ketik <strong>${id}</strong> untuk melanjutkan.</label>
      <div class="actions">
        <input id=${`purge-${id}`} value=${typed} inputmode="numeric" autocomplete="off" onInput=${(e) => setTyped(e.currentTarget.value)} />
        <button type="submit" class="small danger solid" disabled=${busy || !ready}>Hapus permanen</button>
        <button type="button" class="small" onClick=${() => { setOpen(false); setTyped(""); }}>Batal</button>
      </div>
    </form>
  `;
}

function Entry({ m, busy, onHide, onRestore, onDismiss, onPurge }) {
  const hidden = m.hiddenAt !== null;
  return html`
    <li class=${hidden ? "entry is-hidden" : "entry"}>
      <div class="entry-head">
        <strong>${m.username}</strong>
        <span class="muted">#${m.id} · ${formatLocal(m.createdAt)}</span>
      </div>
      ${m.url && html`<div class="muted">Tautan: <code>${m.url}</code></div>`}
      <pre class="msg">${m.message}</pre>
      <div class="flags">
        ${m.reports > 0 && html`<span class="flag warn">${m.reports} laporan${m.lastReportedAt ? ` · terakhir ${formatLocal(m.lastReportedAt)}` : ""}</span>`}
        ${hidden && html`<span class="flag">Disembunyikan ${formatLocal(m.hiddenAt)}</span>`}
      </div>
      <div class="actions">
        ${!hidden && html`<${ConfirmButton} label="Sembunyikan" confirmLabel="Ya, sembunyikan" disabled=${busy} onConfirm=${onHide} />`}
        ${hidden && html`<button type="button" class="small" disabled=${busy} onClick=${onRestore}>Tampilkan lagi</button>`}
        ${hidden && html`<${PurgeButton} id=${m.id} busy=${busy} onPurge=${onPurge} />`}
        ${m.reports > 0 && html`<${ConfirmButton} label="Tolak laporan" confirmLabel="Ya, tolak laporan" danger=${false} disabled=${busy} onConfirm=${onDismiss} />`}
      </div>
    </li>
  `;
}

export function Guestbook({ api, onFail }) {
  const [filter, setFilter] = useState("reported");
  const [items, setItems] = useState([]);
  const [more, setMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  // Menolak hasil permintaan lama kalau filter sudah berganti.
  const ticket = useRef(0);
  const started = useRef(false);

  async function load(which, before) {
    const mine = ++ticket.current;
    setBusy(true);
    setError("");
    try {
      const page = normalizeMessages(await api.messages(which, before, PAGE));
      if (!page) throw Object.assign(new Error("bad"), { status: 500 });
      if (mine !== ticket.current) return;
      setItems((old) => (before == null ? page : [...old, ...page]));
      setMore(page.length === PAGE);
      setLoaded(true);
    } catch (err) {
      if (mine !== ticket.current) return;
      if (!onFail(err)) setError(describeError(err));
    } finally {
      if (mine === ticket.current) setBusy(false);
    }
  }

  async function act(action, done) {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
      setNotice(done);
    } catch (err) {
      if (onFail(err)) return;
      setError(describeError(err));
    }
    // Berhasil atau tidak, tampilkan keadaan terbaru dari awal.
    await load(filter, null);
  }

  function pick(next) {
    if (next === filter) return;
    setFilter(next);
    setItems([]);
    setMore(false);
    setLoaded(false);
    setNotice("");
    load(next, null);
  }

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    load(filter, null);
  }, []);

  const current = MESSAGE_FILTERS.find((f) => f.id === filter);
  const last = items.length ? items[items.length - 1].id : null;
  return html`
    <section aria-label="Guestbook">
      <div class="bar">
        <div class="seg" role="group" aria-label="Saring entri">
          ${MESSAGE_FILTERS.map(
            (f) => html`<button key=${f.id} type="button" class=${filter === f.id ? "tab active" : "tab"} aria-pressed=${filter === f.id} disabled=${busy && filter !== f.id} onClick=${() => pick(f.id)}>${f.label}</button>`,
          )}
        </div>
        <button type="button" disabled=${busy} onClick=${() => load(filter, null)}>${busy ? "Memuat…" : "Muat ulang"}</button>
      </div>
      <${Messages} error=${error} notice=${notice} />
      ${loaded && !busy && items.length === 0 && !error && html`<p class="muted">${current.empty}</p>`}
      <ul class="entries">
        ${items.map(
          (m) => html`
            <${Entry}
              key=${m.id}
              m=${m}
              busy=${busy}
              onHide=${() => act(() => api.hideMessage(m.id), `Entri #${m.id} disembunyikan.`)}
              onRestore=${() => act(() => api.restoreMessage(m.id), `Entri #${m.id} ditampilkan lagi.`)}
              onDismiss=${() => act(() => api.dismissReports(m.id), `Laporan entri #${m.id} ditolak.`)}
              onPurge=${() => act(() => api.purgeMessage(m.id), `Entri #${m.id} dihapus permanen.`)}
            />
          `,
        )}
      </ul>
      ${more && html`<button type="button" disabled=${busy} onClick=${() => load(filter, last)}>${busy ? "Memuat…" : "Muat yang lebih lama"}</button>`}
    </section>
  `;
}
