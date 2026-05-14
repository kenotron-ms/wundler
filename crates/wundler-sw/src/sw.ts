import type { SwConfig } from './types.js';
import { getCache, STATIC_MANIFEST_KEY } from './cache.js';

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
      await cache.put(STATIC_MANIFEST_KEY, response);
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
}
