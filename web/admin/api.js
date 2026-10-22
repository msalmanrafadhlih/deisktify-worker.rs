// Klien API dashboard. Token admin hanya hidup di penutup (closure) ini: tidak di
// `window`, tidak di localStorage/sessionStorage, tidak di URL, tidak di log.

import { parseRetryAfter } from "./logic.js";

export const PATHS = {
  login: "/api/v1/admin/login",
  overview: "/api/v1/admin/overview",
  audit: "/api/v1/admin/audit",
};

export class ApiError extends Error {
  constructor(status, retryAfter = null) {
    // Pesan sengaja generik: isinya tidak pernah menyalin teks dari server.
    super(`HTTP ${status}`);
    this.name = "ApiError";
    this.status = status;
    this.retryAfter = retryAfter;
  }
}

/** `fetchFn` disuntikkan supaya bisa dites tanpa jaringan. */
export function createApi(fetchFn) {
  let token = null;

  async function request(method, path, query) {
    if (!token) throw new ApiError(401);
    const qs = query ? `?${new URLSearchParams(query)}` : "";
    let res;
    try {
      res = await fetchFn(path + qs, {
        method,
        headers: { authorization: `Bearer ${token}` },
        cache: "no-store",
        credentials: "omit",
        referrerPolicy: "no-referrer",
      });
    } catch {
      throw new ApiError(0);
    }
    if (!res.ok) throw new ApiError(res.status, parseRetryAfter(res.headers.get("retry-after")));
    try {
      return await res.json();
    } catch {
      throw new ApiError(500);
    }
  }

  return {
    hasToken: () => token !== null,
    /** Memakai token untuk masuk; token dibuang lagi kalau ditolak. */
    async login(candidate) {
      token = String(candidate).trim();
      try {
        return await request("POST", PATHS.login);
      } catch (e) {
        token = null;
        throw e;
      }
    },
    lock() {
      token = null;
    },
    overview: () => request("GET", PATHS.overview),
    audit: (before, limit) => {
      const q = { limit: String(limit) };
      if (before != null) q.before = String(before);
      return request("GET", PATHS.audit, q);
    },
  };
}
