import type { CacheLike } from './types.js';

export const CACHE_NAME = 'wundler-v1';
export const STATIC_MANIFEST_KEY = 'wundler:static-manifest';

export async function getCache(injected?: CacheLike): Promise<CacheLike> {
  if (injected) return injected;
  const cache = await caches.open(CACHE_NAME);
  return cache as unknown as CacheLike;
}
