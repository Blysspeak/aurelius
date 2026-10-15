'use client'

import { useMemo } from 'react'
import type { GraphModel } from '@/lib/model'
import type { NodeDetail } from '@/layout/protocol'
import { typeColor } from '@/lib/palette'
import { stripProject } from '@/lib/types'

interface Props {
  model: GraphModel
  index: number
  detail: NodeDetail | null
  onClose: () => void
  onPick: (i: number) => void
}

const dateFmt = new Intl.DateTimeFormat('en-GB', {
  day: 'numeric',
  month: 'short',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
})

function when(iso: string | undefined): string {
  if (!iso) return 'never'
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : dateFmt.format(d)
}

export function DetailPanel({ model, index, detail, onClose, onPick }: Props) {
  const type = model.typeNames[model.type[index]]
  const node = detail && detail.node.id === model.ids[index] ? detail.node : null

  // Links grouped by relation and direction, belongs_to last: it only repeats the project.
  const groups = useMemo(() => {
    if (!node || !detail) return []
    const map = new Map<string, number[]>()
    for (const l of detail.links) {
      const key = `${l.out ? '' : '← '}${l.rel.replace(/_/g, ' ')}`
      let list = map.get(key)
      if (!list) map.set(key, (list = []))
      list.push(l.other)
    }
    return [...map.entries()].sort((a, b) => Number(a[0].includes('belongs')) - Number(b[0].includes('belongs')))
  }, [node, detail])

  const data = node?.data && Object.keys(node.data).length > 0 ? JSON.stringify(node.data, null, 2) : null

  return (
    <aside className="detail" aria-label="Node details">
      <header className="detail-head">
        <span className="detail-type">
          <i className="dot" style={{ background: typeColor(type) }} />
          {type.replace('_', ' ')}
          <span className="detail-project">in {model.groups[model.group[index]].name}</span>
        </span>
        <button type="button" className="close" onClick={onClose} aria-label="Close details">
          Esc
        </button>
      </header>
      <h1 className="detail-label">{stripProject(model.labels[index])}</h1>

      {!node ? (
        <p className="detail-wait">Loading the record</p>
      ) : (
        <>
          <dl className="facts">
            <dt>Created</dt>
            <dd>{when(node.created_at)}</dd>
            <dt>Updated</dt>
            <dd>{when(node.updated_at)}</dd>
            <dt>Last read</dt>
            <dd>{when(node.last_accessed_at)}</dd>
            <dt>Reads</dt>
            <dd>{node.access_count}</dd>
            <dt>Memory</dt>
            <dd>{node.memory_kind}</dd>
            <dt>Source</dt>
            <dd>{node.source}</dd>
          </dl>

          {node.note && <div className="note">{node.note}</div>}

          {data && (
            <details className="data">
              <summary>Data</summary>
              <pre>{data}</pre>
            </details>
          )}

          {groups.length > 0 && (
            <section className="links">
              {groups.map(([rel, others]) => (
                <div key={rel} className="link-group">
                  <h2>
                    {rel} <span className="row-count">{others.length}</span>
                  </h2>
                  <ul>
                    {others.slice(0, 40).map((o) => (
                      <li key={o}>
                        <button type="button" className="row" onClick={() => onPick(o)}>
                          <i className="dot" style={{ background: typeColor(model.typeNames[model.type[o]]) }} />
                          <span className="row-text">{stripProject(model.labels[o])}</span>
                        </button>
                      </li>
                    ))}
                    {others.length > 40 && <li className="empty">and {others.length - 40} more</li>}
                  </ul>
                </div>
              ))}
            </section>
          )}
          <p className="detail-id">{node.id}</p>
        </>
      )}
    </aside>
  )
}
