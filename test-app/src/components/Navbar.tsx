import React from 'react';
import { navigate, type Route } from '../router';
import Button from './Button';

interface NavbarProps {
  activeRoute: Route;
  taskCount: number;
}

export default function Navbar({ activeRoute, taskCount }: NavbarProps) {
  const linkStyle = (route: Route): React.CSSProperties => ({
    padding: '8px 16px',
    background: activeRoute === route ? '#2563eb' : 'transparent',
    color: activeRoute === route ? 'white' : '#374151',
    border: 'none',
    borderRadius: '6px',
    cursor: 'pointer',
    fontWeight: activeRoute === route ? '600' : '400',
  });

  return (
    <nav style={{
      display: 'flex', alignItems: 'center', gap: '8px',
      padding: '16px 24px', background: 'white',
      borderBottom: '1px solid #e5e7eb', boxShadow: '0 1px 3px rgba(0,0,0,0.1)'
    }}>
      <span style={{ fontWeight: '700', fontSize: '18px', marginRight: '16px', color: '#1f2937' }}>
        Taskflow
      </span>
      <button style={linkStyle('/')} onClick={() => navigate('/')}>Home</button>
      <button style={linkStyle('/dashboard')} onClick={() => navigate('/dashboard')}>
        Dashboard {taskCount > 0 && <span style={{ background: '#ef4444', color: 'white', borderRadius: '9999px', padding: '1px 7px', fontSize: '12px', marginLeft: '4px' }}>{taskCount}</span>}
      </button>
      <button style={linkStyle('/settings')} onClick={() => navigate('/settings')}>Settings</button>
      <div style={{ marginLeft: 'auto' }}>
        <Button variant="primary" onClick={() => alert('New task!')}>+ New Task</Button>
      </div>
    </nav>
  );
}
