import { useSyncExternalStore } from "react";

const AUTH_KEY = "spinploy-api-key";

// Store listeners for useSyncExternalStore
const listeners = new Set<() => void>();

function notifyListeners() {
	for (const listener of listeners) {
		listener();
	}
}

export function getApiKey(): string | null {
	return localStorage.getItem(AUTH_KEY);
}

export function setApiKey(key: string): void {
	localStorage.setItem(AUTH_KEY, key);
	notifyListeners();
}

export function clearApiKey(): void {
	localStorage.removeItem(AUTH_KEY);
	notifyListeners();
}

function subscribe(listener: () => void): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

function getSnapshot(): string | null {
	return getApiKey();
}

export function useApiKey(): string | null {
	return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
