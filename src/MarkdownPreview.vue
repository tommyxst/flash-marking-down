<script lang="ts">
import { markdownLanguage } from '@codemirror/lang-markdown'
import { computed, defineComponent, h, type VNodeChild } from 'vue'

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

function renderChildren(node: MdNode, src: string, exclude: Set<string> = SKIP_MARKS, trimLeading = false): VNodeChild[] {
  const out: VNodeChild[] = []
  let cursor = node.from
  let needTrim = trimLeading

  const pushText = (raw: string) => {
    const value = needTrim ? raw.replace(/^\s+/, '') : raw
    if (value) {
      out.push(value)
      needTrim = false
    }
  }

  for (let child = node.firstChild; child; child = child.nextSibling) {
    if (child.from > cursor) pushText(src.slice(cursor, child.from))
    if (!exclude.has(child.name)) {
      const rendered = renderNode(child, src)
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

function renderNode(node: MdNode, src: string): VNodeChild {
  const { name, from, to } = node
  switch (name) {
    case 'Document':
      return renderChildren(node, src)
    case 'ATXHeading1':
    case 'SetextHeading1':
      return h('h1', renderChildren(node, src, SKIP_MARKS, true))
    case 'ATXHeading2':
    case 'SetextHeading2':
      return h('h2', renderChildren(node, src, SKIP_MARKS, true))
    case 'ATXHeading3':
      return h('h3', renderChildren(node, src, SKIP_MARKS, true))
    case 'ATXHeading4':
      return h('h4', renderChildren(node, src, SKIP_MARKS, true))
    case 'ATXHeading5':
      return h('h5', renderChildren(node, src, SKIP_MARKS, true))
    case 'ATXHeading6':
      return h('h6', renderChildren(node, src, SKIP_MARKS, true))
    case 'Paragraph':
      return h('p', renderChildren(node, src))
    case 'Blockquote':
      return h('blockquote', renderChildren(node, src, SKIP_MARKS, true))
    case 'BulletList':
      return h('ul', renderChildren(node, src))
    case 'OrderedList':
      return h('ol', renderChildren(node, src))
    case 'ListItem':
      return h('li', renderChildren(node, src, SKIP_MARKS, true))
    case 'Task':
      return h('span', { class: 'md-task-line' }, renderChildren(node, src))
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
      return h('strong', renderChildren(node, src))
    case 'Emphasis':
      return h('em', renderChildren(node, src))
    case 'Strikethrough':
      return h('del', renderChildren(node, src))
    case 'InlineCode':
      return h('code', { class: 'md-inline-code' }, renderChildren(node, src))
    case 'Link': {
      const url = childText(node, src, 'URL').trim()
      const labelExclude = new Set([...SKIP_MARKS, 'URL', 'LinkTitle'])
      const label = renderChildren(node, src, labelExclude)
      return h('span', { class: 'md-link', title: url || undefined }, label.length ? label : src.slice(from, to))
    }
    case 'Autolink':
    case 'URL': {
      const url = src.slice(from, to).replace(/^<|>$/g, '')
      return h('span', { class: 'md-link', title: url }, url)
    }
    case 'Image': {
      // Never render remote or executable image tags automatically (SPECIFICATION.md #5).
      const url = childText(node, src, 'URL').trim()
      const alt = src
        .slice(from, to)
        .replace(/^!\[/, '')
        .replace(/\]\([\s\S]*\)$/, '')
        .trim()
      return h(
        'span',
        { class: 'md-image-stub', title: url || undefined },
        `[图片：${alt || '未命名'} · 已阻止自动外链加载]`,
      )
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
      if (node.firstChild) return renderChildren(node, src)
      return src.slice(from, to)
  }
}

export default defineComponent({
  name: 'MarkdownPreview',
  props: {
    source: { type: String, required: true },
  },
  setup(props) {
    const parsed = computed(() => {
      const { frontmatter, body } = splitFrontmatter(props.source)
      const tree = markdownLanguage.parser.parse(body)
      return { frontmatter, body, tree }
    })

    return () => {
      const { frontmatter, body, tree } = parsed.value
      const children: VNodeChild[] = []
      if (frontmatter) {
        children.push(h('pre', { class: 'md-frontmatter', 'aria-label': '文档元数据' }, frontmatter))
      }
      if (body.trim()) {
        children.push(renderNode(tree.topNode, body))
      } else if (!frontmatter) {
        children.push(h('p', { class: 'hint' }, '（空内容）'))
      }
      return h('div', { class: 'md-preview', tabindex: '0' }, children)
    }
  },
})
</script>
