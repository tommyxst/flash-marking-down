<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { UiError } from '../../api/errors'
import ErrorNotice from '../ErrorNotice.vue'
import type { GraphGap, GraphNode, KnowledgeGraph } from '../../m2'

const props = defineProps<{
  graph: KnowledgeGraph | null
  loading: boolean
  error: UiError | null
  includeMetaPages: boolean
}>()

const emit = defineEmits<{
  'update:includeMetaPages': [val: boolean]
  openWiki: [path: string]
  dismissGap: [gapId: string]
  askGap: [gap: GraphGap]
}>()

const typeFilter = ref<string>('all')
const selectedNodeId = ref<string>('')
const selectedEdgeId = ref<string>('')

const filters = [
  { id: 'all', label: '全部' },
  { id: 'decision', label: '决策' },
  { id: 'concept', label: '概念' },
  { id: 'synthesis', label: '综论' },
  { id: 'entity', label: '组件' },
]

const filteredNodes = computed(() => {
  const nodes = props.graph?.nodes || []
  if (typeFilter.value === 'all') return nodes
  return nodes.filter((n) => n.docType === typeFilter.value)
})

const positionedNodes = computed(() => {
  const nodes = filteredNodes.value
  const cx = 370
  const cy = 195
  const hubs = nodes.filter((n) => n.isHub)
  const regular = nodes.filter((n) => !n.isHub && !n.isOrphan)
  const orphans = nodes.filter((n) => n.isOrphan && !n.isHub)

  const coords = new Map<string, { x: number; y: number }>()

  const placeRing = (list: GraphNode[], rx: number, ry: number, offsetAngle = -Math.PI / 2) => {
    if (!list.length) return
    if (list.length === 1 && rx === 0) {
      coords.set(list[0].id, { x: cx, y: cy })
      return
    }
    list.forEach((n, i) => {
      const angle = offsetAngle + (2 * Math.PI * i) / list.length
      coords.set(n.id, {
        x: Math.round(cx + rx * Math.cos(angle)),
        y: Math.round(cy + ry * Math.sin(angle)),
      })
    })
  }

  if (hubs.length === 1) {
    placeRing(hubs, 0, 0)
  } else {
    placeRing(hubs, 86, 56, -Math.PI / 4)
  }
  placeRing(regular, hubs.length ? 215 : 170, hubs.length ? 128 : 112, -Math.PI / 2)
  placeRing(orphans, 305, 162, 0.35)

  return nodes.map((n) => ({
    ...n,
    x: coords.get(n.id)?.x ?? cx,
    y: coords.get(n.id)?.y ?? cy,
  }))
})

const nodeById = computed(() => {
  const map = new Map<string, GraphNode & { x: number; y: number }>()
  for (const n of positionedNodes.value) map.set(n.id, n)
  return map
})

const filteredEdges = computed(() => {
  const edges = props.graph?.edges || []
  const ids = nodeById.value
  return edges.filter((e) => ids.has(e.from) && ids.has(e.to))
})

watch(
  () => [positionedNodes.value, filteredEdges.value] as const,
  ([nodes, edges]) => {
    if (!nodes.some((n) => n.id === selectedNodeId.value)) {
      selectedNodeId.value = nodes[0]?.id || ''
    }
    if (!edges.some((e) => e.id === selectedEdgeId.value)) {
      selectedEdgeId.value = edges[0]?.id || ''
    }
  },
  { immediate: true },
)

const activeNode = computed(() => nodeById.value.get(selectedNodeId.value) || positionedNodes.value[0] || null)
const activeEdge = computed(
  () => filteredEdges.value.find((e) => e.id === selectedEdgeId.value) || filteredEdges.value[0] || null,
)
const activeNodeEdges = computed(() => {
  if (!activeNode.value) return filteredEdges.value
  const id = activeNode.value.id
  return filteredEdges.value.filter((e) => e.from === id || e.to === id)
})

function nodeTitle(id: string): string {
  return nodeById.value.get(id)?.title || id.split('/').pop()?.replace(/\.md$/i, '') || id
}

function edgeColor(kind: 'link' | 'citation'): string {
  return kind === 'citation' ? 'var(--info)' : 'var(--accent)'
}
</script>

