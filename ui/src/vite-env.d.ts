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
