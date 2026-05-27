import React from 'react';
import { CheckIcon } from '../components/Icon';
import { formatDate } from '../utils/format';
import { isValidEmail } from '../utils/validators';

export default function Home() {
  const [email, setEmail] = React.useState('');
  const [subscribed, setSubscribed] = React.useState(false);

  const features = [
    'Module graph maintained continuously — no full rebuild',
    'Bundles are derived views, not compiled artifacts',
    'Dead code eliminated via call-edge graph reachability',
    'Adaptive delivery: serve only what the client is missing',
    'Profile-guided chunk optimization from real usage data',
  ];

  const handleSubscribe = () => {
    if (isValidEmail(email)) {
      setSubscribed(true);
    } else {
      alert('Please enter a valid email');
    }
  };

  return (
    <div>
      <div style={{ background: 'white', borderRadius: '12px', padding: '32px', marginBottom: '24px', boxShadow: '0 1px 3px rgba(0,0,0,0.1)' }}>
        <h1 style={{ fontSize: '28px', fontWeight: '700', color: '#1f2937', marginBottom: '8px' }}>
          Welcome to Taskflow
        </h1>
        <p style={{ color: '#6b7280', marginBottom: '24px' }}>
          A demo app validating the Cloudpack module graph bundler. Today: {formatDate(new Date())}
        </p>
        <ul style={{ listStyle: 'none', display: 'flex', flexDirection: 'column', gap: '10px' }}>
          {features.map((f, i) => (
            <li key={i} style={{ display: 'flex', alignItems: 'center', gap: '10px', color: '#374151' }}>
              <span style={{ color: '#16a34a', flexShrink: 0 }}><CheckIcon size={18} /></span>
              {f}
            </li>
          ))}
        </ul>
      </div>

      <div style={{ background: 'white', borderRadius: '12px', padding: '24px', boxShadow: '0 1px 3px rgba(0,0,0,0.1)' }}>
        <h2 style={{ fontSize: '18px', fontWeight: '600', marginBottom: '12px', color: '#1f2937' }}>
          Stay updated
        </h2>
        {subscribed ? (
          <p style={{ color: '#16a34a' }}>✓ Subscribed! We'll be in touch.</p>
        ) : (
          <div style={{ display: 'flex', gap: '8px' }}>
            <input
              type="email"
              value={email}
              onChange={e => setEmail(e.target.value)}
              placeholder="your@email.com"
              style={{ flex: 1, padding: '8px 12px', border: '1px solid #d1d5db', borderRadius: '6px', fontSize: '14px' }}
            />
            <button onClick={handleSubscribe} style={{ padding: '8px 16px', background: '#2563eb', color: 'white', border: 'none', borderRadius: '6px', cursor: 'pointer' }}>
              Subscribe
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
