// chunk: initial_root
import { createRoot } from 'react-dom/client';
import App from './App';
import './utils/analytics';
import { Suspense, useState, useEffect, lazy } from 'react';
import Navbar from './components/Navbar';
import Home from './pages/Home';
import { getCurrentRoute } from './router';
import { useStore } from './store';
import { formatTime, formatBytes } from './format';
import { navigate } from '../router';
import Button from './Button';
import React from 'react';
import { CheckIcon } from '../components/Icon';
import { formatDate } from '../utils/format';
import { isValidEmail } from '../utils/validators';

// --- test-app/src/main.tsx ---
const root = createRoot(document.getElementById('root'));
root.render(<App/>);
// --- test-app/src/App.tsx ---
const Dashboard = lazy(()=>import('./pages/Dashboard'));
const Settings = lazy(()=>import('./pages/Settings'));
function LoadingSpinner() {
    return (<div style={{
        display: 'flex',
        justifyContent: 'center',
        padding: '40px'
    }}>
      <div style={{
        fontSize: '24px'
    }}>Loading…</div>
    </div>);
}
default function App() {
    const [route, setRoute] = useState(getCurrentRoute());
    const { taskCount } = useStore();
    useEffect(()=>{
        const handler = ()=>setRoute(getCurrentRoute());
        window.addEventListener('hashchange', handler);
        return ()=>window.removeEventListener('hashchange', handler);
    }, []);
    return (<div style={{
        maxWidth: '1200px',
        margin: '0 auto'
    }}>
      <Navbar activeRoute={route} taskCount={taskCount}/>
      <main style={{
        padding: '24px'
    }}>
        <Suspense fallback={<LoadingSpinner/>}>
          {route === '/' && <Home/>}
          {route === '/dashboard' && <Dashboard/>}
          {route === '/settings' && <Settings/>}
        </Suspense>
      </main>
    </div>);
}
// --- test-app/src/utils/analytics.ts ---
window.analyticsQueue = [];
window.analyticsStats = {
    events: 0,
    bytesLogged: 0
};
function trackEvent(event, data = {}) {
    const entry = {
        event,
        data,
        time: formatTime(new Date())
    };
    window.analyticsQueue.push(entry);
    window.analyticsStats.events++;
    const bytes = JSON.stringify(entry).length;
    window.analyticsStats.bytesLogged += bytes;
    if (process.env.NODE_ENV !== 'production') {
        console.debug(`[analytics] ${event}`, data, formatBytes(bytes));
    }
}
function flushAnalytics() {
    const queue = window.analyticsQueue;
    window.analyticsQueue = [];
    console.info(`[analytics] Flushed ${queue.length} events`);
}
// --- test-app/src/components/Navbar.tsx ---
default function Navbar({ activeRoute, taskCount }) {
    const linkStyle = (route)=>({
            padding: '8px 16px',
            background: activeRoute === route ? '#2563eb' : 'transparent',
            color: activeRoute === route ? 'white' : '#374151',
            border: 'none',
            borderRadius: '6px',
            cursor: 'pointer',
            fontWeight: activeRoute === route ? '600' : '400'
        });
    return (<nav style={{
        display: 'flex',
        alignItems: 'center',
        gap: '8px',
        padding: '16px 24px',
        background: 'white',
        borderBottom: '1px solid #e5e7eb',
        boxShadow: '0 1px 3px rgba(0,0,0,0.1)'
    }}>
      <span style={{
        fontWeight: '700',
        fontSize: '18px',
        marginRight: '16px',
        color: '#1f2937'
    }}>
        Taskflow
      </span>
      <button style={linkStyle('/')} onClick={()=>navigate('/')}>Home</button>
      <button style={linkStyle('/dashboard')} onClick={()=>navigate('/dashboard')}>
        Dashboard {taskCount > 0 && <span style={{
        background: '#ef4444',
        color: 'white',
        borderRadius: '9999px',
        padding: '1px 7px',
        fontSize: '12px',
        marginLeft: '4px'
    }}>{taskCount}</span>}
      </button>
      <button style={linkStyle('/settings')} onClick={()=>navigate('/settings')}>Settings</button>
      <div style={{
        marginLeft: 'auto'
    }}>
        <Button variant="primary" onClick={()=>alert('New task!')}>+ New Task</Button>
      </div>
    </nav>);
}
// --- test-app/src/pages/Home.tsx ---
default function Home() {
    const [email, setEmail] = React.useState('');
    const [subscribed, setSubscribed] = React.useState(false);
    const features = [
        'Module graph maintained continuously — no full rebuild',
        'Bundles are derived views, not compiled artifacts',
        'Dead code eliminated via call-edge graph reachability',
        'Adaptive delivery: serve only what the client is missing',
        'Profile-guided chunk optimization from real usage data'
    ];
    const handleSubscribe = ()=>{
        if (isValidEmail(email)) {
            setSubscribed(true);
        } else {
            alert('Please enter a valid email');
        }
    };
    return (<div>
      <div style={{
        background: 'white',
        borderRadius: '12px',
        padding: '32px',
        marginBottom: '24px',
        boxShadow: '0 1px 3px rgba(0,0,0,0.1)'
    }}>
        <h1 style={{
        fontSize: '28px',
        fontWeight: '700',
        color: '#1f2937',
        marginBottom: '8px'
    }}>
          Welcome to Taskflow
        </h1>
        <p style={{
        color: '#6b7280',
        marginBottom: '24px'
    }}>
          A demo app validating the Wundler module graph bundler. Today: {formatDate(new Date())}
        </p>
        <ul style={{
        listStyle: 'none',
        display: 'flex',
        flexDirection: 'column',
        gap: '10px'
    }}>
          {features.map((f, i)=>(<li key={i} style={{
            display: 'flex',
            alignItems: 'center',
            gap: '10px',
            color: '#374151'
        }}>
              <span style={{
            color: '#16a34a',
            flexShrink: 0
        }}><CheckIcon size={18}/></span>
              {f}
            </li>))}
        </ul>
      </div>

      <div style={{
        background: 'white',
        borderRadius: '12px',
        padding: '24px',
        boxShadow: '0 1px 3px rgba(0,0,0,0.1)'
    }}>
        <h2 style={{
        fontSize: '18px',
        fontWeight: '600',
        marginBottom: '12px',
        color: '#1f2937'
    }}>
          Stay updated
        </h2>
        {subscribed ? (<p style={{
        color: '#16a34a'
    }}>✓ Subscribed! We'll be in touch.</p>) : (<div style={{
        display: 'flex',
        gap: '8px'
    }}>
            <input type="email" value={email} onChange={(e)=>setEmail(e.target.value)} placeholder="your@email.com" style={{
        flex: 1,
        padding: '8px 12px',
        border: '1px solid #d1d5db',
        borderRadius: '6px',
        fontSize: '14px'
    }}/>
            <button onClick={handleSubscribe} style={{
        padding: '8px 16px',
        background: '#2563eb',
        color: 'white',
        border: 'none',
        borderRadius: '6px',
        cursor: 'pointer'
    }}>
              Subscribe
            </button>
          </div>)}
      </div>
    </div>);
}
// --- test-app/src/router.ts ---
function getCurrentRoute() {
    const hash = window.location.hash.replace('#', '') || '/';
    if (hash === '/dashboard') return '/dashboard';
    if (hash === '/settings') return '/settings';
    return '/';
}
function navigate(route) {
    window.location.hash = route;
}
