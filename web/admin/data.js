// Tab Data (D6c): editor tabel yang digerakkan oleh skema dari server. Daftar tabel,
// kolom, jenis, dan batasnya datang dari `GET /admin/tables`; halaman ini tidak
// menyimpan daftar tabelnya sendiri. Semua nilai ditampilkan sebagai teks biasa.

import { html, useEffect, useRef, useState } from "./vendor/standalone.module.js";
import {
  buildBody,
  describeError,
  displayValue,
  downloadName,
  draftFromRow,
  emptyDraft,
  normalizePresetItems,
  normalizeRows,
  normalizeTables,
  presetIds,
  validateDraft,
} from "./logic.js";
import { ConfirmButton, Messages } from "./parts.js";

const PAGE = 50;

/** Menyimpan teks sebagai berkas unduhan lewat tautan sementara. */
function saveText(filename, mime, content) {
  const url = URL.createObjectURL(new Blob([content], { type: mime }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.rel = "noopener";
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// ---------------------------------------------------------------- Formulir

function Field({ col, value, onChange, disabled }) {
  const id = `f-${col.name}`;
  if (col.kind === "bool") {
    return html`
      <label class="check" for=${id}>
        <input id=${id} type="checkbox" checked=${value === true} disabled=${disabled} onChange=${(e) => onChange(e.currentTarget.checked)} />
        ${col.label}
      </label>
    `;
  }
  const type = col.kind === "date" ? "date" : "text";
  return html`
    <label for=${id}>${col.label}${col.nullable ? html` <span class="muted">(boleh kosong)</span>` : ""}
      <input
        id=${id}
        type=${type}
        value=${value}
        inputmode=${col.kind === "int" ? "numeric" : undefined}
        autocomplete="off"
        spellcheck=${false}
        disabled=${disabled}
        onInput=${(e) => onChange(e.currentTarget.value)}
      />
    </label>
  `;
}

function RowForm({ table, initial, creating, busy, onSave, onCancel }) {
  const [draft, setDraft] = useState(initial);
  const [problem, setProblem] = useState("");

  function submit(event) {
    event.preventDefault();
    const why = validateDraft(table, draft, creating);
    setProblem(why);
    if (why) return;
    onSave(buildBody(table, draft, creating));
  }

  const set = (name) => (value) => setDraft((d) => ({ ...d, [name]: value }));
  const keyCol = { name: table.key, label: table.keyLabel, kind: "int", nullable: false };
  return html`
    <form class="card form" onSubmit=${submit} aria-label=${creating ? "Tambah baris" : "Ubah baris"}>
      <h2>${creating ? `Tambah: ${table.label}` : `Ubah: ${table.label}`}</h2>
      ${creating && table.keyGiven && html`<${Field} col=${keyCol} value=${draft[table.key] ?? ""} onChange=${set(table.key)} disabled=${busy} />`}
      ${table.columns.map((c) => html`<${Field} key=${c.name} col=${c} value=${draft[c.name]} onChange=${set(c.name)} disabled=${busy} />`)}
      ${problem && html`<p class="error" role="alert">${problem}</p>`}
      <div class="actions">
        <button type="submit" class="primary" disabled=${busy}>Simpan</button>
        <button type="button" disabled=${busy} onClick=${onCancel}>Batal</button>
      </div>
    </form>
  `;
}

// ------------------------------------------------------------- Isi preset

function PresetEditor({ api, preset, onClose, onFail }) {
  const [embiences, setEmbiences] = useState(null);
  const [selected, setSelected] = useState([]);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    (async () => {
      try {
        const [items, emb] = await Promise.all([api.presetItems(), api.tableRows("embiences", null, 500)]);
        const parsedItems = normalizePresetItems(items);
        const rows = emb && Array.isArray(emb.items) ? emb.items : null;
        if (!parsedItems || !rows || rows.some((r) => !Number.isSafeInteger(r.id) || typeof r.name !== "string")) {
          throw Object.assign(new Error("bad"), { status: 500 });
        }
        setEmbiences(rows.map((r) => ({ id: r.id, name: r.name })));
        setSelected(presetIds(parsedItems, preset.id));
      } catch (err) {
        if (!onFail(err)) setError(describeError(err));
      } finally {
        setBusy(false);
      }
    })();
  }, []);

  const name = (id) => embiences?.find((e) => e.id === id)?.name ?? `#${id}`;
  const free = (embiences ?? []).filter((e) => !selected.includes(e.id));
  const move = (i, d) =>
    setSelected((list) => {
      const j = i + d;
      if (j < 0 || j >= list.length) return list;
      const next = [...list];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });

  async function save() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await api.setPresetItems(preset.id, selected);
      setNotice("Isi preset disimpan.");
    } catch (err) {
      if (!onFail(err)) setError(describeError(err));
    } finally {
      setBusy(false);
    }
  }

  return html`
    <section class="card form" aria-label="Isi preset">
      <h2>Isi preset: ${preset.label}</h2>
      <${Messages} error=${error} notice=${notice} />
      ${!embiences && busy && html`<p class="muted">Memuat…</p>`}
      ${embiences && html`
        ${selected.length === 0 && html`<p class="muted">Preset ini kosong.</p>`}
        <ol class="rows">
          ${selected.map(
            (id, i) => html`
              <li key=${id} class="row">
                <span class="grow">${i + 1}. ${name(id)}</span>
                <span class="actions">
                  <button type="button" class="small" aria-label=${`Naikkan ${name(id)}`} disabled=${busy || i === 0} onClick=${() => move(i, -1)}>↑</button>
                  <button type="button" class="small" aria-label=${`Turunkan ${name(id)}`} disabled=${busy || i === selected.length - 1} onClick=${() => move(i, 1)}>↓</button>
                  <button type="button" class="small danger" disabled=${busy} onClick=${() => setSelected((l) => l.filter((x) => x !== id))}>Keluarkan</button>
                </span>
              </li>
            `,
          )}
        </ol>
        ${free.length > 0 && html`
          <label for="preset-add">Tambahkan suara
            <select id="preset-add" disabled=${busy} value="" onChange=${(e) => { const v = Number(e.currentTarget.value); if (v) setSelected((l) => [...l, v]); e.currentTarget.value = ""; }}>
              <option value="">Pilih…</option>
              ${free.map((e) => html`<option key=${e.id} value=${String(e.id)}>${e.name}</option>`)}
            </select>
          </label>
        `}
        <div class="actions">
          <button type="button" class="primary" disabled=${busy} onClick=${save}>Simpan isi</button>
          <button type="button" onClick=${onClose}>Tutup</button>
        </div>
      `}
      ${!embiences && !busy && html`<div class="actions"><button type="button" onClick=${onClose}>Tutup</button></div>`}
    </section>
  `;
}

