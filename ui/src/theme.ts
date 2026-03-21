export const nodeColors: Record<string, string> = {
  project:    '#c5a44e',
  crate:      '#5db8a9',
  file:       '#7ee787',
  dependency: '#8b949e',
  decision:   '#ff7b72',
  concept:    '#d2a8ff',
  problem:    '#f85149',
  solution:   '#3fb950',
  person:     '#ffa198',
  server:     '#56d4dd',
  module:     '#79c0ff',
  config:     '#e3b341',
  session:    '#bc8cff',
  language:   '#f0883e',
}

export const defaultNodeColor = '#6b7b8e'

export const nodeSizes: Record<string, number> = {
  project:    8,
  crate:      6,
  decision:   5,
  concept:    4.5,
  problem:    5,
  solution:   5,
  file:       3,
  dependency: 2.5,
}

export const defaultNodeSize = 4

export function getNodeColor(type: string): string {
  return nodeColors[type] || defaultNodeColor
}

export function getNodeSize(type: string): number {
  return nodeSizes[type] || defaultNodeSize
}
