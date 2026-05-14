"use strict";
(() => {
  // src/cache.ts
  var CACHE_NAME = "wundler-v1";
  var STATIC_MANIFEST_KEY = "wundler:static-manifest";
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
    } catch {
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

  // src/sw.ts
  async function installHandler(config) {
    try {
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
        } catch {
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
        } catch {
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
