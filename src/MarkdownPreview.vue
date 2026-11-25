<script lang="ts">
import { markdownLanguage } from '@codemirror/lang-markdown'
import { computed, defineComponent, h, ref, watch, type PropType, type VNodeChild } from 'vue'
import { api } from './api/client'
import { citationKindLabels, type CitationItem } from './m2'

type MdNode = ReturnType<typeof markdownLanguage.parser.parse>['topNode']

const SKIP_MARKS = new Set([
  'HeaderMark',
  'QuoteMark',
  'ListMark',
  'EmphasisMark',
  'StrikethroughMark',
  'CodeMark',
  'LinkMark',
])

function imagePlaceholder(src: string, alt: string, note: string) {
  return h('span', { class: 'md-image-stub', title: src || undefined }, `[图片：${alt || '未命名'} · ${note}]`)
}

const LocalImage = defineComponent({
  name: 'LocalImage',
  props: {
    session: { type: String, default: '' },
    basePath: { type: String, default: undefined },
    src: { type: String, required: true },
    alt: { type: String, default: '' },
  },
  setup(props) {
    const dataUrl = ref('')
    const failed = ref(false)

    function isSafeRelative(url: string) {
      const clean = url.trim()
      if (!clean || clean.startsWith('//') || clean.startsWith('/')) return false
      return !/^[a-z][a-z0-9+.-]*:/i.test(clean)
    }

    watch(
      () => [props.session, props.basePath, props.src] as const,
      async ([session, basePath, src]) => {
        dataUrl.value = ''
        failed.value = false
        if (!session || !isSafeRelative(src)) {
          failed.value = true
          return
        }
        try {
          dataUrl.value = await api.readWorkspaceImage(session, basePath, src)
        } catch {
          failed.value = true
        }
      },
      { immediate: true },
    )

    return () => {
      if (dataUrl.value) {
        return h('img', {
          class: 'md-local-image',
          src: dataUrl.value,
          alt: props.alt || '本地图片',
        })
      }
      const note = isSafeRelative(props.src)
        ? (failed.value ? '本地图片无法加载' : '正在加载本地图片…')
        : '已阻止自动外链加载'
      return imagePlaceholder(props.src, props.alt, note)
    }
  },
})

function splitFrontmatter(source: string): { frontmatter: string | null; body: string } {
  const normalized = source.replace(/^\uFEFF/, '').replaceAll('\r\n', '\n')
  if (!normalized.startsWith('---\n')) return { frontmatter: null, body: normalized }
  const end = normalized.indexOf('\n---', 4)
  if (end < 0) return { frontmatter: null, body: normalized }
  const afterClose = end + 4
  if (afterClose < normalized.length && normalized[afterClose] !== '\n') {
    return { frontmatter: null, body: normalized }
  }
  const frontmatter = normalized.slice(4, end).trim()
  const body = normalized.slice(afterClose).replace(/^\n/, '')
  return { frontmatter: frontmatter || null, body }
}

function childText(node: MdNode, src: string, targetName: string): string {
  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.name === targetName) return src.slice(child.from, child.to)
  }
  return ''
}

type RenderCtx = {
  session?: string
  basePath?: string
  citations?: CitationItem[]
  onCite: (citation: CitationItem) => void
}

function pushTextWithCitations(out: VNodeChild[], raw: string, ctx: RenderCtx) {
  if (!ctx.citations?.length) {
    out.push(raw)
    return
  }
  let last = 0
  for (const match of raw.matchAll(/\[(\d+)\]/g)) {
    const citation = ctx.citations.find((item) => item.index === Number(match[1]))
    if (!citation) continue
    const start = match.index ?? 0
    if (start > last) out.push(raw.slice(last, start))
    const label = `查看 ${citationKindLabels[citation.kind]} 引用 ${match[0]}`
    out.push(
      h(
        'button',
        {
          type: 'button',
          class: 'cite-pill',
          title: label,
          'aria-label': label,
          onClick: () => ctx.onCite(citation),
        },
        match[0],
      ),
    )
    last = start + match[0].length
  }
  if (last < raw.length) out.push(raw.slice(last))
}

function renderChildren(
  node: MdNode,
  src: string,
  ctx: RenderCtx,
  exclude: Set<string> = SKIP_MARKS,
  trimLeading = false,
): VNodeChild[] {
  const out: VNodeChild[] = []
  let cursor = node.from
  let needTrim = trimLeading

  const pushText = (raw: string) => {
    const value = needTrim ? raw.replace(/^\s+/, '') : raw
    if (value) {
      pushTextWithCitations(out, value, ctx)
      needTrim = false
    }
  }

  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.from > cursor) pushText(src.slice(cursor, child.from))
    if (!exclude.has(child.name)) {
      const rendered = renderNode(child, src, ctx)
      if (rendered !== null) {
        out.push(rendered)
        needTrim = false
      }
    }
    cursor = child.to
  }
  if (cursor < node.to) pushText(src.slice(cursor, node.to))
  return out
}

