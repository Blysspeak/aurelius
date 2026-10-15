import type { NextConfig } from 'next'

// Static export: `next build` writes plain files to ui/out/, which
// crates/au embeds with rust-embed. No Node runs next to `au view`; the page
// talks to /api/* on the same origin, served by `au view` itself.
const nextConfig: NextConfig = {
  output: 'export',
  images: { unoptimized: true },
  reactStrictMode: true,
}

export default nextConfig
