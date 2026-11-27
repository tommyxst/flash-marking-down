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
export const WIKRA_WIKI_CACHE_KEY = 'wikra-wiki-candidates-v2'

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
