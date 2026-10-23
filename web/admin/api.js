// Klien API dashboard. Token admin hanya hidup di penutup (closure) ini: tidak di
// `window`, tidak di localStorage/sessionStorage, tidak di URL, tidak di log.

import { isId, parseRetryAfter } from "./logic.js";

/** Nama kunci = nama konstanta di `crates/api-types` (dicek oleh tes). `{id}` diisi `withId`. */
export const PATHS = {
  login: "/api/v1/admin/login",
  overview: "/api/v1/admin/overview",
  audit: "/api/v1/admin/audit",
  journal: "/api/v1/admin/journal",
  journalRefresh: "/api/v1/admin/journal/refresh",
  journalProfile: "/api/v1/admin/journal/profile",
  journalRepos: "/api/v1/admin/journal/repos",
  journalReposOrder: "/api/v1/admin/journal/repos/order",
  journalRepo: "/api/v1/admin/journal/repos/{id}",
  journalStack: "/api/v1/admin/journal/stack",
  journalStackOrder: "/api/v1/admin/journal/stack/order",
  journalStackItem: "/api/v1/admin/journal/stack/{id}",
  messages: "/api/v1/admin/messages",
  message: "/api/v1/admin/messages/{id}",
  messageRestore: "/api/v1/admin/messages/{id}/restore",
  messageReports: "/api/v1/admin/messages/{id}/reports",
};

/** Mengisi `{id}`; id selain bilangan bulat positif ditolak sebelum menyentuh jaringan. */
export function withId(template, id) {
  if (!isId(id)) throw new ApiError(400);
  return template.replace("{id}", String(id));
}

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

  async function request(method, path, { query, body } = {}) {
    if (!token) throw new ApiError(401);
    const qs = query ? `?${new URLSearchParams(query)}` : "";
    const headers = { authorization: `Bearer ${token}` };
    const init = { method, headers, cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer" };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    let res;
    try {
      res = await fetchFn(path + qs, init);
    } catch {
      throw new ApiError(0);
    }
    if (!res.ok) throw new ApiError(res.status, parseRetryAfter(res.headers.get("retry-after")));
    if (res.status === 204) return null;
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
      const query = { limit: String(limit) };
      if (before != null) query.before = String(before);
      return request("GET", PATHS.audit, { query });
    },

    // Journal
    journal: () => request("GET", PATHS.journal),
    journalRefresh: () => request("POST", PATHS.journalRefresh),
    setUsername: (github_username) => request("PUT", PATHS.journalProfile, { body: { github_username } }),
    addRepo: (repo_name, topic) => request("POST", PATHS.journalRepos, { body: { repo_name, topic: topic || null } }),
    editRepo: async (id, repo_name, topic) =>
      request("PUT", withId(PATHS.journalRepo, id), { body: { repo_name, topic: topic || null } }),
    deleteRepo: async (id) => request("DELETE", withId(PATHS.journalRepo, id)),
    orderRepos: (ids) => request("PUT", PATHS.journalReposOrder, { body: { ids } }),
    addStack: (name) => request("POST", PATHS.journalStack, { body: { name } }),
    editStack: async (id, name) => request("PUT", withId(PATHS.journalStackItem, id), { body: { name } }),
    deleteStack: async (id) => request("DELETE", withId(PATHS.journalStackItem, id)),
    orderStack: (ids) => request("PUT", PATHS.journalStackOrder, { body: { ids } }),

    // Guestbook
    messages: (filter, before, limit) => {
      const query = { filter, limit: String(limit) };
      if (before != null) query.before = String(before);
      return request("GET", PATHS.messages, { query });
    },
    hideMessage: async (id) => request("DELETE", withId(PATHS.message, id)),
    restoreMessage: async (id) => request("POST", withId(PATHS.messageRestore, id)),
    dismissReports: async (id) => request("DELETE", withId(PATHS.messageReports, id)),
  };
}
