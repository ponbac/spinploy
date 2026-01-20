import { fetchEventSource } from "@microsoft/fetch-event-source";
import { useQuery } from "@tanstack/react-query";
import type { PreviewDetailResponse, PreviewListResponse } from "./api-types";
import { clearApiKey, getApiKey } from "./auth";

const API_BASE_URL = import.meta.env.VITE_API_URL || "/api";

async function fetchWithAuth<T>(endpoint: string): Promise<T> {
	const apiKey = getApiKey() || "";
	const response = await fetch(`${API_BASE_URL}${endpoint}`, {
		headers: {
			"x-api-key": apiKey,
		},
	});

	if (response.status === 401) {
		clearApiKey();
		throw new Error("Unauthorized: Invalid API key");
	}

	if (!response.ok) {
		throw new Error(`API error: ${response.statusText}`);
	}

	return response.json();
}

// Query hooks
export function usePreviewsList() {
	return useQuery({
		queryKey: ["previews"],
		queryFn: () => fetchWithAuth<PreviewListResponse>("/previews"),
		refetchInterval: 5000, // Auto-refresh every 5 seconds
	});
}

export function usePreviewDetail(identifier: string) {
	return useQuery({
		queryKey: ["previews", identifier],
		queryFn: () =>
			fetchWithAuth<PreviewDetailResponse>(`/previews/${identifier}`),
		refetchInterval: 5000,
	});
}

// EventSource-compatible wrapper for fetch-event-source
export interface LogStreamEventSource {
	close: () => void;
	onopen: (() => void) | null;
	onmessage: ((event: { data: string }) => void) | null;
	onerror: (() => void) | null;
}

// SSE log streaming helper with authentication support
export function createLogStream(
	identifier: string,
	service: string,
	tail = 100,
	follow = true,
): LogStreamEventSource {
	const url = new URL(
		`${API_BASE_URL}/previews/${identifier}/containers/${service}/logs`,
		window.location.origin,
	);
	url.searchParams.set("tail", tail.toString());
	url.searchParams.set("follow", follow.toString());

	const controller = new AbortController();
	const eventSource: LogStreamEventSource = {
		close: () => controller.abort(),
		onopen: null,
		onmessage: null,
		onerror: null,
	};

	const apiKey = getApiKey() || "";

	// Start the fetch-event-source connection
	fetchEventSource(url.toString(), {
		headers: {
			"x-api-key": apiKey,
		},
		signal: controller.signal,
		onopen: async (response) => {
			if (response.status === 401) {
				clearApiKey();
				throw new Error("Unauthorized: Invalid API key");
			}
			if (response.ok) {
				eventSource.onopen?.();
			} else {
				throw new Error(`SSE connection failed: ${response.statusText}`);
			}
		},
		onmessage: (event) => {
			if (event.data) {
				eventSource.onmessage?.({ data: event.data });
			}
		},
		onerror: () => {
			eventSource.onerror?.();
			throw new Error("SSE connection error");
		},
	}).catch(() => {
		// Errors are already handled by onerror callback
	});

	return eventSource;
}

// SSE deployment log streaming helper
export function createDeploymentLogStream(
	identifier: string,
	deploymentId: string,
): LogStreamEventSource {
	const url = new URL(
		`${API_BASE_URL}/previews/${identifier}/deployments/${deploymentId}/logs`,
		window.location.origin,
	);

	const controller = new AbortController();
	const eventSource: LogStreamEventSource = {
		close: () => controller.abort(),
		onopen: null,
		onmessage: null,
		onerror: null,
	};

	const apiKey = getApiKey() || "";

	// Start the fetch-event-source connection
	fetchEventSource(url.toString(), {
		headers: {
			"x-api-key": apiKey,
		},
		signal: controller.signal,
		onopen: async (response) => {
			if (response.status === 401) {
				clearApiKey();
				throw new Error("Unauthorized: Invalid API key");
			}
			if (response.ok) {
				eventSource.onopen?.();
			} else {
				throw new Error(`SSE connection failed: ${response.statusText}`);
			}
		},
		onmessage: (event) => {
			if (event.data) {
				eventSource.onmessage?.({ data: event.data });
			}
		},
		onerror: () => {
			eventSource.onerror?.();
			throw new Error("SSE connection error");
		},
	}).catch(() => {
		// Errors are already handled by onerror callback
	});

	return eventSource;
}
