import { formatTime, formatBytes } from './format';

// MODULE-LEVEL SIDE EFFECT — MUST NOT be tree-shaken
// This module writes to window on import. Cloudpack must mark it as SideEffectMarker::Definite
declare global {
  interface Window {
    analyticsQueue: Array<{ event: string; data: unknown; time: string }>;
    analyticsStats: { events: number; bytesLogged: number };
  }
}

window.analyticsQueue = [];
window.analyticsStats = { events: 0, bytesLogged: 0 };

export function trackEvent(event: string, data: unknown = {}): void {
  const entry = { event, data, time: formatTime(new Date()) };
  window.analyticsQueue.push(entry);
  window.analyticsStats.events++;
  const bytes = JSON.stringify(entry).length;
  window.analyticsStats.bytesLogged += bytes;
  if (process.env.NODE_ENV !== 'production') {
    console.debug(`[analytics] ${event}`, data, formatBytes(bytes));
  }
}

export function flushAnalytics(): void {
  const queue = window.analyticsQueue;
  window.analyticsQueue = [];
  // In a real app, this would POST to an analytics endpoint
  console.info(`[analytics] Flushed ${queue.length} events`);
}
