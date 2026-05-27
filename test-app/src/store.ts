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
    { id: '1', title: 'Set up Cloudpack workspace', done: true, priority: 'high' },
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
