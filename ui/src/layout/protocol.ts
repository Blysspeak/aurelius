import type { GraphModel } from '../lib/model'
import type { AureliusNode } from '../lib/types'

export interface NodeDetail {
  node: AureliusNode
  links: { rel: string; other: number; out: boolean }[]
}

export type ToWorker =
  | { type: 'load'; url: string }
  | { type: 'prioritize'; g: number }
  | { type: 'detail'; idx: number; reqId: number }

export type FromWorker =
  | { type: 'model'; model: GraphModel; ms: number }
  | {
      type: 'positions'
      g: number
      members: Uint32Array
      xy: Float32Array
      /** false while the project on screen is still settling. */
      final: boolean
      laidOut: number
      total: number
    }
  | { type: 'detail'; reqId: number; detail: NodeDetail }
  | { type: 'error'; message: string }
