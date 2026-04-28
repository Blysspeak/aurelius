/// <reference types="vite/client" />

declare module '*.module.css' {
  const classes: { readonly [key: string]: string }
  export default classes
}

declare module 'react-force-graph-2d' {
  import { ComponentType } from 'react'
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const ForceGraph2D: ComponentType<any>
  export default ForceGraph2D
}

declare module 'd3-force-3d' {
  // Minimal surface — only what physics.ts uses. Matches d3-force-3d API.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  type Accessor<T> = T | ((node: any, i: number, nodes: any[]) => T)
  interface Collide {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (alpha: number): any
    radius(r: Accessor<number>): Collide
    strength(s: number): Collide
    iterations(n: number): Collide
  }
  export function forceCollide(radius?: Accessor<number>): Collide
}
