import type { CacheLike, SwConfig, ManifestRequest, ManifestResponse, StaticManifest } from './types.js';
import { STATIC_MANIFEST_KEY } from './cache.js';

/** Default ABS request timeout. Static fallback is always the safety hatch. */
export const DEFAULT_ABS_TIMEOUT_MS = 100;

export interface NavigationOptions {
  absTimeoutMs?: number;
}

/**
 * Reads the static manifest from cache and inspects each chunk.
 * For each chunk whose bundle file is already in cache, adds all its module
 * hashes to the result so ABS can skip sending them.
 */
export async function collectCachedModuleHashes(
  cache: CacheLike,
  cdnBaseUrl: string,
): Promise<{ static_: StaticManifest | null; hashes: string[] }> {
  const manifestResponse = await cache.match(STATIC_MANIFEST_KEY);
  if (!manifestResponse) {
    return { static_: null, hashes: [] };
  }

  const static_: StaticManifest = (await manifestResponse.json()) as StaticManifest;
  const hashes: string[] = [];

  for (const chunk of static_.chunks) {
    const chunkUrl = `${cdnBaseUrl}/chunks/${chunk.hash.slice(0, 8)}.js`;
    const cached = await cache.match(chunkUrl);
    if (cached) {
      hashes.push(...chunk.modules);
    }
  }

  return { static_, hashes };
}

/**
 * POSTs to the ABS /manifest endpoint with an AbortController-based timeout.
 * Returns null on timeout, non-2xx response, or any exception.
 * The AbortController timer is always cleared in the finally block.
 */
export async function fetchDelta(
  config: SwConfig,
  request: ManifestRequest,
  timeoutMs: number,
): Promise<ManifestResponse | null> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const fetchPromise = fetch(`${config.absBaseUrl}/manifest`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
      signal: controller.signal,
    });

    // Race the real fetch against an abort-triggered sentinel that resolves null.
    const timeoutPromise = new Promise<null>((resolve) => {
      controller.signal.addEventListener('abort', () => resolve(null), { once: true });
    });

    const response = await Promise.race<Response | null>([fetchPromise, timeoutPromise]);

    if (!response || !response.ok) return null;
    return (await response.json()) as ManifestResponse;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Builds the full chunk URLs for a given entry point from the static manifest.
 * Used as the fallback when ABS is unavailable.
 */
export function staticFallbackUrls(
  static_: StaticManifest,
  entryPoint: string,
  cdnBaseUrl: string,
): string[] {
  const chunkIds = static_.entry_chunks[entryPoint] ?? [];
  return chunkIds.flatMap((id) => {
    const chunk = static_.chunks.find((c) => c.id === id);
    if (!chunk) return [];
    return [`${cdnBaseUrl}/chunks/${chunk.hash.slice(0, 8)}.js`];
  });
}