<template>
  <div class="stage-body flush">
    <div class="graph-layout">
      <div class="graph-canvas-wrap">
        <div class="graph-toolbar">
          <div style="display:flex;align-items:center;gap:6px;flex-wrap:wrap;">
            <span class="kicker">筛选</span>
            <button
              v-for="f in filters"
              :key="f.id"
              type="button"
              class="btn sm"
              :class="typeFilter === f.id ? 'primary' : 'quiet'"
              @click="typeFilter = f.id"
            >{{ f.label }}</button>
          </div>
          <div style="display:flex;align-items:center;gap:8px;font-size:11.5px;color:var(--muted);">
            <span>{{ includeMetaPages ? '已显示目录与日志页' : '已隐藏目录与日志页' }}</span>
            <button
              type="button"
              class="btn quiet xs"
              @click="emit('update:includeMetaPages', !includeMetaPages)"
            >{{ includeMetaPages ? '隐藏' : '显示' }}</button>
          </div>
        </div>

        <div class="graph-svg-stage">
          <div v-if="loading && !graph" class="empty-stage">
            <p>正在构建知识图谱…</p>
          </div>
          <div v-else-if="error && !graph" class="empty-stage">
            <ErrorNotice :error="error" />
          </div>
          <div v-else-if="!positionedNodes.length" class="empty-stage">
            <p>当前筛选条件下暂无可展示的知识节点。</p>
          </div>
          <template v-else>
            <svg class="graph-svg" viewBox="0 0 740 410" role="img" aria-label="交互式知识图谱">
              <g
                v-for="e in filteredEdges"
                :key="e.id"
                style="cursor:pointer;"
                @click="selectedEdgeId = e.id"
              >
                <line
                  :x1="nodeById.get(e.from)?.x"
                  :y1="nodeById.get(e.from)?.y"
                  :x2="nodeById.get(e.to)?.x"
                  :y2="nodeById.get(e.to)?.y"
                  stroke="transparent"
                  stroke-width="14"
                />
                <line
                  :x1="nodeById.get(e.from)?.x"
                  :y1="nodeById.get(e.from)?.y"
                  :x2="nodeById.get(e.to)?.x"
                  :y2="nodeById.get(e.to)?.y"
                  :stroke="edgeColor(e.kind)"
                  :stroke-width="activeEdge?.id === e.id ? 2.6 : 1.5"
                  :stroke-opacity="activeEdge?.id === e.id ? 1 : 0.55"
                  :stroke-dasharray="e.kind === 'citation' ? '3 3' : undefined"
                />
              </g>

              <g
                v-for="n in positionedNodes"
                :key="n.id"
                style="cursor:pointer;"
                @click="selectedNodeId = n.id"
                @dblclick="emit('openWiki', n.id)"
              >
                <circle
                  :cx="n.x"
                  :cy="n.y"
                  :r="(n.isHub ? 11 : n.isOrphan ? 7.5 : 8.5) + (activeNode?.id === n.id ? 4 : 0)"
                  :fill="activeNode?.id === n.id ? 'var(--accent-soft)' : 'transparent'"
                />
                <circle
                  :cx="n.x"
                  :cy="n.y"
                  :r="n.isHub ? 11 : n.isOrphan ? 7.5 : 8.5"
                  :fill="activeNode?.id === n.id ? 'var(--accent)' : n.isOrphan ? 'var(--warning)' : 'var(--surface-raised)'"
                  :stroke="activeNode?.id === n.id ? 'var(--accent)' : n.isHub ? 'var(--accent)' : 'var(--line-strong)'"
                  stroke-width="1.8"
                />
                <text
                  :x="n.x"
                  :y="n.y + (n.isHub ? 11 : n.isOrphan ? 7.5 : 8.5) + 15"
                  text-anchor="middle"
                  fill="var(--text)"
                  font-size="11.5"
                  :font-weight="activeNode?.id === n.id ? 650 : 500"
                >{{ n.title.length > 14 ? `${n.title.slice(0, 14)}…` : n.title }}</text>
              </g>
            </svg>

            <div v-if="activeEdge" class="edge-popover">
              <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;">
                <div style="display:flex;align-items:center;gap:6px;min-width:0;">
                  <span class="pill accent">{{ activeEdge.label }}</span>
                  <strong style="font-size:12.5px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">
                    {{ nodeTitle(activeEdge.from) }} → {{ nodeTitle(activeEdge.to) }}
                  </strong>
                </div>
                <button
                  type="button"
                  class="btn sm"
                  @click="emit('openWiki', activeEdge.from)"
                >打开笔记</button>
              </div>
              <div style="font-size:12.5px;color:var(--text-secondary);">
                关联依据：{{ activeEdge.snippet || '通过双链或共同引文建立关联' }}
              </div>
            </div>
          </template>
        </div>
      </div>

      <aside class="graph-side" aria-label="关联列表与待补充连接">
        <div>
          <div class="kicker" style="margin-bottom:6px;">关联列表（支持键盘上下切换）</div>
          <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:8px;">
            <span style="font-size:13.5px;font-weight:650;">{{ activeNode?.title || '未选中' }}</span>
            <button
              v-if="activeNode"
              type="button"
              class="btn quiet xs"
              @click="emit('openWiki', activeNode.id)"
            >打开</button>
          </div>

          <div style="display:flex;flex-direction:column;gap:5px;">
            <button
              v-for="e in activeNodeEdges"
              :key="e.id"
              type="button"
              class="list-item"
              :class="{ active: activeEdge?.id === e.id }"
              @click="selectedEdgeId = e.id"
              @focus="selectedEdgeId = e.id"
            >
              <div class="list-item-row">
                <span class="list-item-title">
                  {{ nodeTitle(activeNode && e.from === activeNode.id ? e.to : e.from) }}
                </span>
                <span class="pill">{{ e.label }}</span>
              </div>
              <span class="list-item-sub">{{ e.snippet || '双链关联' }}</span>
            </button>
            <p v-if="!activeNodeEdges.length" class="hint">此笔记暂无连接。</p>
          </div>
        </div>

        <div style="padding-top:12px;border-top:1px solid var(--line-subtle);">
          <div class="kicker" style="margin-bottom:8px;">可能遗漏的关联</div>
          <div v-if="graph?.gaps.length" style="display:flex;flex-direction:column;gap:10px;">
            <div v-for="gap in graph.gaps" :key="gap.id" class="cs-card">
              <div style="display:flex;justify-content:space-between;align-items:center;">
                <span class="pill warning">待补充关联</span>
                <button
                  type="button"
                  class="btn quiet xs"
                  @click="emit('dismissGap', gap.id)"
                >忽略</button>
              </div>
              <strong style="font-size:13px;">{{ gap.title }}</strong>
              <p style="font-size:12px;color:var(--text-secondary);">{{ gap.reason }}</p>
              <button
                type="button"
                class="btn primary sm"
                @click="emit('askGap', gap)"
              >让 AI 帮忙查证</button>
            </div>
          </div>
          <p v-else class="hint">当前图谱关联完整，没有待补充缺口。</p>
        </div>
      </aside>
    </div>
  </div>
</template>
