// chunk: commons
import { useReducer, useCallback } from 'react';

// --- test-app/src/components/Button.tsx ---
const variantStyles = {
    primary: {
        background: '#2563eb',
        color: 'white',
        border: 'none'
    },
    secondary: {
        background: 'white',
        color: '#374151',
        border: '1px solid #d1d5db'
    },
    danger: {
        background: '#ef4444',
        color: 'white',
        border: 'none'
    }
};
const sizeStyles = {
    sm: {
        padding: '4px 10px',
        fontSize: '13px'
    },
    md: {
        padding: '8px 16px',
        fontSize: '14px'
    },
    lg: {
        padding: '12px 24px',
        fontSize: '16px'
    }
};
default function Button({ children, onClick, variant = 'secondary', disabled = false, size = 'md' }) {
    return (<button onClick={onClick} disabled={disabled} style={{
        ...variantStyles[variant],
        ...sizeStyles[size],
        borderRadius: '6px',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        fontWeight: '500',
        transition: 'opacity 0.15s'
    }}>
      {children}
    </button>);
}
// --- test-app/src/utils/validators.ts ---
function isValidEmail(email) {
    return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}
function isValidUsername(username) {
    return /^[a-zA-Z0-9]{3,20}$/.test(username);
}
function isNonEmpty(value) {
    return value.trim().length > 0;
}
function isValidUrl(url) {
    try {
        new URL(url);
        return true;
    } catch  {
        return false;
    }
}
// --- test-app/src/components/Icon.tsx ---
function CheckIcon({ size = 16 }) {
    return (<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5}>
      <polyline points="20 6 9 17 4 12"/>
    </svg>);
}
function TrashIcon({ size = 16 }) {
    return (<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
      <polyline points="3 6 5 6 21 6"/>
      <path d="M19 6l-1 14H6L5 6"/>
      <path d="M10 11v6M14 11v6"/>
    </svg>);
}
function CloseIcon({ size = 16 }) {
    return (<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
      <line x1="18" y1="6" x2="6" y2="18"/>
      <line x1="6" y1="6" x2="18" y2="18"/>
    </svg>);
}
// --- test-app/src/store.ts ---
const initialState = {
    tasks: [
        {
            id: '1',
            title: 'Set up Wundler workspace',
            done: true,
            priority: 'high'
        },
        {
            id: '2',
            title: 'Implement module summarizer',
            done: false,
            priority: 'high'
        },
        {
            id: '3',
            title: 'Build graph analyzer',
            done: false,
            priority: 'medium'
        },
        {
            id: '4',
            title: 'Write integration tests',
            done: false,
            priority: 'medium'
        },
        {
            id: '5',
            title: 'Validate against real app',
            done: false,
            priority: 'low'
        }
    ]
};
function reducer(state, action) {
    switch(action.type){
        case 'ADD_TASK':
            return {
                ...state,
                tasks: [
                    ...state.tasks,
                    {
                        ...action.payload,
                        id: Date.now().toString()
                    }
                ]
            };
        case 'TOGGLE_TASK':
            return {
                ...state,
                tasks: state.tasks.map((t)=>t.id === action.payload ? {
                        ...t,
                        done: !t.done
                    } : t)
            };
        case 'REMOVE_TASK':
            return {
                ...state,
                tasks: state.tasks.filter((t)=>t.id !== action.payload)
            };
        default:
            return state;
    }
}
function useStore() {
    const [state, dispatch] = useReducer(reducer, initialState);
    const addTask = useCallback((title, priority = 'medium')=>{
        dispatch({
            type: 'ADD_TASK',
            payload: {
                title,
                priority,
                done: false
            }
        });
    }, []);
    const toggleTask = useCallback((id)=>{
        dispatch({
            type: 'TOGGLE_TASK',
            payload: id
        });
    }, []);
    const removeTask = useCallback((id)=>{
        dispatch({
            type: 'REMOVE_TASK',
            payload: id
        });
    }, []);
    return {
        tasks: state.tasks,
        taskCount: state.tasks.filter((t)=>!t.done).length,
        addTask,
        toggleTask,
        removeTask
    };
}
// --- test-app/src/utils/format.ts ---
function formatDate(date) {
    return date.toLocaleDateString('en-US', {
        month: 'short',
        day: 'numeric',
        year: 'numeric'
    });
}
function formatTime(date) {
    return date.toLocaleTimeString('en-US', {
        hour: '2-digit',
        minute: '2-digit'
    });
}
function formatPhone(phone) {
    const cleaned = phone.replace(/\D/g, '');
    const match = cleaned.match(/^(\d{3})(\d{3})(\d{4})$/);
    if (match) return `(${match[1]}) ${match[2]}-${match[3]}`;
    return phone;
}
function formatBytes(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
