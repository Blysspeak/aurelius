/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        aurelius: {
          gold: '#c5a44e',
          goldLight: '#d4b96a',
          goldDark: '#9e833a',
          teal: '#5db8a9',
          tealLight: '#7ed4c5',
          bg: '#0c1018',
          bgCard: '#131a24',
          bgSidebar: '#101720',
          border: '#1e2a38',
          borderLight: '#2a3a4e',
          text: '#c9d1d9',
          textMuted: '#6b7b8e',
          textBright: '#e6edf3',
        },
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'sans-serif'],
        mono: ['JetBrains Mono', 'Fira Code', 'monospace'],
      },
    },
  },
  plugins: [],
}
