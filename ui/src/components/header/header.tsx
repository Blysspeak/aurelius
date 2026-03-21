import { Search } from 'lucide-react'
import styles from './header.module.css'

interface HeaderProps {
  totalNodes: number
  totalEdges: number
  searchQuery: string
  onSearchChange: (query: string) => void
}

export function Header({ totalNodes, totalEdges, searchQuery, onSearchChange }: HeaderProps) {
  return (
    <header className={styles.header}>
      <div className={styles.brand}>
        <img src="/logo.png" alt="Aurelius" className={styles.logo} />
        <span className={styles.title}>Aurelius</span>
      </div>

      <div className={styles.stats}>
        <span className={styles.stat}>
          <span className={styles.statValue}>{totalNodes}</span> nodes
        </span>
        <span className={styles.divider}>·</span>
        <span className={styles.stat}>
          <span className={styles.statValue}>{totalEdges}</span> edges
        </span>
      </div>

      <div className={styles.searchBox}>
        <Search size={14} className={styles.searchIcon} />
        <input
          type="text"
          placeholder="Search nodes... (press /)"
          value={searchQuery}
          onChange={(e) => onSearchChange(e.target.value)}
          className={styles.searchInput}
          id="search-input"
        />
      </div>
    </header>
  )
}