// ------------------------------------------------------------------ Tabel

function DataTable({ table, rows, busy, onEdit, onDelete, onPreset }) {
  const cols = table.columns;
  return html`
    <div class="scroll">
      <table class="data">
        <thead>
          <tr>
            <th>${table.keyLabel}</th>
            ${cols.map((c) => html`<th key=${c.name}>${c.label}</th>`)}
            ${table.editable && html`<th><span class="sr">Aksi</span></th>`}
          </tr>
        </thead>
        <tbody>
          ${rows.map(
            (r) => html`
              <tr key=${r[table.key]}>
                <td class="num">${r[table.key]}</td>
                ${cols.map((c) => html`<td key=${c.name} class=${c.kind === "int" ? "num cell" : "cell"}>${displayValue(c, r[c.name])}</td>`)}
                ${table.editable && html`
                  <td class="actions-cell">
                    <span class="actions">
                      <button type="button" class="small" disabled=${busy} onClick=${() => onEdit(r)}>Ubah</button>
                      ${table.name === "embience_presets" && html`<button type="button" class="small" disabled=${busy} onClick=${() => onPreset(r)}>Isi preset</button>`}
                      <${ConfirmButton} label="Hapus" confirmLabel="Ya, hapus" disabled=${busy} onConfirm=${() => onDelete(r)} />
                    </span>
                  </td>
                `}
              </tr>
            `,
          )}
        </tbody>
      </table>
    </div>
  `;
}

// ---------------------------------------------------------------- Halaman

