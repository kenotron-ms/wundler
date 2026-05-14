// chunk: commons

// module: test-app/src/components/Button.tsx
(function() {
import React from 'react';

interface ButtonProps {
  children: React.ReactNode;
  onClick?: () => void;
  variant?: 'primary' | 'secondary' | 'danger';
  disabled?: boolean;
  size?: 'sm' | 'md' | 'lg';
}

const variantStyles: Record<string, React.CSSProperties> = {
  primary: { background: '#2563eb', color: 'white', border: 'none' },
  secondary: { background: 'white', color: '#374151', border: '1px solid #d1d5db' },
  danger: { background: '#ef4444', color: 'white', border: 'none' },
};

const sizeStyles: Record<string, React.CSSProperties> = {
  sm: { padding: '4px 10px', fontSize: '13px' },
  md: { padding: '8px 16px', fontSize: '14px' },
  lg: { padding: '12px 24px', fontSize: '16px' },
};

export default function Button({ children, onClick, variant = 'secondary', disabled = false, size = 'md' }: ButtonProps) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        ...variantStyles[variant],
        ...sizeStyles[size],
        borderRadius: '6px',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        fontWeight: '500',
        transition: 'opacity 0.15s',
      }}
    >
      {children}
    </button>
  );
}

})();

// module: test-app/src/utils/validators.ts
(function() {
// All exports ARE used (in Home.tsx and Settings.tsx)

export function isValidEmail(email: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

export function isValidUsername(username: string): boolean {
  return /^[a-zA-Z0-9]{3,20}$/.test(username);
}

export function isNonEmpty(value: string): boolean {
  return value.trim().length > 0;
}

export function isValidUrl(url: string): boolean {
  try {
    new URL(url);
    return true;
  } catch {
    return false;
  }
}

})();

// module: test-app/src/components/Icon.tsx
(function() {
import React from 'react';

// CheckIcon IS used (in Home.tsx and Dashboard.tsx)
export function CheckIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5}>
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}

// TrashIcon IS used (in Dashboard.tsx)
export function TrashIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6l-1 14H6L5 6" />
      <path d="M10 11v6M14 11v6" />
    </svg>
  );
}

// CloseIcon is DEAD — never imported anywhere (for wundler DCE testing)
export function CloseIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}

})();

// module: test-app/src/store.ts
(function() {
import { useReducer, useCallback } from 'react';

export interface Task {
  id: string;
  title: string;
  done: boolean;
  priority: 'low' | 'medium' | 'high';
}

interface State {
  tasks: Task[];
}

type Action =
  | { type: 'ADD_TASK'; payload: Omit<Task, 'id'> }
  | { type: 'TOGGLE_TASK'; payload: string }
  | { type: 'REMOVE_TASK'; payload: string };

const initialState: State = {
  tasks: [
    { id: '1', title: 'Set up Wundler workspace', done: true, priority: 'high' },
    { id: '2', title: 'Implement module summarizer', done: false, priority: 'high' },
    { id: '3', title: 'Build graph analyzer', done: false, priority: 'medium' },
    { id: '4', title: 'Write integration tests', done: false, priority: 'medium' },
    { id: '5', title: 'Validate against real app', done: false, priority: 'low' },
  ],
};

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case 'ADD_TASK':
      return {
        ...state,
        tasks: [...state.tasks, { ...action.payload, id: Date.now().toString() }],
      };
    case 'TOGGLE_TASK':
      return {
        ...state,
        tasks: state.tasks.map(t =>
          t.id === action.payload ? { ...t, done: !t.done } : t
        ),
      };
    case 'REMOVE_TASK':
      return {
        ...state,
        tasks: state.tasks.filter(t => t.id !== action.payload),
      };
    default:
      return state;
  }
}

export function useStore() {
  const [state, dispatch] = useReducer(reducer, initialState);

  const addTask = useCallback((title: string, priority: Task['priority'] = 'medium') => {
    dispatch({ type: 'ADD_TASK', payload: { title, priority, done: false } });
  }, []);

  const toggleTask = useCallback((id: string) => {
    dispatch({ type: 'TOGGLE_TASK', payload: id });
  }, []);

  const removeTask = useCallback((id: string) => {
    dispatch({ type: 'REMOVE_TASK', payload: id });
  }, []);

  return {
    tasks: state.tasks,
    taskCount: state.tasks.filter(t => !t.done).length,
    addTask,
    toggleTask,
    removeTask,
  };
}

})();

// module: test-app/src/utils/format.ts
(function() {
// formatDate IS used (in Home.tsx and Dashboard.tsx)
export function formatDate(date: Date): string {
  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

// formatTime IS used in analytics module
export function formatTime(date: Date): string {
  return date.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' });
}

// formatPhone is DEAD — never imported anywhere (wundler DCE test target)
export function formatPhone(phone: string): string {
  const cleaned = phone.replace(/\D/g, '');
  const match = cleaned.match(/^(\d{3})(\d{3})(\d{4})$/);
  if (match) return `(${match[1]}) ${match[2]}-${match[3]}`;
  return phone;
}

// formatBytes IS used (in analytics.ts)
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

})();
