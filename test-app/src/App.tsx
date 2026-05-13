import React, { Suspense, useState, useEffect, lazy } from 'react';
import Navbar from './components/Navbar';
import Home from './pages/Home';
import { getCurrentRoute, type Route } from './router';
import { useStore } from './store';

// Lazy-loaded pages — these create separate chunks
const Dashboard = lazy(() => import('./pages/Dashboard'));
const Settings = lazy(() => import('./pages/Settings'));

function LoadingSpinner() {
  return (
    <div style={{ display: 'flex', justifyContent: 'center', padding: '40px' }}>
      <div style={{ fontSize: '24px' }}>Loading…</div>
    </div>
  );
}

export default function App() {
  const [route, setRoute] = useState<Route>(getCurrentRoute());
  const { taskCount } = useStore();

  useEffect(() => {
    const handler = () => setRoute(getCurrentRoute());
    window.addEventListener('hashchange', handler);
    return () => window.removeEventListener('hashchange', handler);
  }, []);

  return (
    <div style={{ maxWidth: '1200px', margin: '0 auto' }}>
      <Navbar activeRoute={route} taskCount={taskCount} />
      <main style={{ padding: '24px' }}>
        <Suspense fallback={<LoadingSpinner />}>
          {route === '/' && <Home />}
          {route === '/dashboard' && <Dashboard />}
          {route === '/settings' && <Settings />}
        </Suspense>
      </main>
    </div>
  );
}
