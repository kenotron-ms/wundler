export async function loadHeavy() {
    return await import('./heavy');
}

export async function loadByName(name: string) {
    return import(name);
}
