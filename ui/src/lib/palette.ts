// One palette for node types, generated in OKLCH so every hue sits at a
// similar lightness on the dark canvas: no type shouts over another, and the
// graph reads as texture until you look for something. Knowledge types
// (decision, problem, solution, concept) get the clearest hues; structural
// types (file, config, dependency...) stay low-chroma greys.
const TYPE_COLORS: Record<string, string> = {
  project: '#eec469',
  decision: '#73abda',
  problem: '#dd7767',
  solution: '#76be86',
  concept: '#b695cf',
  session: '#69babf',
  task: '#e5ab66',
  work_log: '#9e805e',
  user_fact: '#dd90ae',
  doc: '#afb784',
  document: '#afb784',
  reference: '#9da37f',
  digest: '#74a2a4',
  config: '#8594a4',
  file: '#869889',
  dependency: '#8b8679',
  server: '#7798a3',
  skill: '#cc9684',
  person: '#d5a2a1',
  module: '#8e91a5',
}

export const FALLBACK_COLOR = '#7b8186'

export function typeColor(type: string): string {
  return TYPE_COLORS[type] ?? FALLBACK_COLOR
}

/** Base radius in layout units; hubs and work items read larger than notes. */
export function typeRadius(type: string): number {
  switch (type) {
    case 'project':
      return 9
    case 'session':
    case 'task':
      return 3.2
    case 'work_log':
    case 'file':
    case 'dependency':
      return 2.2
    default:
      return 2.8
  }
}

export function hexToRgb(hex: string): [number, number, number] {
  const v = parseInt(hex.slice(1), 16)
  return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255]
}
