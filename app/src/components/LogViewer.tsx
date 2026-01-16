import { Pause, Play, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createLogStream, type LogStreamEventSource } from "@/lib/api-client";

const MAX_LOG_LINES = 2500;

type LogEntry = { id: number; text: string };

export default function LogViewer(props: {
	identifier: string;
	service: string;
}) {
	const [logs, setLogs] = useState<LogEntry[]>([]);
	const [isPaused, setIsPaused] = useState(false);
	const [isConnected, setIsConnected] = useState(false);
	const logEndRef = useRef<HTMLDivElement>(null);
	const eventSourceRef = useRef<LogStreamEventSource | null>(null);
	const containerRef = useRef<HTMLDivElement>(null);
	const isPausedRef = useRef(false);
	const nextIdRef = useRef(0);

	// Keep ref in sync with state for use in SSE callback
	isPausedRef.current = isPaused;

	// Auto-scroll to bottom when new logs arrive (unless manually scrolled up)
	// biome-ignore lint/correctness/useExhaustiveDependencies: logs.length triggers scroll on new logs
	useEffect(() => {
		if (!isPaused && logEndRef.current && containerRef.current) {
			const container = containerRef.current;
			const isScrolledToBottom =
				container.scrollHeight - container.scrollTop <=
				container.clientHeight + 100;

			if (isScrolledToBottom) {
				logEndRef.current.scrollIntoView({ behavior: "smooth" });
			}
		}
	}, [isPaused, logs.length]);

	// Connect to SSE stream
	useEffect(() => {
		const eventSource = createLogStream(
			props.identifier,
			props.service,
			100,
			true,
		);
		eventSourceRef.current = eventSource;

		eventSource.onopen = () => {
			setIsConnected(true);
		};

		eventSource.onmessage = (event) => {
			if (!isPausedRef.current) {
				const id = nextIdRef.current++;
				setLogs((prev) => {
					const next = [...prev, { id, text: event.data }];
					return next.length > MAX_LOG_LINES
						? next.slice(-MAX_LOG_LINES)
						: next;
				});
			}
		};

		eventSource.onerror = () => {
			setIsConnected(false);
			eventSource.close();
		};

		return () => {
			eventSource.close();
		};
	}, [props.identifier, props.service]);

	const handleClear = () => {
		setLogs([]);
	};

	return (
		<div className="bg-gray-950 border-2 border-emerald-500 shadow-[0_0_30px_rgba(16,185,129,0.2)]">
			{/* Header */}
			<div className="bg-linear-to-r from-emerald-950 to-gray-900 border-b-2 border-emerald-500 p-4 flex items-center justify-between">
				<div className="flex items-center gap-4">
					<h3 className="font-mono text-lg font-bold text-emerald-400 uppercase tracking-wider">
						[ Container Logs: {props.service} ]
					</h3>
					<div className="flex items-center gap-2">
						<div
							className={`h-2 w-2 rounded-full ${isConnected ? "bg-emerald-400 animate-pulse" : "bg-red-400"}`}
						/>
						<span className="text-xs font-mono text-gray-400">
							{isConnected ? "STREAMING" : "DISCONNECTED"}
						</span>
					</div>
				</div>

				<div className="flex items-center gap-2">
					<button
						type="button"
						onClick={() => setIsPaused(!isPaused)}
						className="p-2 bg-gray-800 hover:bg-gray-700 border border-gray-700 transition-colors"
						title={isPaused ? "Resume" : "Pause"}
					>
						{isPaused ? (
							<Play size={16} className="text-emerald-400" />
						) : (
							<Pause size={16} className="text-gray-400" />
						)}
					</button>
					<button
						type="button"
						onClick={handleClear}
						className="p-2 bg-gray-800 hover:bg-gray-700 border border-gray-700 transition-colors"
						title="Clear logs"
					>
						<Trash2 size={16} className="text-gray-400" />
					</button>
				</div>
			</div>

			{/* Log Content */}
			<div
				ref={containerRef}
				className="bg-black p-4 font-mono text-sm h-[600px] overflow-y-auto scrollbar-thin scrollbar-track-gray-900 scrollbar-thumb-emerald-800"
				style={{
					backgroundImage: `
            repeating-linear-gradient(
              0deg,
              rgba(16, 185, 129, 0.02) 0px,
              rgba(16, 185, 129, 0.02) 1px,
              transparent 1px,
              transparent 20px
            )
          `,
				}}
			>
				{logs.length === 0 ? (
					<div className="text-gray-600 text-center py-8">
						Waiting for logs...
					</div>
				) : (
					<div className="space-y-0.5">
						{logs.map((entry, idx) => (
							<div
								key={entry.id}
								className="hover:bg-emerald-950/20 leading-relaxed text-emerald-300/90"
							>
								<span className="text-gray-600 select-none mr-4">
									{String(idx + 1).padStart(4, "0")}
								</span>
								<span className="whitespace-pre-wrap break-all">
									{entry.text}
								</span>
							</div>
						))}
						<div ref={logEndRef} />
					</div>
				)}
			</div>

			{/* Footer */}
			<div className="bg-gray-900 border-t-2 border-emerald-500 p-2 flex items-center justify-between text-xs font-mono">
				<div className="text-gray-500">
					{logs.length} {logs.length === 1 ? "line" : "lines"}
				</div>
				{isPaused && (
					<div className="text-amber-400 font-bold animate-pulse">
						[ PAUSED ]
					</div>
				)}
			</div>
		</div>
	);
}