function renderNode(node: MdNode, src: string, ctx: RenderCtx): VNodeChild {
  const { name, from, to } = node
  switch (name) {
    case 'Document':
      return renderChildren(node, src, ctx)
    case 'ATXHeading1':
    case 'SetextHeading1':
      return h('h1', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'ATXHeading2':
    case 'SetextHeading2':
      return h('h2', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'ATXHeading3':
      return h('h3', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'ATXHeading4':
      return h('h4', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'ATXHeading5':
      return h('h5', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'ATXHeading6':
      return h('h6', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'Paragraph':
      return h('p', renderChildren(node, src, ctx))
    case 'Blockquote':
      return h('blockquote', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'BulletList':
      return h('ul', renderChildren(node, src, ctx))
    case 'OrderedList':
      return h('ol', renderChildren(node, src, ctx))
    case 'ListItem':
      return h('li', renderChildren(node, src, ctx, SKIP_MARKS, true))
    case 'Task':
      return h('span', { class: 'md-task-line' }, renderChildren(node, src, ctx))
    case 'TaskMarker': {
      const raw = src.slice(from, to)
      const done = raw === '[x]' || raw === '[X]'
      const cancelled = raw === '[-]'
      const glyph = done ? '☑' : cancelled ? '☒' : '☐'
      const label = done ? '已完成' : cancelled ? '已取消' : '未完成'
      return h('span', { class: 'md-task-box', role: 'img', 'aria-label': label }, glyph)
    }
    case 'FencedCode':
    case 'CodeBlock': {
      const info = childText(node, src, 'CodeInfo').trim()
      const code = childText(node, src, 'CodeText') || src.slice(from, to).replace(/^```[^\n]*\n?/, '').replace(/\n?```$/, '')
      return h('pre', { class: 'md-code-block', 'data-lang': info || undefined }, h('code', code))
    }
    case 'HorizontalRule':
      return h('hr')
    case 'StrongEmphasis':
      return h('strong', renderChildren(node, src, ctx))
    case 'Emphasis':
      return h('em', renderChildren(node, src, ctx))
    case 'Strikethrough':
      return h('del', renderChildren(node, src, ctx))
    case 'InlineCode':
      return h('code', { class: 'md-inline-code' }, renderChildren(node, src, ctx))
    case 'Link': {
      const raw = src.slice(from, to)
      // The parser treats bare [n] citation markers as shortcut links.
      if (/^(?:\[\d+\])+$/.test(raw)) {
        const citations: VNodeChild[] = []
        pushTextWithCitations(citations, raw, ctx)
        return citations
      }
      const url = childText(node, src, 'URL').trim()
      const labelExclude = new Set([...SKIP_MARKS, 'URL', 'LinkTitle'])
      const label = renderChildren(node, src, ctx, labelExclude)
      return h('span', { class: 'md-link', title: url || undefined }, label.length ? label : raw)
    }
    case 'Autolink':
    case 'URL': {
      const url = src.slice(from, to).replace(/^<|>$/g, '')
      return h('span', { class: 'md-link', title: url }, url)
    }
    case 'Image': {
      const url = childText(node, src, 'URL').trim()
      const alt = src
        .slice(from, to)
        .replace(/^!\[/, '')
        .replace(/\]\([\s\S]*\)$/, '')
        .trim()
      if (!ctx.session) return imagePlaceholder(url, alt, '此处不加载图片')
      return h(LocalImage, { session: ctx.session, basePath: ctx.basePath, src: url, alt })
    }
    case 'HardBreak':
      return h('br')
    case 'Escape':
      return src.slice(from + 1, to)
    case 'HTMLBlock':
    case 'HTMLTag':
    case 'CommentBlock':
    case 'Comment':
      return h('code', { class: 'md-raw-html' }, src.slice(from, to))
    case 'Table':
      return h('pre', { class: 'md-code-block' }, src.slice(from, to))
    default:
      if (node.firstChild) return renderChildren(node, src, ctx)
      return src.slice(from, to)
  }
}

export default defineComponent({
  name: 'MarkdownPreview',
  props: {
    source: { type: String, required: true },
    session: { type: String, default: '' },
    basePath: { type: String, default: undefined },
    compact: { type: Boolean, default: false },
    citations: { type: Array as PropType<CitationItem[]> },
  },
  emits: { cite: (_citation: CitationItem) => true },
  setup(props, { emit }) {
    const parsed = computed(() => {
      const { frontmatter, body } = splitFrontmatter(props.source)
      const tree = markdownLanguage.parser.parse(body)
      return { frontmatter, body, tree }
    })

    return () => {
      const { frontmatter, body, tree } = parsed.value
      const ctx: RenderCtx = {
        session: props.session,
        basePath: props.basePath,
        citations: props.citations,
        onCite: (citation) => emit('cite', citation),
      }
      const children: VNodeChild[] = []
      if (frontmatter) {
        children.push(h('pre', { class: 'md-frontmatter', 'aria-label': '文档元数据' }, frontmatter))
      }
      if (body.trim()) {
        children.push(renderNode(tree.topNode, body, ctx))
      } else if (!frontmatter) {
        children.push(h('p', { class: 'hint' }, '（空内容）'))
      }
      return h('div', { class: ['md-preview', { 'md-preview-compact': props.compact }], tabindex: '0' }, children)
    }
  },
})
</script>
