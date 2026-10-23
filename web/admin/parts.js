// Bagian tampilan yang dipakai beberapa tab. Semua teks lewat Preact (teks biasa).

import { html, useState } from "./vendor/standalone.module.js";

/** Tombol dua langkah: klik pertama meminta konfirmasi, klik kedua menjalankan. */
export function ConfirmButton({ label, confirmLabel = "Ya, lanjut", onConfirm, disabled, danger = true }) {
  const [asking, setAsking] = useState(false);
  if (!asking) {
    return html`<button type="button" class=${danger ? "small danger" : "small"} disabled=${disabled} onClick=${() => setAsking(true)}>${label}</button>`;
  }
  return html`
    <span class="confirm" role="group" aria-label="Konfirmasi">
      <button
        type="button"
        class="small danger solid"
        disabled=${disabled}
        onClick=${() => {
          setAsking(false);
          onConfirm();
        }}
      >${confirmLabel}</button>
      <button type="button" class="small" onClick=${() => setAsking(false)}>Batal</button>
    </span>
  `;
}

/** Pesan hasil: `error` (merah) atau `notice` (hijau/netral). */
export function Messages({ error, notice }) {
  return html`
    ${error && html`<p class="error" role="alert">${error}</p>`}
    ${notice && html`<p class="ok" role="status">${notice}</p>`}
  `;
}
