import type { DocWidthMode, ThemeName } from '../m2'

export function isLoopbackUrl(url: string): boolean {
  return /^https?:\/\/(127\.0\.0\.1|localhost|\[::1\])(?::\d+)?(\/|$)/i.test(url.trim())
}

export function formatCountdown(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

export function shortDate(value: string | null | undefined): string {
  if (!value) return '未记录'
  const clean = value.trim()
  const isoMatch = clean.match(/^(\d{4})-(\d{2}-\d{2})/)
  if (isoMatch) return isoMatch[2]
  return clean.slice(0, 16)
}

export const WIKRA_PREFS_STORAGE_KEY = 'wikra-ui-prefs-v2'

export function opKindLabel(op: string): string {
  switch (op) {
    case 'create_page':
      return '新建'
    case 'append_section':
      return '追加'
    case 'replace_section':
      return '替换章节'
    case 'add_link':
      return '补双链'
    case 'set_frontmatter_key':
      return '补别名'
    default:
      return '更新'
  }
}

export function changesetStatusLabel(status: string): string {
  switch (status) {
    case 'applied':
      return '已完成'
    case 'partial':
      return '含待确认项'
    case 'partially_reverted':
      return '已部分撤销'
    case 'reverted':
      return '已撤销'
    default:
      return '整理记录'
  }
}

export function changesetStatusTone(status: string): string {
  switch (status) {
    case 'applied':
      return 'success'
    case 'partial':
      return 'warning'
    case 'partially_reverted':
      return 'info'
    default:
      return ''
  }
}

export function pendingReasonLabel(reason: string): string {
  const r = reason.trim()
  if (!r) return '需人工确认'
  const lower = r.toLowerCase()
  if (lower.includes('replace_section') || r.includes('替换')) return '替换已有章节'
  if (lower.includes('wikra-core') || lower.includes('decision') || r.includes('决策') || r.includes('核心')) return '核心决策页'
  if (lower.includes('conflict') || r.includes('冲突') || r.includes('分歧')) return '存在观点分歧'
  if (lower.includes('uncertain') || r.includes('不确定')) return '结论待核实'
  if (lower.includes('thin') || r.includes('单薄')) return '单薄新页'
  if (lower.includes('heading') || lower.includes('h2') || r.includes('同名')) return '章节标题重复'
  if (lower.includes('hash') || lower.includes('modified') || r.includes('变动')) return '来源变动'
  if (lower.includes('batch') || lower.includes('15') || r.includes('批量')) return '涉及页面较多'
  return r
}

export function lintRuleLabel(rule: string): string {
  switch (rule) {
    case 'broken_link':
      return '断链'
    case 'missing_source':
    case 'quote_mismatch':
      return '引文异常'
    case 'structure_blocked':
      return '标记未闭合'
    case 'broken_anchor':
      return '锚点失效'
    case 'ambiguous_name':
      return '重名链接'
    case 'stale_conflict_marker':
      return '残留冲突标记'
    case 'index_drift':
      return '目录页待同步'
    case 'orphan':
      return '尚未关联'
    case 'thin_page':
      return '内容较单薄'
    case 'semantic_duplicate':
      return '疑似重复内容'
    case 'semantic_contradiction':
      return '疑似观点冲突'
    default:
      return '体检提醒'
  }
}

export function shortChangesetId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id
}

export function applyBodyPreferences(theme: ThemeName, docWidth: DocWidthMode) {
  const safeTheme: ThemeName = theme === 'paper' ? 'paper' : 'slate'
  const safeWidth: DocWidthMode = docWidth === 'full' ? 'full' : 'readable'
  document.body.dataset.theme = safeTheme
  document.body.dataset.width = safeWidth
  if (!document.body.dataset.density) {
    document.body.dataset.density = 'standard'
  }
  try {
    window.localStorage.setItem(WIKRA_PREFS_STORAGE_KEY, JSON.stringify({ theme: safeTheme, docWidth: safeWidth }))
  } catch {
    // Ignore storage quota / sandbox restrictions.
  }
}

