// API types matching backend src/api/types.rs

export type PreviewStatus = 'Building' | 'Running' | 'Failed' | 'Unknown'

export interface ContainerSummary {
  name: string
  service: string
  state: string
}

export interface PreviewSummary {
  identifier: string
  composeId: string
  prId: string | null
  branch: string
  status: PreviewStatus
  createdAt: string | null
  lastDeployedAt: string | null
  frontendUrl: string | null
  backendUrl: string | null
  prUrl: string | null
  containers: ContainerSummary[]
}

export interface DeploymentInfo {
  deploymentId: string
  createdAt: string | null
  startedAt: string | null
  finishedAt: string | null
  durationSeconds: number | null
}

export interface PreviewListResponse {
  previews: PreviewSummary[]
}

export interface PreviewDetailResponse extends PreviewSummary {
  deployments: DeploymentInfo[]
}
