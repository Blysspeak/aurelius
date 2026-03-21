import { X, Clock, Hash, Database, Link2 } from 'lucide-react'
import type { AureliusNode, AureliusEdge } from '../../types'
import { parseNodeType, parseRelation } from '../../types'
import { getNodeColor } from '../../theme'
import styles from './nodeDetail.module.css'

interface NodeDetailProps {
  node: AureliusNode
  edges: AureliusEdge[]
  allNodes: AureliusNode[]
  onClose: () => void
  onSelectNode: (id: string) => void
}

export function NodeDetail({ node, edges, allNodes, onClose, onSelectNode }: NodeDetailProps) {
  const type = parseNodeType(node.node_type)
  const color = getNodeColor(type)
  const nodeMap = new Map(allNodes.map(n => [n.id, n]))

  const connectedEdges = edges.filter(
    e => e.from_id === node.id || e.to_id === node.id
  )

  return (
    <div className={styles.panel}>
      <button className={styles.closeBtn} onClick={onClose}>
        <X size={16} />
      </button>

      <div className={styles.header}>
        <h2 className={styles.title}>{node.label}</h2>
        <span
          className={styles.typeBadge}
          style={{
            background: `${color}18`,
            color: color,
            borderColor: `${color}40`,
          }}
        >
          {type}
        </span>
      </div>

      {node.note && (
        <div className={styles.field}>
          <div className={styles.fieldLabel}>Note</div>
          <div className={styles.fieldValue}>{node.note}</div>
        </div>
      )}

      <div className={styles.metaGrid}>
        <div className={styles.metaItem}>
          <Database size={12} />
          <span>{node.source}</span>
        </div>
        <div className={styles.metaItem}>
          <Hash size={12} />
          <span>{node.memory_kind || 'semantic'}</span>
        </div>
        <div className={styles.metaItem}>
          <Clock size={12} />
          <span>{new Date(node.created_at).toLocaleDateString()}</span>
        </div>
        <div className={styles.metaItem}>
          <Link2 size={12} />
          <span>{connectedEdges.length} connections</span>
        </div>
      </div>

      {node.access_count > 0 && (
        <div className={styles.field}>
          <div className={styles.fieldLabel}>Access Count</div>
          <div className={styles.fieldValue}>{node.access_count}</div>
        </div>
      )}

      <div className={styles.field}>
        <div className={styles.fieldLabel}>ID</div>
        <div className={styles.fieldValueMono}>{node.id}</div>
      </div>

      {connectedEdges.length > 0 && (
        <div className={styles.field}>
          <div className={styles.fieldLabel}>Connections</div>
          <ul className={styles.edgeList}>
            {connectedEdges.map(edge => {
              const isOutgoing = edge.from_id === node.id
              const otherId = isOutgoing ? edge.to_id : edge.from_id
              const otherNode = nodeMap.get(otherId)
              const otherLabel = otherNode?.label || otherId.slice(0, 8)
              const otherType = otherNode ? parseNodeType(otherNode.node_type) : 'unknown'
              const relation = parseRelation(edge.relation)

              return (
                <li key={edge.id} className={styles.edgeItem}>
                  <span className={styles.edgeDir}>{isOutgoing ? '→' : '←'}</span>
                  <span className={styles.edgeRelation}>{relation}</span>
                  <button
                    className={styles.edgeTarget}
                    onClick={() => onSelectNode(otherId)}
                    style={{ color: getNodeColor(otherType) }}
                  >
                    {otherLabel}
                  </button>
                </li>
              )
            })}
          </ul>
        </div>
      )}
    </div>
  )
}
