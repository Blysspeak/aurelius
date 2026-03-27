import { nodeColors, defaultNodeColor } from '../../theme'
import styles from './sidebar.module.css'

interface SidebarProps {
  typeCounts: Record<string, number>
  activeFilters: Set<string>
  onToggleFilter: (type: string) => void
  onClearFilters: () => void
  projectCounts: Record<string, number>
  activeProject: string | null
  onSelectProject: (project: string | null) => void
}

export function Sidebar({ typeCounts, activeFilters, onToggleFilter, onClearFilters, projectCounts, activeProject, onSelectProject }: SidebarProps) {
  const types = Object.entries(typeCounts).sort((a, b) => b[1] - a[1])
  const projects = Object.entries(projectCounts).sort((a, b) => b[1] - a[1])

  return (
    <aside className={styles.sidebar}>
      {projects.length > 0 && (
        <div className={styles.section}>
          <h3 className={styles.sectionTitle}>Projects</h3>
          <button
            className={`${styles.filterItem} ${!activeProject ? styles.active : ''}`}
            onClick={() => onSelectProject(null)}
          >
            <span className={styles.filterDot} style={{ background: '#c5a44e' }} />
            <span className={styles.filterLabel}>All</span>
            <span className={styles.filterCount}>{Object.values(projectCounts).reduce((a, b) => a + b, 0)}</span>
          </button>
          {projects.map(([proj, count]) => (
            <button
              key={proj}
              className={`${styles.filterItem} ${activeProject === proj ? styles.active : ''}`}
              onClick={() => onSelectProject(proj)}
            >
              <span className={styles.filterDot} style={{ background: '#c5a44e' }} />
              <span className={styles.filterLabel}>{proj}</span>
              <span className={styles.filterCount}>{count}</span>
            </button>
          ))}
        </div>
      )}

      <div className={styles.section}>
        <h3 className={styles.sectionTitle}>Node Types</h3>
        <button
          className={`${styles.filterItem} ${activeFilters.size === 0 ? styles.active : ''}`}
          onClick={onClearFilters}
        >
          <span className={styles.filterDot} style={{ background: '#c9d1d9' }} />
          <span className={styles.filterLabel}>All</span>
          <span className={styles.filterCount}>{Object.values(typeCounts).reduce((a, b) => a + b, 0)}</span>
        </button>

        {types.map(([type, count]) => (
          <button
            key={type}
            className={`${styles.filterItem} ${activeFilters.has(type) ? styles.active : ''}`}
            onClick={() => onToggleFilter(type)}
          >
            <span
              className={styles.filterDot}
              style={{ background: nodeColors[type] || defaultNodeColor }}
            />
            <span className={styles.filterLabel}>{type}</span>
            <span className={styles.filterCount}>{count}</span>
          </button>
        ))}
      </div>

      <div className={styles.section}>
        <h3 className={styles.sectionTitle}>Keyboard</h3>
        <div className={styles.shortcuts}>
          <div className={styles.shortcut}>
            <kbd className={styles.kbd}>/</kbd>
            <span>Search</span>
          </div>
          <div className={styles.shortcut}>
            <kbd className={styles.kbd}>Esc</kbd>
            <span>Close panel</span>
          </div>
          <div className={styles.shortcut}>
            <kbd className={styles.kbd}>Scroll</kbd>
            <span>Zoom</span>
          </div>
        </div>
      </div>
    </aside>
  )
}
