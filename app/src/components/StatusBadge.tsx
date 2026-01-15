import type { PreviewStatus } from '@/lib/api-types'
import { cn } from '@/lib/utils'

interface StatusBadgeProps {
  status: PreviewStatus
  className?: string
}

export default function StatusBadge({ status, className }: StatusBadgeProps) {
  const statusConfig = {
    Building: {
      label: 'BUILDING',
      className: 'bg-amber-500/20 text-amber-400 border-amber-500/50',
      dotClassName: 'bg-amber-400 animate-pulse',
    },
    Running: {
      label: 'RUNNING',
      className: 'bg-emerald-500/20 text-emerald-400 border-emerald-500/50',
      dotClassName: 'bg-emerald-400',
    },
    Failed: {
      label: 'FAILED',
      className: 'bg-red-500/20 text-red-400 border-red-500/50',
      dotClassName: 'bg-red-400',
    },
    Unknown: {
      label: 'UNKNOWN',
      className: 'bg-gray-500/20 text-gray-400 border-gray-500/50',
      dotClassName: 'bg-gray-400',
    },
  }

  const config = statusConfig[status]

  return (
    <div
      className={cn(
        'inline-flex items-center gap-2 px-3 py-1 border-2 font-mono text-xs font-bold tracking-wider uppercase',
        config.className,
        className,
      )}
    >
      <div
        className={cn('h-2 w-2 rounded-full', config.dotClassName)}
        aria-hidden="true"
      />
      {config.label}
    </div>
  )
}
