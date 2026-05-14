import type { SwConfig, ManifestRequest } from './types.js';
import { getCache } from './cache.js';
import {
  collectCachedModuleHashes,
  fetchDelta,
  staticFallbackUrls,
  DEFAULT_ABS_TIMEOUT_MS,
  type NavigationOptions,
} from './delta.js';

// Declare self as ServiceWorkerGlobalScope for type safety within this module.
declare const self: ServiceWorkerGlobalScope;

/**
 * Fetches the static manifest from the CDN and stores it in the cache.
 * Survives network failures by logging a warning without rethrowing.
 */
export async function installHandler(config: SwConfig): Promise<void> {
  try {
    const response = await fetch(`${config.cdnBaseUrl}/manifest.json`, {
      cache: 'no-cache',
    });
    if (response.ok) {
      const cache = await getCache(config.cache);
      await cache.put('wundler:static-manifest', response);
    }
  } catch (err) {
    console.warn('[wundler-sw] install: manifest fetch failed', err);
  }
}

/**
 * Claims all open clients so the new service worker takes control immediately.
 */
export async function activateHandler(): Promise<void> {
  // `clients` is a ServiceWorker global; access via globalThis so test stubs work.
  const c = (globalThis as Record<string, unknown>).clients as
    | { claim(): Promise<void> }
    | undefined;
  if (c?.claim) {
    await c.claim();
  }
}

/**
 * Pre-fetches the JS chunks needed for a navigation by querying ABS (or
 * falling back to the static manifest) and caching any missing files.
 * Returns the list of required chunk URLs.
 */
export async function handleNavigation(
  config: SwConfig,
  entryPoint: string,
  opts?: NavigationOptions,
): Promise<string[]> {
  const timeoutMs = opts?.absTimeoutMs ?? DEFAULT_ABS_TIMEOUT_MS;
  const cache = await getCache(config.cache);

  const { static_, hashes } = await collectCachedModuleHashes(cache, config.cdnBaseUrl);

  const request: ManifestRequest = {
    entry_point: entryPoint,
    cached_hashes: hashes,
    build_id: static_?.build_id,
  };

  const delta = await fetchDelta(config, request, timeoutMs);

  let requiredUrls: string[];
  let prefetchUrls: string[] = [];

  if (delta) {
    requiredUrls = delta.fetch_urls;
    prefetchUrls = delta.prefetch_urls;
  } else if (static_) {
    requiredUrls = staticFallbackUrls(static_, entryPoint, config.cdnBaseUrl);
  } else {
    requiredUrls = [];
  }

  // Fetch all required URLs into cache in parallel.
  // Skip URLs already in cache; only store responses that are ok.
  await Promise.all(
    requiredUrls.map(async (url) => {
      const existing = await cache.match(url);
      if (existing) return;
      const resp = await fetch(url);
      if (resp.ok) {
        await cache.put(url, resp);
      }
    }),
  );

  // Prefetch additional URLs fire-and-forget with the same skip-if-cached logic.
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
        // Ignore prefetch errors — they are best-effort.
      }
    })();
  }

  return requiredUrls;
}

// ---------------------------------------------------------------------------
// Auto-wire: register event listeners only in a real Service Worker context.
// `ServiceWorkerGlobalScope` is undefined in Node.js / Vitest environments,
// so this block is safely skipped during tests.
// ---------------------------------------------------------------------------
if (typeof ServiceWorkerGlobalScope !== 'undefined') {
  const CONFIG: SwConfig = {
    cdnBaseUrl: (globalThis as Record<string, unknown>).__WUNDLER_CDN__ as string ?? '',
    absBaseUrl: (globalThis as Record<string, unknown>).__WUNDLER_ABS__ as string ?? '',
  };

  self.addEventListener('install', (event) => {
    event.waitUntil(
      Promise.all([installHandler(CONFIG), self.skipWaiting()]).then(() => undefined),
    );
  });

  self.addEventListener('activate', (event) => {
    event.waitUntil(activateHandler());
  });

  self.addEventListener('fetch', (event) => {
    if (event.request.mode !== 'navigate') return;

    const url = new URL(event.request.url);
    const stripped = url.pathname.replace(/^\//, '');
    const entryPoint = stripped.replace(/\//g, '.') || 'home';

    event.respondWith(
      handleNavigation(CONFIG, entryPoint).then(() => fetch(event.request)),
    );
  });
}
