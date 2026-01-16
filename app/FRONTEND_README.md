# Spinploy Frontend

A distinctive **Terminal Command Center** interface for managing preview deployments. Built with React, TanStack Router & Query, and Tailwind CSS.

## Design Philosophy

**Brutalist-Technical Aesthetic**: Raw, functional, terminal-inspired interface with:
- **Monospace-first typography**: JetBrains Mono for that command-line feel
- **Dark theme**: Near-black backgrounds with neon accents (emerald green, cyan, amber)
- **Data-dense layouts**: Information-rich cards showing status at a glance
- **Real-time animations**: Pulse effects, status transitions, SSE log streaming
- **Terminal vibes**: Grid backgrounds, monospace fonts, harsh borders, high contrast

## Pages Implemented

### 1. Home (`/`)
Landing page with:
- Hero section with animated grid background
- Feature showcase
- Terminal-style command demo
- CTA to view deployments

### 2. Previews List (`/previews`)
Dashboard showing all active preview deployments:
- Card-based grid layout
- Status badges (Building, Running, Failed, Unknown)
- Container info with live states
- Links to frontend/backend URLs and Azure PR
- Auto-refreshes every 5 seconds
- Click any preview to view details

### 3. Preview Detail (`/previews/:identifier`)
Detailed view of a single preview:
- Comprehensive info panel
- Deployment history timeline with durations
- Container list with clickable items
- Live log viewer (SSE streaming)

## Components

### `StatusBadge`
Animated status indicator with:
- Color-coded states (emerald=running, amber=building, red=failed, gray=unknown)
- Pulsing dot animation for "Building" state
- Monospace typography

### `LogViewer`
Real-time container log streaming via SSE:
- Live connection indicator
- Pause/resume streaming
- Clear logs button
- Auto-scroll with manual override
- Line numbers
- Terminal green text on black background
- Scanline effect background

## API Integration

**Files:**
- `lib/api-types.ts` - TypeScript types matching Rust backend
- `lib/api-client.ts` - TanStack Query hooks + SSE helper

**Hooks:**
- `usePreviewsList()` - Fetches all previews (auto-refreshes every 5s)
- `usePreviewDetail(identifier)` - Fetches single preview details
- `createLogStream(identifier, service)` - Creates SSE connection for logs

## Environment Variables

Create `.env.local`:

```bash
# API base URL (default: /api for same-origin)
VITE_API_URL=/api

# API key for authentication
VITE_API_KEY=your-api-key-here
```

## Typography

- **Headings/Identifiers**: JetBrains Mono (800 weight for massive titles)
- **Body/UI**: IBM Plex Sans
- **Logs/Code**: JetBrains Mono with ligatures disabled

## Color Palette

```css
/* Base */
Background: #0a0a0a (near black)
Cards: #020617 (gray-950)
Borders: #1f2937 (gray-800)

/* Accents */
Success/Running: #10b981 (emerald-500)
Building: #f59e0b (amber-500)
Failed: #ef4444 (red-500)
Links: #06b6d4 (cyan-400)
```

## Key Features

1. **Auto-refresh**: Preview data updates every 5 seconds
2. **SSE Streaming**: Real-time log streaming with EventSource
3. **Responsive**: Mobile-friendly with grid breakpoints
4. **Animations**: Pulse effects, hover states, gradient bars
5. **Terminal Aesthetic**: Monospace fonts, harsh borders, scanlines, grid backgrounds

## Running

```bash
# Install dependencies
bun install

# Start dev server (port 3000)
bun run dev

# Build for production
bun run build
```

## Integration with Backend

The frontend expects the backend API at `/api` by default. For development:

1. Start backend: `cargo run` (port 3000)
2. Start frontend: `cd app && bun run dev` (port 3001)
3. Frontend will proxy `/api/*` requests to backend (configure in vite.config if needed)

Or set `VITE_API_URL=http://localhost:3000/api` to point directly to backend.
