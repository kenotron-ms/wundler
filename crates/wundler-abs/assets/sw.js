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

  // src/sw.ts
  async function installHandler(config) {
    try {
      const response = await fetch(`${config.cdnBaseUrl}/manifest.json`, {
        cache: "no-cache"
      });
      if (response.ok) {
        const cache = await getCache(config.cache);
        await cache.put(STATIC_MANIFEST_KEY, response);
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
  }
})();
