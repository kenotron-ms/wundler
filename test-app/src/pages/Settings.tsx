import React, { useState } from 'react';
import Button from '../components/Button';
import { isValidEmail, isValidUsername } from '../utils/validators';

interface SettingsState {
  username: string;
  email: string;
  notifications: boolean;
  theme: 'light' | 'dark' | 'system';
  chunkingStrategy: 'route-based' | 'pgo' | 'package-based';
}

export default function Settings() {
  const [settings, setSettings] = useState<SettingsState>({
    username: 'dev',
    email: 'dev@example.com',
    notifications: true,
    theme: 'light',
    chunkingStrategy: 'route-based',
  });
  const [saved, setSaved] = useState(false);
  const [errors, setErrors] = useState<Partial<Record<keyof SettingsState, string>>>({});

  const handleSave = () => {
    const newErrors: typeof errors = {};
    if (!isValidEmail(settings.email)) newErrors.email = 'Invalid email';
    if (!isValidUsername(settings.username)) newErrors.username = 'Username must be 3-20 alphanumeric chars';
    setErrors(newErrors);
    if (Object.keys(newErrors).length === 0) {
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    }
  };

  return (
    <div style={{ background: 'white', borderRadius: '12px', padding: '32px', boxShadow: '0 1px 3px rgba(0,0,0,0.1)', maxWidth: '600px' }}>
      <h2 style={{ fontSize: '20px', fontWeight: '600', marginBottom: '24px', color: '#1f2937' }}>Settings</h2>

      <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
        {[
          { label: 'Username', key: 'username' as const, type: 'text' },
          { label: 'Email', key: 'email' as const, type: 'email' },
        ].map(({ label, key, type }) => (
          <div key={key}>
            <label style={{ display: 'block', fontSize: '14px', fontWeight: '500', color: '#374151', marginBottom: '4px' }}>{label}</label>
            <input
              type={type}
              value={settings[key] as string}
              onChange={e => setSettings(s => ({ ...s, [key]: e.target.value }))}
              style={{ width: '100%', padding: '8px 12px', border: `1px solid ${errors[key] ? '#ef4444' : '#d1d5db'}`, borderRadius: '6px' }}
            />
            {errors[key] && <p style={{ color: '#ef4444', fontSize: '12px', marginTop: '4px' }}>{errors[key]}</p>}
          </div>
        ))}

        <div>
          <label style={{ display: 'block', fontSize: '14px', fontWeight: '500', color: '#374151', marginBottom: '4px' }}>Chunking Strategy (Cloudpack)</label>
          <select
            value={settings.chunkingStrategy}
            onChange={e => setSettings(s => ({ ...s, chunkingStrategy: e.target.value as any }))}
            style={{ width: '100%', padding: '8px 12px', border: '1px solid #d1d5db', borderRadius: '6px' }}
          >
            <option value="route-based">Route-Based (Day 1)</option>
            <option value="pgo">PGO-Driven (Level 2)</option>
            <option value="package-based">Package-Based (Legacy)</option>
          </select>
        </div>

        <label style={{ display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}>
          <input
            type="checkbox"
            checked={settings.notifications}
            onChange={e => setSettings(s => ({ ...s, notifications: e.target.checked }))}
          />
          <span style={{ fontSize: '14px', color: '#374151' }}>Enable build notifications</span>
        </label>

        <Button variant="primary" onClick={handleSave}>
          {saved ? '✓ Saved' : 'Save Settings'}
        </Button>
      </div>
    </div>
  );
}
