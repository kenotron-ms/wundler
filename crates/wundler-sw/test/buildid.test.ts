import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { handleNavigation } from '../src/sw.js';
import { STATIC_MANIFEST_KEY } from '../src/cache.js';
import { makeStubCache } from './helpers.js';
import type { StaticManifest, ManifestResponse, SwConfig } from '../src/types.js';

const CDN_BASE = 'https://cdn.example.com';
const ABS_BASE = 'https://abs.example.com';

/** Static manifest fixture with build_id 'b1'. */
const STATIC_OLD: StaticManifest = {
  build_id: 'b1',
  chunks: [],
  entry_chunks: { home: [] },
  module_index: {},
};

describe('build-id rotation', () => {
  let originalFetch: typeof globalThis.fetch;
  let originalClients: unknown;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
    originalClients = (globalThis as Record<string, unknown>).clients;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    (globalThis as Record<string, unknown>).clients = originalClients;
  });

  it('postMessages clients when ABS reports a new build_id', async () => {
    const cache = makeStubCache();
    await cache.put(STATIC_MANIFEST_KEY, new Response(JSON.stringify(STATIC_OLD)));

    const client1 = { postMessage: vi.fn() };
    const client2 = { postMessage: vi.fn() };
    const mockClients = {
      matchAll: vi.fn().mockResolvedValue([client1, client2]),
    };
    (globalThis as Record<string, unknown>).clients = mockClients;

    const deltaResponse: ManifestResponse = {
      build_id: 'b2-NEW',
      fetch_urls: [],
      prefetch_urls: [],
      ttl: 60,
    };

    const stubFetch = vi.fn(async (url: string, _opts?: RequestInit): Promise<Response> => {
      if (url === `${ABS_BASE}/manifest`) {
        return new Response(JSON.stringify(deltaResponse), { status: 200 });
      }
      return new Response(null, { status: 200 });
    });
    (globalThis as Record<string, unknown>).fetch = stubFetch;

    const config: SwConfig = { cdnBaseUrl: CDN_BASE, absBaseUrl: ABS_BASE, cache };
    await handleNavigation(config, 'home');

    expect(client1.postMessage).toHaveBeenCalledWith({
      type: 'wundler:build-id-changed',
      build_id: 'b2-NEW',
    });
    expect(client2.postMessage).toHaveBeenCalledWith({
      type: 'wundler:build-id-changed',
      build_id: 'b2-NEW',
    });
  });

  it('does NOT postMessage when ABS reports the same build_id', async () => {
    const cache = makeStubCache();
    await cache.put(STATIC_MANIFEST_KEY, new Response(JSON.stringify(STATIC_OLD)));

    const client1 = { postMessage: vi.fn() };
    const mockClients = {
      matchAll: vi.fn().mockResolvedValue([client1]),
    };
    (globalThis as Record<string, unknown>).clients = mockClients;

    const deltaResponse: ManifestResponse = {
      build_id: 'b1', // Same as STATIC_OLD.build_id — no rotation
      fetch_urls: [],
      prefetch_urls: [],
      ttl: 60,
    };

    const stubFetch = vi.fn(async (url: string, _opts?: RequestInit): Promise<Response> => {
      if (url === `${ABS_BASE}/manifest`) {
        return new Response(JSON.stringify(deltaResponse), { status: 200 });
      }
      return new Response(null, { status: 200 });
    });
    (globalThis as Record<string, unknown>).fetch = stubFetch;

    const config: SwConfig = { cdnBaseUrl: CDN_BASE, absBaseUrl: ABS_BASE, cache };
    await handleNavigation(config, 'home');

    expect(client1.postMessage).not.toHaveBeenCalled();
    expect(mockClients.matchAll).not.toHaveBeenCalled();
  });
});
