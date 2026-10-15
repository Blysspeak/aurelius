import type { Metadata } from 'next'
import { IBM_Plex_Mono, Fira_Sans_Condensed, Literata } from 'next/font/google'
import './globals.css'

// Fonts are fetched at build time and shipped inside the static export, so
// `au view` works offline. Cyrillic is required: most labels are Russian.
const serif = Literata({ subsets: ['latin', 'cyrillic'], variable: '--font-serif', display: 'swap' })
const sans = Fira_Sans_Condensed({
  subsets: ['latin', 'cyrillic'],
  weight: ['400', '500', '600'],
  variable: '--font-sans',
  display: 'swap',
})
const mono = IBM_Plex_Mono({ subsets: ['latin', 'cyrillic'], weight: ['400'], variable: '--font-mono', display: 'swap' })

export const metadata: Metadata = {
  title: 'Aurelius memory graph',
  description: 'Read the Aurelius knowledge graph, one project at a time.',
  icons: { icon: '/logo.png' },
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${serif.variable} ${sans.variable} ${mono.variable}`}>
      <body>{children}</body>
    </html>
  )
}
