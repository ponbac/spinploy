import { createFileRoute, Link } from '@tanstack/react-router'
import { Box, Terminal, Zap, GitBranch } from 'lucide-react'

export const Route = createFileRoute('/')({
  component: HomePage,
})

function HomePage() {
  return (
    <div className="min-h-screen bg-[#0a0a0a] text-gray-100">
      {/* Hero Section */}
      <div className="relative overflow-hidden">
        {/* Animated Background Grid */}
        <div
          className="absolute inset-0 opacity-20"
          style={{
            backgroundImage: `
              linear-gradient(rgba(16, 185, 129, 0.1) 1px, transparent 1px),
              linear-gradient(90deg, rgba(16, 185, 129, 0.1) 1px, transparent 1px)
            `,
            backgroundSize: '50px 50px',
          }}
        />

        {/* Content */}
        <div className="relative max-w-6xl mx-auto px-8 py-32">
          <div className="text-center space-y-8">
            {/* Title */}
            <div className="space-y-4">
              <div className="inline-block">
                <div className="flex items-center gap-3 text-emerald-400 mb-4">
                  <Terminal size={32} className="animate-pulse" />
                  <div className="h-1 w-16 bg-emerald-500" />
                </div>
              </div>
              <h1 className="text-7xl md:text-8xl font-black tracking-tighter font-mono text-transparent bg-clip-text bg-gradient-to-r from-emerald-400 to-cyan-400">
                SPINPLOY
              </h1>
              <div className="h-2 w-48 bg-emerald-500 mx-auto" />
            </div>

            {/* Tagline */}
            <p className="text-2xl text-gray-400 font-mono max-w-2xl mx-auto">
              Lightning-fast preview deployments for your{' '}
              <span className="text-emerald-400 font-bold">
                Azure DevOps
              </span>{' '}
              pull requests
            </p>

            {/* CTA */}
            <div className="pt-8">
              <Link
                to="/previews"
                className="group inline-flex items-center gap-3 bg-emerald-500 hover:bg-emerald-400 text-black px-8 py-4 font-mono font-bold text-lg uppercase tracking-wider transition-all hover:shadow-[0_0_40px_rgba(16,185,129,0.6)] border-2 border-emerald-400"
              >
                <Box size={24} />
                View Deployments
                <div className="h-6 w-1 bg-black group-hover:w-3 transition-all" />
              </Link>
            </div>
          </div>

          {/* Features Grid */}
          <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mt-24">
            <div className="bg-gray-950 border-2 border-gray-800 hover:border-emerald-500 transition-all p-8 group">
              <div className="text-emerald-400 mb-4 group-hover:scale-110 transition-transform">
                <Zap size={40} />
              </div>
              <h3 className="font-mono text-xl font-bold text-gray-200 mb-3">
                INSTANT PREVIEW
              </h3>
              <p className="text-gray-500 text-sm leading-relaxed">
                Create isolated preview environments with a single{' '}
                <code className="text-emerald-400">/preview</code> command in
                your PR comments
              </p>
            </div>

            <div className="bg-gray-950 border-2 border-gray-800 hover:border-cyan-500 transition-all p-8 group">
              <div className="text-cyan-400 mb-4 group-hover:scale-110 transition-transform">
                <GitBranch size={40} />
              </div>
              <h3 className="font-mono text-xl font-bold text-gray-200 mb-3">
                BRANCH ISOLATION
              </h3>
              <p className="text-gray-500 text-sm leading-relaxed">
                Each preview runs in its own container with dedicated domains
                for frontend and backend services
              </p>
            </div>

            <div className="bg-gray-950 border-2 border-gray-800 hover:border-amber-500 transition-all p-8 group">
              <div className="text-amber-400 mb-4 group-hover:scale-110 transition-transform">
                <Terminal size={40} />
              </div>
              <h3 className="font-mono text-xl font-bold text-gray-200 mb-3">
                LIVE LOGS
              </h3>
              <p className="text-gray-500 text-sm leading-relaxed">
                Stream container logs in real-time with SSE for instant
                debugging and monitoring
              </p>
            </div>
          </div>

          {/* Terminal Demo */}
          <div className="mt-24 bg-black border-2 border-emerald-500 shadow-[0_0_40px_rgba(16,185,129,0.2)] overflow-hidden">
            <div className="bg-gray-900 border-b-2 border-emerald-500 px-4 py-2 flex items-center gap-2">
              <div className="flex gap-2">
                <div className="h-3 w-3 rounded-full bg-red-500" />
                <div className="h-3 w-3 rounded-full bg-amber-500" />
                <div className="h-3 w-3 rounded-full bg-emerald-500" />
              </div>
              <div className="text-gray-500 text-sm font-mono ml-4">
                azure-devops-pr-comment.sh
              </div>
            </div>
            <div className="p-6 font-mono text-sm space-y-2">
              <div className="flex items-start gap-3">
                <span className="text-gray-600">$</span>
                <span className="text-emerald-400">
                  /preview{' '}
                  <span className="text-gray-500">
                    # Create preview deployment
                  </span>
                </span>
              </div>
              <div className="text-gray-500 pl-6">
                → Creating preview environment for PR #123...
              </div>
              <div className="text-gray-500 pl-6">
                → Deploying containers: frontend, backend
              </div>
              <div className="text-emerald-400 pl-6">
                ✓ Preview deployed at: https://pr-123.example.com
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
