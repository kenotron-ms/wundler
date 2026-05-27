import { createContext } from 'react';
import type { FC } from 'react';

export const VERSION = '1.0.0';
export function greet(name: string): string { return `Hello, ${name}`; }
export class Greeter { greet(name: string): string { return `Hello, ${name}`; } }
export default function main() { console.log('main'); }