export function Data({ api, onFail }) {
  const [tables, setTables] = useState(null);
  const [name, setName] = useState(null);
  const [rows, setRows] = useState([]);
  const [more, setMore] = useState(false);
  const [mode, setMode] = useState(null); // null | {creating} | {row}
  const [preset, setPreset] = useState(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const ticket = useRef(0);
  const started = useRef(false);

  const table = tables?.find((t) => t.name === name) ?? null;

  async function fetchTables() {
    const next = normalizeTables(await api.tables());
    if (!next) throw Object.assign(new Error("bad"), { status: 500 });
    setTables(next);
    return next;
  }

  async function fetchRows(t, before) {
    const mine = ++ticket.current;
    const page = normalizeRows(await api.tableRows(t.name, before, PAGE), t);
    if (!page) throw Object.assign(new Error("bad"), { status: 500 });
    if (mine !== ticket.current) return;
    setRows((old) => (before == null ? page : [...old, ...page]));
    setMore(t.paged && page.length === PAGE);
  }

  /** Menjalankan `fn`, menangani galat, lalu `busy` kembali false. */
  async function guarded(fn) {
    setBusy(true);
    setError("");
    try {
      await fn();
    } catch (err) {
      if (!onFail(err)) setError(describeError(err));
      return false;
    } finally {
      setBusy(false);
    }
    return true;
  }

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    guarded(async () => {
      const list = await fetchTables();
      const first = list.find((t) => t.editable) ?? list[0];
      if (first) {
        setName(first.name);
        await fetchRows(first, null);
      }
    });
  }, []);

  function pick(next) {
    if (next.name === name || busy) return;
    setName(next.name);
    setRows([]);
    setMore(false);
    setMode(null);
    setPreset(null);
    setNotice("");
    guarded(() => fetchRows(next, null));
  }

  /** Setelah perubahan: muat ulang baris dan jumlah baris tabel. */
  async function afterChange(done) {
    await guarded(async () => {
      const list = await fetchTables();
      const current = list.find((t) => t.name === name);
      if (current) await fetchRows(current, null);
      setNotice(done);
    });
  }

  async function save(body) {
    if (busy || !table) return;
    const creating = mode?.creating === true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (creating) await api.insertRow(table.name, body);
      else await api.updateRow(table.name, mode.row[table.key], body);
      setMode(null);
    } catch (err) {
      if (onFail(err)) return;
      setError(describeError(err));
      setBusy(false);
      return;
    }
    await afterChange(creating ? "Baris ditambahkan." : "Baris disimpan.");
  }

  async function remove(row) {
    if (busy || !table) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await api.deleteRow(table.name, row[table.key]);
    } catch (err) {
      if (onFail(err)) return;
      setError(describeError(err));
    }
    await afterChange("Baris dihapus.");
  }

  async function exportAs(format) {
    if (busy || !table) return;
    setNotice("");
    const ok = await guarded(async () => {
      const text = await api.exportTable(table.name, format);
      saveText(downloadName(table.name, format), format === "csv" ? "text/csv" : "application/json", text);
    });
    if (ok) setNotice(`Berkas ${downloadName(table.name, format)} diunduh.`);
  }

  const full = table && table.editable && table.rows >= table.maxRows;
  return html`
    <section aria-label="Data">
      <${Messages} error=${error} notice=${notice} />
      ${!tables && busy && html`<p class="muted">Memuat…</p>`}
      ${tables && html`
        <nav class="seg" aria-label="Pilih tabel">
          ${tables.map(
            (t) => html`<button key=${t.name} type="button" class=${t.name === name ? "tab active" : "tab"} aria-pressed=${t.name === name} disabled=${busy && t.name !== name} onClick=${() => pick(t)}>${t.label} <span class="count">${t.rows}</span></button>`,
          )}
        </nav>
      `}
      ${table && html`
        <div class="bar">
          <p class="muted">
            ${table.editable ? `${table.rows} dari paling banyak ${table.maxRows} baris.` : `${table.rows} baris. Tabel ini hanya bisa dilihat dan diekspor.`}
            ${table.name === "chat_messages" ? " Untuk menyembunyikan atau menghapus entri, pakai tab Guestbook." : ""}
          </p>
          <span class="actions">
            ${table.editable && html`<button type="button" class="primary" disabled=${busy || full || mode !== null} onClick=${() => { setPreset(null); setMode({ creating: true }); }}>Tambah baris</button>`}
            <button type="button" disabled=${busy} onClick=${() => exportAs("json")}>Ekspor JSON</button>
            <button type="button" disabled=${busy} onClick=${() => exportAs("csv")}>Ekspor CSV</button>
          </span>
        </div>
        ${full && html`<p class="muted">Tabel sudah penuh. Hapus satu baris untuk menambah.</p>`}
        ${mode && html`
          <${RowForm}
            key=${mode.creating ? "new" : mode.row[table.key]}
            table=${table}
            creating=${mode.creating === true}
            initial=${mode.creating ? emptyDraft(table) : draftFromRow(table, mode.row)}
            busy=${busy}
            onSave=${save}
            onCancel=${() => setMode(null)}
          />
        `}
        ${preset && html`<${PresetEditor} key=${preset.id} api=${api} preset=${{ id: preset[table.key], label: preset.label }} onClose=${() => setPreset(null)} onFail=${onFail} />`}
        ${rows.length === 0 && !busy && html`<p class="muted">Tabel ini kosong.</p>`}
        ${rows.length > 0 && html`
          <${DataTable}
            table=${table}
            rows=${rows}
            busy=${busy}
            onEdit=${(r) => { setPreset(null); setMode({ row: r }); }}
            onDelete=${remove}
            onPreset=${(r) => { setMode(null); setPreset(r); }}
          />
        `}
        ${more && html`<button type="button" disabled=${busy} onClick=${() => { const last = rows[rows.length - 1][table.key]; guarded(() => fetchRows(table, last)); }}>Muat yang lebih lama</button>`}
      `}
    </section>
  `;
}
