/**
 * Notifies all connected clients that the service worker has observed a new
 * build_id from ABS.  Clients can listen for this message and reload or
 * otherwise react to the deployment.
 *
 * Per-client errors are swallowed — a client may have navigated away between
 * the matchAll() call and the postMessage() call.
 */
export async function notifyBuildIdChanged(newBuildId: string): Promise<void> {
  type ClientsLike = {
    matchAll(): Promise<Array<{ postMessage(msg: unknown): void }>>;
  };

  const c = (globalThis as Record<string, unknown>).clients as ClientsLike | undefined;
  if (!c?.matchAll) return;

  const clientList = await c.matchAll();

  for (const client of clientList) {
    try {
      client.postMessage({ type: 'cloudpack:build-id-changed', build_id: newBuildId });
    } catch {
      // Client may be gone; swallow error.
    }
  }
}
