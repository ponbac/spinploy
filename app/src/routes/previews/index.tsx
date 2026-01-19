import { createFileRoute, Link } from "@tanstack/react-router";
import { Clock, Container, ExternalLink, GitBranch } from "lucide-react";
import StatusBadge from "@/components/StatusBadge";
import { usePreviewsList } from "@/lib/api-client";
import { formatDateTime } from "@/lib/utils";

export const Route = createFileRoute("/previews/")({
	component: PreviewsPage,
});

function PreviewsPage() {
	const { data, isLoading, error } = usePreviewsList();

	if (isLoading) {
		return (
			<div className="min-h-screen bg-[#0a0a0a] p-8">
				<div className="max-w-7xl mx-auto">
					<div className="h-8 w-48 bg-gray-800 animate-pulse mb-8" />
					<div className="grid grid-cols-1 gap-6">
						{[1, 2, 3].map((i) => (
							<div
								key={i}
								className="h-48 bg-gray-900 border-2 border-gray-800 animate-pulse"
							/>
						))}
					</div>
				</div>
			</div>
		);
	}

	if (error) {
		return (
			<div className="min-h-screen bg-[#0a0a0a] p-8 flex items-center justify-center">
				<div className="bg-red-950/20 border-2 border-red-500 text-red-400 p-8 font-mono">
					<div className="text-xl font-bold mb-2">[ ERROR ]</div>
					<div>Failed to load previews: {error.message}</div>
				</div>
			</div>
		);
	}

	const previews = data?.previews || [];

	return (
		<div className="min-h-screen bg-[#0a0a0a] text-gray-100">
			{/* Header Section */}
			<div className="border-b-4 border-emerald-500 bg-gradient-to-b from-emerald-950/20 to-transparent">
				<div className="max-w-7xl mx-auto p-8">
					<div className="flex items-baseline gap-4">
						<h1 className="text-5xl font-black tracking-tighter font-mono text-emerald-400">
							PREVIEW.DEPLOYMENTS
						</h1>
						<div className="text-gray-500 font-mono text-sm">
							[ {previews.length} ACTIVE ]
						</div>
					</div>
					<div className="mt-4 h-1 w-32 bg-emerald-500" />
				</div>
			</div>

			{/* Content */}
			<div className="max-w-7xl mx-auto p-8">
				{previews.length === 0 ? (
					<div className="bg-gray-900 border-2 border-gray-700 p-12 text-center">
						<div className="text-gray-500 font-mono text-lg">
							[ NO ACTIVE PREVIEWS ]
						</div>
						<div className="text-gray-600 text-sm mt-2">
							Create a preview with /preview command in Azure DevOps PR
						</div>
					</div>
				) : (
					<div className="grid grid-cols-1 gap-6">
						{previews.map((preview) => (
							<Link
								key={preview.identifier}
								to="/previews/$identifier"
								params={{ identifier: preview.identifier }}
								className="group"
							>
								<div className="bg-gray-950 border-2 border-gray-800 hover:border-emerald-500 transition-all duration-300 hover:shadow-[0_0_30px_rgba(16,185,129,0.15)] overflow-hidden">
									{/* Top Bar */}
									<div className="bg-gray-900 border-b-2 border-gray-800 p-4 flex items-center justify-between">
										<div className="flex items-center gap-4">
											<div className="font-mono text-xl font-bold text-emerald-400 tracking-tight">
												{preview.identifier}
											</div>
											<StatusBadge status={preview.status} />
										</div>
										<div className="text-xs text-gray-500 font-mono">
											{preview.lastDeployedAt
												? formatDateTime(preview.lastDeployedAt)
												: "Never deployed"}
										</div>
									</div>

									{/* Content Grid */}
									<div className="p-6 grid grid-cols-1 md:grid-cols-2 gap-6">
										{/* Left Column - Info */}
										<div className="space-y-4">
											{/* Branch */}
											<div className="flex items-start gap-3">
												<GitBranch className="text-gray-500 mt-1" size={16} />
												<div>
													<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
														Branch
													</div>
													<div className="font-mono text-sm text-gray-300">
														{preview.branch}
													</div>
												</div>
											</div>

											{/* PR Link */}
											{preview.prUrl && (
												<div className="flex items-start gap-3">
													<ExternalLink
														className="text-gray-500 mt-1"
														size={16}
													/>
													<div>
														<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
															Pull Request
														</div>
														<a
															href={preview.prUrl}
															target="_blank"
															rel="noopener noreferrer"
															className="font-mono text-sm text-cyan-400 hover:text-cyan-300 underline"
															onClick={(e) => e.stopPropagation()}
														>
															PR #{preview.prId}
														</a>
													</div>
												</div>
											)}

											{/* Timestamps */}
											<div className="flex items-start gap-3">
												<Clock className="text-gray-500 mt-1" size={16} />
												<div>
													<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-1">
														Created
													</div>
													<div className="font-mono text-sm text-gray-400">
														{preview.createdAt
															? formatDateTime(preview.createdAt)
															: "Unknown"}
													</div>
												</div>
											</div>
										</div>

										{/* Right Column - URLs & Containers */}
										<div className="space-y-4">
											{/* URLs */}
											{(preview.frontendUrl || preview.backendUrl) && (
												<div className="bg-gray-900 border border-gray-800 p-4">
													<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-3">
														Endpoints
													</div>
													<div className="space-y-2">
														{preview.frontendUrl && (
															<a
																href={preview.frontendUrl}
																target="_blank"
																rel="noopener noreferrer"
																className="block text-sm font-mono text-emerald-400 hover:text-emerald-300 break-all"
																onClick={(e) => e.stopPropagation()}
															>
																→ {preview.frontendUrl}
															</a>
														)}
														{preview.backendUrl && (
															<a
																href={preview.backendUrl}
																target="_blank"
																rel="noopener noreferrer"
																className="block text-sm font-mono text-cyan-400 hover:text-cyan-300 break-all"
																onClick={(e) => e.stopPropagation()}
															>
																→ {preview.backendUrl}
															</a>
														)}
													</div>
												</div>
											)}

											{/* Containers */}
											{preview.containers.length > 0 && (
												<div className="bg-gray-900 border border-gray-800 p-4">
													<div className="text-xs text-gray-500 uppercase tracking-wider font-mono mb-3 flex items-center gap-2">
														<Container size={14} />
														Containers [{preview.containers.length}]
													</div>
													<div className="space-y-1">
														{preview.containers.map((container) => (
															<div
																key={container.name}
																className="flex items-center justify-between text-xs font-mono"
															>
																<span className="text-gray-400">
																	{container.service}
																</span>
																<span
																	className={
																		container.state === "running"
																			? "text-emerald-400"
																			: "text-red-400"
																	}
																>
																	{container.state}
																</span>
															</div>
														))}
													</div>
												</div>
											)}
										</div>
									</div>

									{/* Bottom hover indicator */}
									<div className="h-1 bg-gradient-to-r from-transparent via-emerald-500 to-transparent transform scale-x-0 group-hover:scale-x-100 transition-transform duration-500" />
								</div>
							</Link>
						))}
					</div>
				)}
			</div>
		</div>
	);
}
