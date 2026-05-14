import { vi } from 'vitest';

/**
 * Sets up minimal SW globals on `globalThis`:
 *  - `self`    with a stubbed `skipWaiting` and `addEventListener`
 *  - `clients` with a stubbed `claim`
 *
 * Returns the stubs so tests can assert on them.
 */
export function installSwGlobals() {
  const mockSelf = {
    skipWaiting: vi.fn().mockResolvedValue(undefined),
    addEventListener: vi.fn(),
  };
  const mockClients = {
    claim: vi.fn().mockResolvedValue(undefined),
  };

  (globalThis as Record<string, unknown>).self = mockSelf;
  (globalThis as Record<string, unknown>).clients = mockClients;

  return { self: mockSelf, clients: mockClients };
}

/**
 * Returns a `vi.fn()` that behaves like `fetch`:
 *  - If `opts.fail` is true, always throws a network error.
 *  - Otherwise, looks up `url` in `routes` and returns a 200 Response with the
 *    JSON-serialised body, or a 404 Response when the route is not found.
 */
export function makeStubFetch(
  routes: Record<string, unknown>,
  opts?: { fail?: boolean },
) {
  return vi.fn(async (url: string, _options?: unknown): Promise<Response> => {
    if (opts?.fail) {
      throw new Error('network error');
    }
    const body = routes[url];
    if (body !== undefined) {
      return new Response(JSON.stringify(body), { status: 200 });
    }
    return new Response(null, { status: 404 });
  });
}

/**
 * Returns a minimal CacheLike stub backed by a `Map<string, Response>`.
 * All methods are wrapped in `vi.fn()` so tests can assert calls.
 */
export function makeStubCache() {
  const store = new Map<string, Response>();
  return {
    put: vi.fn(async (key: RequestInfo | URL, value: Response) => {
      store.set(String(key), value);
    }),
    match: vi.fn(async (key: RequestInfo | URL) => store.get(String(key))),
    keys: vi.fn(async () => [] as ReadonlyArray<Request>),
    delete: vi.fn(async (key: RequestInfo | URL) => store.delete(String(key))),
  };
}
