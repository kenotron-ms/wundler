export type Route = '/' | '/dashboard' | '/settings';

export function getCurrentRoute(): Route {
  const hash = window.location.hash.replace('#', '') || '/';
  if (hash === '/dashboard') return '/dashboard';
  if (hash === '/settings') return '/settings';
  return '/';
}

export function navigate(route: Route): void {
  window.location.hash = route;
}
