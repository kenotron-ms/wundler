import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { installHandler, activateHandler } from '../src/sw.js';
import { STATIC_MANIFEST_KEY } from '../src/cache.js';
import { installSwGlobals, makeStubFetch, makeStubCache } from './helpers.js';

const CDN_BASE = 'https://cdn.example.com';

describe('installHandler', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    installSwGlobals();
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  it('fetches /manifest.json from CDN base and stores in cache', async () => {
    const manifest = {
      build_id: 'test-build',
      chunks: [],
      entry_chunks: {},
      module_index: {},
    };
    const stubFetch = makeStubFetch({ [`${CDN_BASE}/manifest.json`]: manifest });
    (globalThis as Record<string, unknown>).fetch = stubFetch;

    const cache = makeStubCache();
    await installHandler({ cdnBaseUrl: CDN_BASE, absBaseUrl: '', cache });

    expect(stubFetch).toHaveBeenCalledWith(`${CDN_BASE}/manifest.json`, {
      cache: 'no-cache',
    });
    expect(cache.put).toHaveBeenCalledWith(STATIC_MANIFEST_KEY, expect.any(Response));
  });

  it('survives manifest fetch failure (resolves, does not throw)', async () => {
    const stubFetch = makeStubFetch({}, { fail: true });
    (globalThis as Record<string, unknown>).fetch = stubFetch;

    const cache = makeStubCache();
    await expect(
      installHandler({ cdnBaseUrl: CDN_BASE, absBaseUrl: '', cache }),
    ).resolves.not.toThrow();
  });
});

describe('activateHandler', () => {
  it('claims all clients', async () => {
    const { clients } = installSwGlobals();

    await activateHandler();

    expect(clients.claim).toHaveBeenCalledOnce();
  });
});
