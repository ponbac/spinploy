import { Pause, Play, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
	createDeploymentLogStream,
	type LogStreamEventSource,
} from "@/lib/api-client";

interface DeploymentLogViewerProps {
	identifier: string;
	deploymentId: string;
	deploymentNumber: number;
	onClose: () => void;
}

export default function DeploymentLogViewer({
	identifier,
	deploymentId,
	deploymentNumber,
	onClose,
}: DeploymentLogViewerProps) {
	const [logs, setLogs] = useState<string[]>([]);
	const [isPaused, setIsPaused] = useState(false);
	const [isConnected, setIsConnected] = useState(false);
	const logEndRef = useRef<HTMLDivElement>(null);
	const eventSourceRef = useRef<LogStreamEventSource | null>(null);
	const containerRef = useRef<HTMLDivElement>(null);
	const isPausedRef = useRef(isPaused);
	isPausedRef.current = isPaused;

	// Auto-scroll to bottom when new logs arrive (unless manually scrolled up)
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
	}, [logs, isPaused]);

	// Connect to SSE stream
	useEffect(() => {
		const eventSource = createDeploymentLogStream(identifier, deploymentId);
		eventSourceRef.current = eventSource;

		eventSource.onopen = () => {
			setIsConnected(true);
		};

		eventSource.onmessage = (event) => {
			if (!isPausedRef.current) {
				setLogs((prev) => [...prev, event.data]);
			}
		};

		eventSource.onerror = () => {
			setIsConnected(false);
			eventSource.close();
		};

		return () => {
			eventSource.close();
		};
	}, [identifier, deploymentId]);

	const handleClear = () => {
		setLogs([]);
	};

	return (
		<div className="bg-gray-950 border-2 border-amber-500 shadow-[0_0_30px_rgba(245,158,11,0.2)]">
			{/* Header */}
			<div className="bg-gradient-to-r from-amber-950 to-gray-900 border-b-2 border-amber-500 p-4 flex items-center justify-between">
				<div className="flex items-center gap-4">
					<h3 className="font-mono text-lg font-bold text-amber-400 uppercase tracking-wider">
						[ Deployment #{deploymentNumber} Logs ]
					</h3>
					<div className="flex items-center gap-2">
						<div
							className={`h-2 w-2 rounded-full ${isConnected ? "bg-amber-400 animate-pulse" : "bg-red-400"}`}
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
							<Play size={16} className="text-amber-400" />
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
					<button
						type="button"
						onClick={onClose}
						className="p-2 bg-gray-800 hover:bg-red-900 border border-gray-700 transition-colors"
						title="Close"
					>
						<X size={16} className="text-gray-400" />
					</button>
				</div>
			</div>

			{/* Log Content */}
			<div
				ref={containerRef}
				className="bg-black p-4 font-mono text-sm h-[600px] overflow-y-auto scrollbar-thin scrollbar-track-gray-900 scrollbar-thumb-amber-800"
				style={{
					backgroundImage: `
            repeating-linear-gradient(
              0deg,
              rgba(245, 158, 11, 0.02) 0px,
              rgba(245, 158, 11, 0.02) 1px,
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
						{logs.map((log, idx) => (
							<div
								key={`log-${idx}-${log.slice(0, 20)}`}
								className="hover:bg-amber-950/20 leading-relaxed text-amber-300/90"
							>
								<span className="text-gray-600 select-none mr-4">
									{String(idx + 1).padStart(4, "0")}
								</span>
								<span className="whitespace-pre-wrap break-all">{log}</span>
							</div>
						))}
						<div ref={logEndRef} />
					</div>
				)}
			</div>

			{/* Footer */}
			<div className="bg-gray-900 border-t-2 border-amber-500 p-2 flex items-center justify-between text-xs font-mono">
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