export function readCachedPreferences(): { theme: ThemeName; docWidth: DocWidthMode } {
  try {
    const raw = window.localStorage.getItem(WIKRA_PREFS_STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as { theme?: string; docWidth?: string }
      return {
        theme: parsed.theme === 'paper' ? 'paper' : 'slate',
        docWidth: parsed.docWidth === 'full' ? 'full' : 'readable',
      }
    }
  } catch {
    // Fallback to defaults.
  }
  return { theme: 'slate', docWidth: 'readable' }
}

export function isUserTypingInEditor(): boolean {
  const active = document.activeElement
  if (!active || active === document.body) return false
  if (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA') return true
  if ((active as HTMLElement).isContentEditable) return true
  return Boolean(active.closest('.cm-editor'))
}

export interface ParsedWikiBody {
  leadMarkdown: string
  sections: { heading: string; body: string }[]
  protectedText: string
}

const PROTECT_BLOCK_RE = /<!--\s*(?:wikra|fmd)-protect:start\s*-->([\s\S]*?)<!--\s*(?:wikra|fmd)-protect:end\s*-->/i
const FOOTNOTE_DEF_LINE_RE = /^\[\^[^\]]+\]:.*$/gm

/**
 * Strips YAML frontmatter, separates the protected "我的理解" block and footnote definitions,
 * and splits remaining content into H2 sections so active recall can mask each section cleanly
 * while still rendering all Markdown through the hardened MarkdownPreview component.
 */
export function parseWikiBodyForView(rawBody: string): ParsedWikiBody {
  let body = rawBody.replace(/^\uFEFF/, '').replaceAll('\r\n', '\n')

  if (body.startsWith('---\n')) {
    const end = body.indexOf('\n---', 4)
    if (end >= 0) {
      const afterClose = end + 4
      if (afterClose >= body.length || body[afterClose] === '\n') {
        body = body.slice(afterClose).replace(/^\n/, '')
      }
    }
  }

  // Remove top-level H1 if it duplicates the title header row
  body = body.replace(/^#\s+[^\n]+\n+/, '')

  let protectedText = ''
  const protectMatch = body.match(PROTECT_BLOCK_RE)
  if (protectMatch) {
    protectedText = protectMatch[1]
      .replace(/^\s*##\s*我的理解\s*\n*/i, '')
      .trim()
    body = body.replace(PROTECT_BLOCK_RE, '').trim()
  }

  // Strip standalone footnote definitions because WikiDocumentView renders structured footnotes
  body = body.replace(FOOTNOTE_DEF_LINE_RE, '').replace(/\n{3,}/g, '\n\n').trim()

  const lines = body.split('\n')
  const leadLines: string[] = []
  const sections: { heading: string; body: string }[] = []
  let current: { heading: string; lines: string[] } | null = null
  let inFence = false

  for (const line of lines) {
    if (/^\s*```/.test(line)) inFence = !inFence
    if (!inFence && /^##\s+/.test(line)) {
      if (current) {
        sections.push({ heading: current.heading, body: current.lines.join('\n').trim() })
      }
      current = { heading: line.replace(/^##\s+/, '').trim(), lines: [] }
    } else if (current) {
      current.lines.push(line)
    } else {
      leadLines.push(line)
    }
  }
  if (current) {
    sections.push({ heading: current.heading, body: current.lines.join('\n').trim() })
  }

  return {
    leadMarkdown: leadLines.join('\n').trim(),
    sections,
    protectedText,
  }
}

export function slugifyMarkdownPath(input: string): string {
  const trimmed = input.trim().replaceAll('\\', '/').replace(/^\/+/, '')
  if (!trimmed) return ''
  if (trimmed.includes('/')) {
    const segments = trimmed
      .split('/')
      .map((seg) => seg.trim().replace(/[<>:"|?*]/g, '-'))
      .filter(Boolean)
    if (!segments.length) return ''
    const joined = segments.join('/')
    return /\.md$/i.test(joined) ? joined : `${joined}.md`
  }
  const safe = trimmed.replace(/[<>:"|?*]/g, '-').replace(/\s+/g, '-')
  return /\.md$/i.test(safe) ? `wiki/concepts/${safe}` : `wiki/concepts/${safe}.md`
}
