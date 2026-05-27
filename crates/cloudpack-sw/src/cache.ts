import type { CacheLike } from './types.js';

export const CACHE_NAME = 'cloudpack-v1';
export const STATIC_MANIFEST_KEY = 'cloudpack:static-manifest';

export async function getCache(injected?: CacheLike): Promise<CacheLike> {
  if (injected) return injected;
  const cache = await caches.open(CACHE_NAME);
  return cache as unknown as CacheLike;
}
