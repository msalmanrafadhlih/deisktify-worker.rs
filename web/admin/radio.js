// Tab Radio (D7b): status pemeriksaan stasiun dan tombol "Cek sekarang". Pemeriksa
// (cron di server) hanya menandai stasiun yang basi; tautan baru ditempel lewat tab Data.
// URL stream tidak pernah sampai ke halaman ini (isinya memuat token).

import { html, useEffect, useRef, useState } from "./vendor/standalone.module.js";
import {
  describeError,
  describeRadioReport,
  describeStation,
  formatLocal,
  formatWait,
  manualWait,
  normalizeRadio,
  normalizeRadioReport,
} from "./logic.js";
import { Messages } from "./parts.js";

const TONE_LABELS = { ok: "flag ok", warn: "flag warn", bad: "flag bad", muted: "flag" };

function Summary({ data, busy, wait, onCheck }) {
  const active = data.items.filter((i) => i.isActive);
  const healthy = active.filter((i) => i.result === "ok" && !i.linkChanged).length;
  const failing = active.filter((i) => i.failures > 0).length;
  const off = data.items.filter((i) => i.disabledByCheck).length;
  return html`
    <article class="card">
      <h2>Status radio</h2>
      <dl>
        <div class="stat"><dt>Stasiun aktif</dt><dd>${active.length} dari ${data.items.length}</dd></div>
        <div class="stat"><dt>Sehat</dt><dd>${healthy}</dd></div>
        <div class="stat"><dt>Sedang gagal</dt><dd>${failing}</dd></div>
        <div class="stat"><dt>Dimatikan otomatis</dt><dd>${off}</dd></div>
        <div class="stat">
          <dt>Pengecekan otomatis berikutnya</dt>
          <dd>${formatLocal(data.nextRunAt)}<span class="hint">tiap jam</span></dd>
        </div>
      </dl>
      <div class="actions">
        <button type="button" class="primary" disabled=${busy || wait > 0} onClick=${onCheck}>
          ${wait > 0 ? `Tunggu ${formatWait(wait)}` : busy ? "Memproses…" : "Cek sekarang"}
        </button>
      </div>
      <p class="muted">
        ${`Pemeriksa hanya membaca header stream dan menandai stasiun yang gagal ${data.rules.failThreshold} kali berturut-turut selama minimal ${data.rules.minFailHours} jam. Ia tidak bisa membuat token baru: tempel tautan baru lewat tab Data. "Cek sekarang" memeriksa sampai ${data.rules.batch} stasiun aktif dan bisa memakan beberapa detik.`}
      </p>
    </article>
  `;
}

function Stations({ data }) {
  return html`
    <article class="card">
      <h2>Stasiun</h2>
      ${data.items.length === 0 && html`<p class="muted">Belum ada stasiun.</p>`}
      <ul class="rows">
        ${data.items.map((item) => {
          const state = describeStation(item, data.rules, data.now);
          return html`
            <li key=${item.id} class="row">
              <div class="grow">
                <strong>${item.label}</strong>
                <span class="hint">${state.detail}</span>
              </div>
              <span class=${TONE_LABELS[state.tone]}>${state.label}</span>
            </li>
          `;
        })}
      </ul>
    </article>
  `;
}

export function Radio({ api, onFail }) {
  const [data, setData] = useState(null);
  const [busy, setBusy] = useState(true); // memuat sejak tampil pertama
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [wait, setWait] = useState(0);
  const started = useRef(false);

  async function load() {
    const next = normalizeRadio(await api.radio());
    if (!next) throw Object.assign(new Error("bad"), { status: 500 });
    setData(next);
    setWait(manualWait(next));
    return next;
  }

  async function check() {
    if (busy || wait > 0) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const report = normalizeRadioReport(await api.radioCheck());
      if (!report) throw Object.assign(new Error("bad"), { status: 500 });
      const next = await load();
      const labels = new Map(next.items.map((i) => [i.id, i.label]));
      setNotice(describeRadioReport(report, (id) => labels.get(id) ?? `#${id}`));
    } catch (err) {
      if (onFail(err)) return;
      if (err && err.status === 429 && err.retryAfter) setWait(err.retryAfter);
      setError(describeError(err));
    } finally {
      setBusy(false);
    }
  }

  function reload() {
    setBusy(true);
    setError("");
    setNotice("");
    load()
      .catch((err) => {
        if (!onFail(err)) setError(describeError(err));
      })
      .finally(() => setBusy(false));
  }

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    reload();
  }, []);

  useEffect(() => {
    if (wait <= 0) return undefined;
    const timer = setTimeout(() => setWait((w) => Math.max(0, w - 1)), 1000);
    return () => clearTimeout(timer);
  }, [wait]);

  return html`
    <section aria-label="Radio">
      <${Messages} error=${error} notice=${notice} />
      ${!data && busy && html`<p class="muted">Memuat…</p>`}
      ${!data && !busy && html`<div class="actions"><button type="button" onClick=${reload}>Coba lagi</button></div>`}
      ${data && html`
        <div class="stack">
          <${Summary} data=${data} busy=${busy} wait=${wait} onCheck=${check} />
          <${Stations} data=${data} />
          <div class="actions"><button type="button" disabled=${busy} onClick=${reload}>Muat ulang</button></div>
        </div>
      `}
    </section>
  `;
}
