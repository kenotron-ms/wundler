"use strict";
(() => {
  // src/cache.ts
  var CACHE_NAME = "wundler-v1";
  var STATIC_MANIFEST_KEY = "wundler:static-manifest";

  // Client-side dedup for chunk error reports — prevent re-reporting the same
  // (build_id, chunk_id, error_type) triple within one SW lifecycle.
  const REPORTED_CHUNK_ERRORS = new Set();

  async function getCache(injected) {
    if (injected) return injected;
    const cache = await caches.open(CACHE_NAME);
    return cache;
  }

  // src/delta.ts
  var DEFAULT_ABS_TIMEOUT_MS = 100;
  async function collectCachedModuleHashes(cache, cdnBaseUrl) {
    const manifestResponse = await cache.match(STATIC_MANIFEST_KEY);
    if (!manifestResponse) {
      return { static_: null, hashes: [] };
    }
    const static_ = await manifestResponse.json();
    const hashes = [];
    for (const chunk of static_.chunks) {
      const chunkUrl = `${cdnBaseUrl}/chunks/${chunk.hash.slice(0, 8)}.js`;
      const cached = await cache.match(chunkUrl);
      if (cached) {
        hashes.push(...chunk.modules);
      }
    }
    return { static_, hashes };
  }
  async function fetchDelta(config, request, timeoutMs) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
      const fetchPromise = fetch(`${config.absBaseUrl}/manifest`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request),
        signal: controller.signal
      });
      const timeoutPromise = new Promise((resolve) => {
        controller.signal.addEventListener("abort", () => resolve(null), { once: true });
      });
      const response = await Promise.race([fetchPromise, timeoutPromise]);
      if (!response || !response.ok) return null;
      return await response.json();
    } catch (e) {
      const isTimeout = e instanceof Error && e.name === "AbortError";
      reportChunkError(config.absBaseUrl, {
        buildId: request.build_id ?? "",
        chunkId: "",
        url: `${config.absBaseUrl}/manifest`,
        errorType: isTimeout ? "network_timeout" : "load_failed",
        sessionId: "",
      });
      return null;
    } finally {
      clearTimeout(timer);
    }
  }
  function staticFallbackUrls(static_, entryPoint, cdnBaseUrl) {
    const chunkIds = static_.entry_chunks[entryPoint] ?? [];
    return chunkIds.flatMap((id) => {
      const chunk = static_.chunks.find((c) => c.id === id);
      if (!chunk) return [];
      return [`${cdnBaseUrl}/chunks/${chunk.hash.slice(0, 8)}.js`];
    });
  }

  // src/buildid.ts
  async function notifyBuildIdChanged(newBuildId) {
    const c = globalThis.clients;
    if (!c?.matchAll) return;
    const clientList = await c.matchAll();
    for (const client of clientList) {
      try {
        client.postMessage({ type: "wundler:build-id-changed", build_id: newBuildId });
      } catch {
      }
    }
  }

  // src/telemetry.ts
  /**
   * Fire-and-forget POST to /telemetry/chunk-error.
   * Never awaited. Never propagates errors. Uses keepalive:true to survive navigation.
   * Client-side dedup: same (buildId, chunkId, errorType) is only reported once per lifecycle.
   *
   * @param {string} absBaseUrl - Base URL of the ABS server.
   * @param {object} payload - { buildId, chunkId, url, errorType, sessionId }
   */
  function reportChunkError(absBaseUrl, payload) {
    const dedupeKey = `${payload.buildId}:${payload.chunkId}:${payload.errorType}`;
    if (REPORTED_CHUNK_ERRORS.has(dedupeKey)) {
      return; // already reported this triple in current SW lifecycle
    }
    REPORTED_CHUNK_ERRORS.add(dedupeKey);

    const body = JSON.stringify({
      build_id: payload.buildId || "",
      chunk_id: payload.chunkId || "",
      url: payload.url || "",
      error_type: payload.errorType,
      timestamp_ms: Date.now(),
      session_id: payload.sessionId || "",
    });

    try {
      void fetch(`${absBaseUrl}/telemetry/chunk-error`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body,
        keepalive: true,
      }).catch(() => {}); // explicit swallow — never propagate
    } catch {
      // fetch itself threw (e.g. network unavailable) — ignore
    }
  }

  // src/sw.ts
  async function installHandler(config) {
    try {
      // NOTE(security/signing-gap): This fetches manifest.json from the CDN and does
      // not verify the ed25519 signature. The ABS serves a signed manifest on
      // GET /manifest/full.json with X-Wundler-Signature header, but the SW
      // currently uses the CDN copy and skips verification entirely.
      //
      // Full verification requires:
      //   1. Fetch from ABS /manifest/full.json instead of CDN manifest.json
      //   2. Read the X-Wundler-Signature response header (base64 ed25519)
      //   3. Decode the header and call SubtleCrypto.verify() with the pinned
      //      public key and the canonical manifest bytes
      //   4. Reject the manifest if verification fails
      //
      // Tracked as: Security P2 — JS half (not yet implemented).
      // Until this is done, X-Wundler-Signature provides no tamper-evidence.
      const response = await fetch(`${config.cdnBaseUrl}/manifest.json`, {
        cache: "no-cache"
      });
      if (response.ok) {
        const cache = await getCache(config.cache);
        await cache.put("wundler:static-manifest", response);
      }
    } catch (err) {
      console.warn("[wundler-sw] install: manifest fetch failed", err);
    }
  }
  async function activateHandler() {
    const c = globalThis.clients;
    if (c?.claim) {
      await c.claim();
    }
  }
  async function handleNavigation(config, entryPoint, opts) {
    const timeoutMs = opts?.absTimeoutMs ?? DEFAULT_ABS_TIMEOUT_MS;
    const cache = await getCache(config.cache);
    const { static_, hashes } = await collectCachedModuleHashes(cache, config.cdnBaseUrl);
    const request = {
      entry_point: entryPoint,
      cached_hashes: hashes,
      build_id: static_?.build_id
    };
    const delta = await fetchDelta(config, request, timeoutMs);
    const buildId = static_?.build_id;
    if (delta && buildId && delta.build_id !== buildId) {
      await notifyBuildIdChanged(delta.build_id);
      void (async () => {
        try {
          const resp = await fetch(`${config.cdnBaseUrl}/manifest.json`, { cache: "no-cache" });
          if (resp.ok) {
            await cache.put(STATIC_MANIFEST_KEY, resp);
          }
        } catch (e) {
          reportChunkError(config.absBaseUrl, {
            buildId: buildId ?? "",
            chunkId: "",
            url: `${config.cdnBaseUrl}/manifest.json`,
            errorType: "load_failed",
            sessionId: "",
          });
        }
      })();
    }
    let requiredUrls;
    let prefetchUrls = [];
    if (delta) {
      requiredUrls = delta.fetch_urls;
      prefetchUrls = delta.prefetch_urls;
    } else if (static_) {
      requiredUrls = staticFallbackUrls(static_, entryPoint, config.cdnBaseUrl);
    } else {
      requiredUrls = [];
    }
    await Promise.all(
      requiredUrls.map(async (url) => {
        const existing = await cache.match(url);
        if (existing) return;
        const resp = await fetch(url);
        if (resp.ok) {
          await cache.put(url, resp);
        }
      })
    );
    for (const url of prefetchUrls) {
      void (async () => {
        try {
          const existing = await cache.match(url);
          if (existing) return;
          const resp = await fetch(url);
          if (resp.ok) {
            await cache.put(url, resp);
          }
        } catch (e) {
          const chunkId = url.split("/").pop()?.replace(/\.js$/, "") ?? "";
          reportChunkError(config.absBaseUrl, {
            buildId: buildId ?? "",
            chunkId,
            url,
            errorType: "load_failed",
            sessionId: "",
          });
        }
      })();
    }
    return requiredUrls;
  }
  if (typeof ServiceWorkerGlobalScope !== "undefined") {
    const CONFIG = {
      cdnBaseUrl: globalThis.__WUNDLER_CDN__ ?? "",
      absBaseUrl: globalThis.__WUNDLER_ABS__ ?? ""
    };
    self.addEventListener("install", (event) => {
      event.waitUntil(
        Promise.all([installHandler(CONFIG), self.skipWaiting()]).then(() => void 0)
      );
    });
    self.addEventListener("activate", (event) => {
      event.waitUntil(activateHandler());
    });
    self.addEventListener("fetch", (event) => {
      if (event.request.mode !== "navigate") return;
      const url = new URL(event.request.url);
      const stripped = url.pathname.replace(/^\//, "");
      const entryPoint = stripped.replace(/\//g, ".") || "home";
      event.respondWith(
        handleNavigation(CONFIG, entryPoint).then(() => fetch(event.request))
      );
    });
  }
})();
