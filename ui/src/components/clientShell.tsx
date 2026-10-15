'use client'

import dynamic from 'next/dynamic'

// The graph needs a GPU, a Worker and window: nothing of it can prerender.
const GraphApp = dynamic(() => import('./graphApp').then((m) => m.GraphApp), {
  ssr: false,
  loading: () => <div className="boot">Opening the graph</div>,
})

export function ClientShell() {
  return <GraphApp />
}
