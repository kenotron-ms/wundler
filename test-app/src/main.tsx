import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';

// Analytics side-effect — must not be tree-shaken
import './utils/analytics';

const root = createRoot(document.getElementById('root')!);
root.render(<App />);
