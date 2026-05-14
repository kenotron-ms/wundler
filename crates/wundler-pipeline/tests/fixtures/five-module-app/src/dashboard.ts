import { greet } from './util';

export function renderDashboard(): string {
    return `<div class="dashboard">${greet('dashboard')}</div>`;
}
