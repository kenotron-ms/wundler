// chunk: lazy_1
import { useState } from 'react';
import { CheckIcon, TrashIcon } from '../components/Icon';
import Button from '../components/Button';
import { useStore } from '../store';
import { formatDate } from '../utils/format';

// --- test-app/src/pages/Dashboard.tsx ---
const priorityColors = {
    low: '#6b7280',
    medium: '#f59e0b',
    high: '#ef4444'
};
default function Dashboard() {
    const { tasks, addTask, toggleTask, removeTask } = useStore();
    const [newTaskTitle, setNewTaskTitle] = useState('');
    const [newTaskPriority, setNewTaskPriority] = useState('medium');
    const handleAdd = ()=>{
        if (newTaskTitle.trim()) {
            addTask(newTaskTitle.trim(), newTaskPriority);
            setNewTaskTitle('');
        }
    };
    return (<div style={{
        display: 'flex',
        flexDirection: 'column',
        gap: '16px'
    }}>
      <div style={{
        background: 'white',
        borderRadius: '12px',
        padding: '24px',
        boxShadow: '0 1px 3px rgba(0,0,0,0.1)'
    }}>
        <h2 style={{
        fontSize: '20px',
        fontWeight: '600',
        marginBottom: '16px',
        color: '#1f2937'
    }}>Add Task</h2>
        <div style={{
        display: 'flex',
        gap: '8px'
    }}>
          <input value={newTaskTitle} onChange={(e)=>setNewTaskTitle(e.target.value)} onKeyDown={(e)=>e.key === 'Enter' && handleAdd()} placeholder="Task title…" style={{
        flex: 1,
        padding: '8px 12px',
        border: '1px solid #d1d5db',
        borderRadius: '6px'
    }}/>
          <select value={newTaskPriority} onChange={(e)=>setNewTaskPriority(e.target.value)} style={{
        padding: '8px 12px',
        border: '1px solid #d1d5db',
        borderRadius: '6px'
    }}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>
          <Button variant="primary" onClick={handleAdd}>Add</Button>
        </div>
      </div>

      <div style={{
        background: 'white',
        borderRadius: '12px',
        padding: '24px',
        boxShadow: '0 1px 3px rgba(0,0,0,0.1)'
    }}>
        <h2 style={{
        fontSize: '20px',
        fontWeight: '600',
        marginBottom: '16px',
        color: '#1f2937'
    }}>
          Tasks ({tasks.filter((t)=>!t.done).length} remaining)
        </h2>
        <div style={{
        display: 'flex',
        flexDirection: 'column',
        gap: '8px'
    }}>
          {tasks.map((task)=>(<div key={task.id} style={{
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            padding: '12px',
            border: '1px solid #e5e7eb',
            borderRadius: '8px',
            opacity: task.done ? 0.6 : 1,
            background: task.done ? '#f9fafb' : 'white'
        }}>
              <button onClick={()=>toggleTask(task.id)} style={{
            width: 24,
            height: 24,
            borderRadius: '50%',
            border: '2px solid',
            borderColor: task.done ? '#16a34a' : '#d1d5db',
            background: task.done ? '#16a34a' : 'white',
            cursor: 'pointer',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'white',
            flexShrink: 0
        }}>
                {task.done && <CheckIcon size={12}/>}
              </button>
              <span style={{
            flex: 1,
            textDecoration: task.done ? 'line-through' : 'none',
            color: '#374151'
        }}>
                {task.title}
              </span>
              <span style={{
            fontSize: '12px',
            fontWeight: '600',
            color: priorityColors[task.priority]
        }}>
                {task.priority}
              </span>
              <span style={{
            fontSize: '11px',
            color: '#9ca3af'
        }}>
                {formatDate(new Date())}
              </span>
              <button onClick={()=>removeTask(task.id)} style={{
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            color: '#9ca3af',
            padding: '4px'
        }}>
                <TrashIcon size={14}/>
              </button>
            </div>))}
        </div>
      </div>
    </div>);
}
