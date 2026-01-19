import { createFileRoute, Link } from "@tanstack/react-router";
import {
	Activity,
	ArrowLeft,
	Clock,
	Container,
	ExternalLink,
	GitBranch,
} from "lucide-react";
import { useState } from "react";
import LogViewer from "@/components/LogViewer";
import StatusBadge from "@/components/StatusBadge";
import { usePreviewDetail } from "@/lib/api-client";
import { formatDateTime, formatTime } from "@/lib/utils";

export const Route = createFileRoute("/previews/$identifier")({
	component: PreviewDetailPage,
});

function formatDuration(seconds: number | null): string {
	if (!seconds) return "-";
	const mins = Math.floor(seconds / 60);
	const secs = seconds % 60;
	return `${mins}m ${secs}s`;
}

function PreviewDetailPage() {
	const { identifier } = Route.useParams();
	const { data, isLoading, error } = usePreviewDetail(identifier);
	const [selectedService, setSelectedService] = useState<string | null>(null);

	if (isLoading) {
		return (
			<div className="min-h-screen bg-[#0a0a0a] p-8">
				<div className="max-w-7xl mx-auto">
					<div className="h-12 w-64 bg-gray-800 animate-pulse mb-8" />
					<div className="h-96 bg-gray-900 border-2 border-gray-800 animate-pulse" />
				</div>
			</div>
		);
	}

	if (error || !data) {
		return (
			<div className="min-h-screen bg-[#0a0a0a] p-8 flex items-center justify-center">
				<div className="bg-red-950/20 border-2 border-red-500 text-red-400 p-8 font-mono max-w-2xl">
					<div className="text-xl font-bold mb-2">[ ERROR ]</div>
					<div>
						Failed to load preview details: {error?.message || "Not found"}
					</div>
					<Link
						to="/previews"
						className="inline-block mt-4 text-cyan-400 hover:text-cyan-300 underline"
					>
						← Back to previews
					</Link>
				</div>
			</div>
		);
	}

	return (
		<div className="min-h-screen bg-[#0a0a0a] text-gray-100">
			{/* Header */}
			<div className="border-b-4 border-emerald-500 bg-gradient-to-b from-emerald-950/20 to-transparent">
				<div className="max-w-7xl mx-auto p-8">
					<Link
						to="/previews"
						className="inline-flex items-center gap-2 text-gray-400 hover:text-emerald-400 font-mono text-sm mb-4 transition-colors"
					>
						<ArrowLeft size={16} />
						BACK TO PREVIEWS
					</Link>
					<div className="flex items-baseline justify-between flex-wrap gap-4">
						<div className="flex items-baseline gap-4">
							<h1 className="text-5xl font-black tracking-tighter font-mono text-emerald-400">
								{data.identifier}
							</h1>
							<StatusBadge status={data.status} />
						</div>
					</div>
					<div className="mt-4 h-1 w-32 bg-emerald-500" />
				</div>
			</div>

			{/* Content */}
			<div className="max-w-7xl mx-auto p-8">
				<div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
					{/* Left Column - Main Info */}
					<div className="lg:col-span-2 space-y-6">
						{/* Info Panel */}
						<div className="bg-gray-950 border-2 border-gray-800">
							<div className="bg-gray-900 border-b-2 border-gray-800 p-4">
								<h2 className="font-mono text-lg font-bold text-gray-300 uppercase tracking-wider">
									[ Information ]
								</h2>
							</div>
							<div className="p-6 grid grid-cols-1 md:grid-cols-2 gap-6">
								{/* Branch */}
								<div className="flex items-start gap-3">
									<GitBranch className="text-emerald-500 mt-1" size={20} />
									<div>
										<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
											Branch
										</div>
										<div className="font-mono text-base text-gray-300">
											{data.branch}
										</div>
									</div>
								</div>

								{/* PR Link */}
								{data.prUrl && (
									<div className="flex items-start gap-3">
										<ExternalLink className="text-cyan-500 mt-1" size={20} />
										<div>
											<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
												Pull Request
											</div>
											<a
												href={data.prUrl}
												target="_blank"
												rel="noopener noreferrer"
												className="font-mono text-base text-cyan-400 hover:text-cyan-300 underline"
											>
												PR #{data.prId}
											</a>
										</div>
									</div>
								)}

								{/* Created */}
								<div className="flex items-start gap-3">
									<Clock className="text-gray-500 mt-1" size={20} />
									<div>
										<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
											Created
										</div>
										<div className="font-mono text-sm text-gray-400">
											{data.createdAt
												? formatDateTime(data.createdAt)
												: "Unknown"}
										</div>
									</div>
								</div>

								{/* Last Deployed */}
								<div className="flex items-start gap-3">
									<Activity className="text-gray-500 mt-1" size={20} />
									<div>
										<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
											Last Deployed
										</div>
										<div className="font-mono text-sm text-gray-400">
											{data.lastDeployedAt
												? formatDateTime(data.lastDeployedAt)
												: "Never"}
										</div>
									</div>
								</div>
							</div>
						</div>

						{/* URLs Panel */}
						{(data.frontendUrl || data.backendUrl) && (
							<div className="bg-gray-950 border-2 border-gray-800">
								<div className="bg-gray-900 border-b-2 border-gray-800 p-4">
									<h2 className="font-mono text-lg font-bold text-gray-300 uppercase tracking-wider">
										[ Endpoints ]
									</h2>
								</div>
								<div className="p-6 space-y-4">
									{data.frontendUrl && (
										<div>
											<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-2">
												Frontend
											</div>
											<a
												href={data.frontendUrl}
												target="_blank"
												rel="noopener noreferrer"
												className="block text-base font-mono text-emerald-400 hover:text-emerald-300 break-all bg-gray-900 p-3 border border-gray-800 hover:border-emerald-500 transition-colors"
											>
												→ {data.frontendUrl}
											</a>
										</div>
									)}
									{data.backendUrl && (
										<div>
											<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-2">
												Backend API
											</div>
											<a
												href={data.backendUrl}
												target="_blank"
												rel="noopener noreferrer"
												className="block text-base font-mono text-cyan-400 hover:text-cyan-300 break-all bg-gray-900 p-3 border border-gray-800 hover:border-cyan-500 transition-colors"
											>
												→ {data.backendUrl}
											</a>
										</div>
									)}
								</div>
							</div>
						)}

						{/* Deployment History */}
						<div className="bg-gray-950 border-2 border-gray-800">
							<div className="bg-gray-900 border-b-2 border-gray-800 p-4">
								<h2 className="font-mono text-lg font-bold text-gray-300 uppercase tracking-wider">
									[ Deployment History ]
								</h2>
							</div>
							<div className="p-6">
								{data.deployments.length === 0 ? (
									<div className="text-gray-500 font-mono text-center py-8">
										No deployments yet
									</div>
								) : (
									<div className="space-y-3">
										{(() => {
											// Sort by date and assign numbers, then show newest first
											const getTimestamp = (d: (typeof data.deployments)[0]) =>
												new Date(
													d.createdAt || d.startedAt || "1970-01-01",
												).getTime();
											const sorted = [...data.deployments].sort(
												(a, b) => getTimestamp(a) - getTimestamp(b),
											);
											const withNumbers = sorted.map((d, i) => ({
												...d,
												number: i + 1,
											}));
											// Reverse to show newest first
											return withNumbers.reverse().map((deployment) => {
												const statusColor =
													deployment.status === "done"
														? "text-emerald-400"
														: deployment.status === "error"
															? "text-red-400"
															: deployment.status === "running"
																? "text-yellow-400"
																: "text-gray-400";
												return (
													<div
														key={deployment.deploymentId}
														className="bg-gray-900 border border-gray-800 p-4"
													>
														<div className="flex items-center justify-between mb-2">
															<div className="flex items-center gap-3">
																<div className="font-mono text-sm text-gray-400">
																	#{deployment.number}
																</div>
																{deployment.status && (
																	<div
																		className={`font-mono text-xs font-bold uppercase ${statusColor}`}
																	>
																		{deployment.status}
																	</div>
																)}
															</div>
															<div className="font-mono text-xs text-gray-500">
																{deployment.finishedAt
																	? formatDateTime(deployment.finishedAt)
																	: deployment.startedAt
																		? "In progress..."
																		: "Queued"}
															</div>
														</div>
														<div className="grid grid-cols-3 gap-4 text-xs">
															<div>
																<div className="text-gray-500 mb-1">
																	Duration
																</div>
																<div className="font-mono text-gray-300">
																	{formatDuration(deployment.durationSeconds)}
																</div>
															</div>
															<div>
																<div className="text-gray-500 mb-1">
																	Started
																</div>
																<div className="font-mono text-gray-300">
																	{deployment.startedAt
																		? formatTime(deployment.startedAt)
																		: "-"}
																</div>
															</div>
															<div>
																<div className="text-gray-500 mb-1">
																	Finished
																</div>
																<div className="font-mono text-gray-300">
																	{deployment.finishedAt
																		? formatTime(deployment.finishedAt)
																		: "-"}
																</div>
															</div>
														</div>
													</div>
												);
											});
										})()}
									</div>
								)}
							</div>
						</div>
					</div>

					{/* Right Column - Containers */}
					<div className="space-y-6">
						<div className="bg-gray-950 border-2 border-gray-800">
							<div className="bg-gray-900 border-b-2 border-gray-800 p-4">
								<h2 className="font-mono text-lg font-bold text-gray-300 uppercase tracking-wider flex items-center gap-2">
									<Container size={18} />
									Containers
								</h2>
							</div>
							<div className="p-4">
								{data.containers.length === 0 ? (
									<div className="text-gray-500 font-mono text-sm text-center py-8">
										No containers found
									</div>
								) : (
									<div className="space-y-2">
										{data.containers.map((container) => (
											<button
												type="button"
												key={container.name}
												onClick={() => setSelectedService(container.service)}
												className={`w-full text-left p-3 border-2 transition-all ${
													selectedService === container.service
														? "bg-emerald-950/30 border-emerald-500"
														: "bg-gray-900 border-gray-800 hover:border-gray-700"
												}`}
											>
												<div className="flex items-center justify-between mb-2">
													<div className="font-mono text-sm font-bold text-gray-300">
														{container.service}
													</div>
													<div
														className={`text-xs font-mono font-bold ${
															container.state === "running"
																? "text-emerald-400"
																: "text-red-400"
														}`}
													>
														{container.state}
													</div>
												</div>
												<div className="font-mono text-xs text-gray-500 break-all">
													{container.name}
												</div>
											</button>
										))}
									</div>
								)}
							</div>
						</div>
					</div>
				</div>

				{/* Log Viewer */}
				{selectedService && (
					<div className="mt-6">
						<LogViewer
							key={`${identifier}-${selectedService}`}
							identifier={identifier}
							service={selectedService}
						/>
					</div>
				)}
			</div>
		</div>
	);
}
