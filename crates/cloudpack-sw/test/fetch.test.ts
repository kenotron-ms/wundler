import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { handleNavigation } from '../src/sw.js';
import { STATIC_MANIFEST_KEY } from '../src/cache.js';
import { makeStubCache } from './helpers.js';
import type { StaticManifest, ManifestResponse, SwConfig } from '../src/types.js';

const CDN_BASE = 'https://cdn.example.com';
const ABS_BASE = 'https://abs.example.com';

/** Minimal static manifest fixture: one chunk, one entry point. */
const STATIC: StaticManifest = {
  build_id: 'b1',
  chunks: [{ id: 'shell', hash: 'aaaa1111deadbeef', modules: ['module-a', 'module-b'] }],
  entry_chunks: { home: ['shell'] },
  module_index: {},
};

/** Chunk URL derived from hash.slice(0, 8) */
const CHUNK_URL = `${CDN_BASE}/chunks/aaaa1111.js`;

describe('handleNavigation', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  it('calls ABS with current cached hashes and fetches missing chunks', async () => {
    const cache = makeStubCache();

    // Pre-load static manifest in cache (no chunks cached yet).
    await cache.put(STATIC_MANIFEST_KEY, new Response(JSON.stringify(STATIC)));

    const deltaResponse: ManifestResponse = {
      build_id: 'b1',
      fetch_urls: [CHUNK_URL],
      prefetch_urls: [],
      ttl: 60,
    };

    const stubFetch = vi.fn(async (url: string, _opts?: RequestInit): Promise<Response> => {
      if (url === `${ABS_BASE}/manifest`) {
        return new Response(JSON.stringify(deltaResponse), { status: 200 });
      }
      if (url === CHUNK_URL) {
        return new Response('/* chunk */', { status: 200 });
      }
      return new Response(null, { status: 404 });
    });
    (globalThis as Record<string, unknown>).fetch = stubFetch;

    const config: SwConfig = { cdnBaseUrl: CDN_BASE, absBaseUrl: ABS_BASE, cache };
    await handleNavigation(config, 'home');

    // Should have POSTed to ABS /manifest with correct body.
    const absCall = stubFetch.mock.calls.find(([url]) => url === `${ABS_BASE}/manifest`);
    expect(absCall).toBeDefined();
    const [, absOpts] = absCall!;
    expect((absOpts as RequestInit).method).toBe('POST');
    const body = JSON.parse((absOpts as RequestInit).body as string) as Record<string, unknown>;
    expect(body['entry_point']).toBe('home');
    expect(Array.isArray(body['cached_hashes'])).toBe(true);

    // Should have fetched the chunk and stored it in cache.
    expect(stubFetch).toHaveBeenCalledWith(CHUNK_URL);
    expect(cache.put).toHaveBeenCalledWith(CHUNK_URL, expect.any(Response));
  });

  it('falls back to static manifest when ABS times out', async () => {
    const cache = makeStubCache();

    // Pre-load static manifest in cache.
    await cache.put(STATIC_MANIFEST_KEY, new Response(JSON.stringify(STATIC)));

    const stubFetch = vi.fn(async (url: string, opts?: RequestInit): Promise<Response> => {
      if (url === `${ABS_BASE}/manifest`) {
        // Hang forever — simulate ABS timeout.
        return new Promise<Response>((_, reject) => {
          // React to abort signal so Promise.race can time out.
          if (opts?.signal) {
            opts.signal.addEventListener('abort', () => {
              reject(new DOMException('AbortError', 'AbortError'));
            });
          }
          // Never resolves on its own.
        });
      }
      if (url === CHUNK_URL) {
        return new Response('/* chunk */', { status: 200 });
      }
      return new Response(null, { status: 404 });
    });
    (globalThis as Record<string, unknown>).fetch = stubFetch;

    const config: SwConfig = { cdnBaseUrl: CDN_BASE, absBaseUrl: ABS_BASE, cache };
    const urls = await handleNavigation(config, 'home', { absTimeoutMs: 50 });

    // Static fallback should produce the shell chunk URL.
    expect(urls).toContain(CHUNK_URL);

    // The chunk should have been fetched and cached.
    expect(cache.put).toHaveBeenCalledWith(CHUNK_URL, expect.any(Response));
  });
});
