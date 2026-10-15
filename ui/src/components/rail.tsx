'use client'

import { useMemo, useState } from 'react'
import type { GraphModel } from '@/lib/model'
import { UNASSIGNED } from '@/lib/model'
import { typeColor } from '@/lib/palette'
import { stripProject } from '@/lib/types'
import type { Backend } from '@/render/graphRenderer'
import type { Scope } from './graphApp'

interface Props {
  model: GraphModel | null
  scope: Scope
  onScope: (s: Scope) => void
  hiddenTypes: Set<number>
  onToggleType: (t: number) => void
  onShowAllTypes: () => void
  query: string
  onQuery: (q: string) => void
  matches: number[] | null
  searching: boolean
  onPick: (i: number) => void
  backend: Backend | null
}

export function Rail(p: Props) {
  const { model, scope } = p
  const [projectFilter, setProjectFilter] = useState('')

  const projects = useMemo(() => {
    if (!model) return []
    return model.groups
      .map((g, i) => ({ i, name: g.name, size: g.size }))
      .filter((g) => g.size > 0)
      .sort((a, b) => (a.name === UNASSIGNED ? 1 : b.name === UNASSIGNED ? -1 : b.size - a.size))
  }, [model])

  const shownProjects = useMemo(() => {
    const f = projectFilter.trim().toLowerCase()
    return f ? projects.filter((g) => g.name.toLowerCase().includes(f)) : projects
  }, [projects, projectFilter])

  const typeCounts = useMemo(() => {
    if (!model) return []
    const counts = new Uint32Array(model.typeNames.length)
    for (let i = 0; i < model.n; i++) if (scope === 'all' || model.group[i] === scope) counts[model.type[i]]++
    return model.typeNames
      .map((name, t) => ({ t, name, count: counts[t] }))
      .filter((x) => x.count > 0)
      .sort((a, b) => b.count - a.count)
  }, [model, scope])

  return (
    <aside className="rail">
      <header className="rail-head">
        <span className="wordmark">Aurelius</span>
        {p.backend && (
          <span className="backend" title="Renderer in use">
            {p.backend}
          </span>
        )}
      </header>

      <div className="search">
        <input
          id="search"
          type="search"
          placeholder="Search memory"
          autoComplete="off"
          spellCheck={false}
          value={p.query}
          onChange={(e) => p.onQuery(e.target.value)}
        />
        <kbd aria-hidden="true">/</kbd>
      </div>

      {p.query.trim() ? (
        <section className="rail-section rail-grow" aria-label="Search results">
          <h2 className="rail-title">
            {p.searching ? 'Searching' : p.matches ? `${p.matches.length} found` : ''}
          </h2>
          <ul className="list">
            {model &&
              p.matches?.map((i) => (
                <li key={i}>
                  <button type="button" className="row row-result" onClick={() => p.onPick(i)}>
                    <i className="dot" style={{ background: typeColor(model.typeNames[model.type[i]]) }} />
                    <span className="row-text">{stripProject(model.labels[i])}</span>
                    <span className="row-meta">{model.groups[model.group[i]].name}</span>
                  </button>
                </li>
              ))}
            {p.matches && p.matches.length === 0 && !p.searching && (
              <li className="empty">Nothing matches. Try fewer words or another spelling.</li>
            )}
          </ul>
        </section>
      ) : (
        <section className="rail-section rail-grow" aria-label="Projects">
          <div className="rail-title-row">
            <h2 className="rail-title">Projects</h2>
            {projects.length > 12 && (
              <input
                className="mini-filter"
                placeholder="filter"
                value={projectFilter}
                onChange={(e) => setProjectFilter(e.target.value)}
                aria-label="Filter projects"
              />
            )}
          </div>
          <ul className="list">
            <li>
              <button
                type="button"
                className={`row ${scope === 'all' ? 'is-active' : ''}`}
                onClick={() => p.onScope('all')}
              >
                <span className="row-text">All projects</span>
                <span className="row-count">{model?.n.toLocaleString('en') ?? ''}</span>
              </button>
            </li>
            {shownProjects.map((g) => (
              <li key={g.i}>
                <button
                  type="button"
                  className={`row ${scope === g.i ? 'is-active' : ''}`}
                  onClick={() => p.onScope(g.i)}
                >
                  <span className="row-text">{g.name}</span>
                  <span className="row-count">{g.size.toLocaleString('en')}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section className="rail-section" aria-label="Node types">
        <div className="rail-title-row">
          <h2 className="rail-title">Types</h2>
          {p.hiddenTypes.size > 0 && (
            <button type="button" className="link" onClick={p.onShowAllTypes}>
              Show all
            </button>
          )}
        </div>
        <ul className="types">
          {typeCounts.map(({ t, name, count }) => {
            const off = p.hiddenTypes.has(t)
            return (
              <li key={t}>
                <button
                  type="button"
                  className={`type ${off ? 'is-off' : ''}`}
                  aria-pressed={!off}
                  onClick={() => p.onToggleType(t)}
                  title={off ? `Show ${name}` : `Hide ${name}`}
                >
                  <i className="dot" style={{ background: typeColor(name) }} />
                  <span className="row-text">{name.replace('_', ' ')}</span>
                  <span className="row-count">{count.toLocaleString('en')}</span>
                </button>
              </li>
            )
          })}
        </ul>
      </section>
    </aside>
  )
}
