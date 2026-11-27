import {
  acceptCompletion,
  autocompletion,
  completionKeymap,
  pickedCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from '@codemirror/autocomplete'
import { syntaxTree } from '@codemirror/language'
import { Prec, type EditorState, type Extension } from '@codemirror/state'
import { keymap, tooltips, type EditorView } from '@codemirror/view'
import type { WikilinkCandidate } from '../m2'

export interface WikilinkCompletionOptions {
  getCandidates: () => WikilinkCandidate[]
  isComposing?: () => boolean
  compact?: boolean
  maxItems?: number
}

interface WikilinkCompletion extends Completion {
  candidate: WikilinkCandidate
  matchedAlias: string | null
  titleRanges: number[]
}

const TRIGGER_BEFORE_RE = /(?:\[\[|【【)[^\[\]【】\]|#\r\n]{0,40}/
const CODE_NODE_NAMES = new Set([
  'FencedCode',
  'CodeBlock',
  'CodeText',
  'InlineCode',
  'CodeMark',
  'CodeInfo',
])

function isInFrontmatter(state: EditorState, pos: number): boolean {
  const { doc } = state
  if (doc.lines < 2) return false
  const firstText = doc.line(1).text.replace(/^\uFEFF/, '')
  if (firstText.trim() !== '---') return false

  let closingLine = -1
  const maxScan = Math.min(doc.lines, 120)
  for (let i = 2; i <= maxScan; i += 1) {
    if (doc.line(i).text.trim() === '---') {
      closingLine = i
      break
    }
  }
  if (closingLine < 0) return false
  return pos <= doc.line(closingLine).to
}

function isInCodeContext(state: EditorState, pos: number, triggerFrom: number): boolean {
  const tree = syntaxTree(state)
  let node: ReturnType<typeof tree.resolveInner> | null = tree.resolveInner(pos, -1)
  while (node) {
    if (CODE_NODE_NAMES.has(node.name)) return true
    node = node.parent
  }

  // Also guard against unclosed inline backticks earlier on the same line.
  const line = state.doc.lineAt(triggerFrom)
  const beforeTrigger = line.text.slice(0, Math.max(0, triggerFrom - line.from))
  let backtickSpans = 0
  for (let i = 0; i < beforeTrigger.length; i += 1) {
    if (beforeTrigger[i] === '\\') {
      i += 1
      continue
    }
    if (beforeTrigger[i] === '`') {
      backtickSpans += 1
      while (i + 1 < beforeTrigger.length && beforeTrigger[i + 1] === '`') i += 1
    }
  }
  return backtickSpans % 2 === 1
}

export function hasWikilinkTrigger(state: EditorState): boolean {
  const sel = state.selection.main
  if (!sel.empty) return false
  const pos = sel.head
  if (isInFrontmatter(state, pos)) return false
  const line = state.doc.lineAt(pos)
  const textBefore = line.text.slice(0, pos - line.from)
  const m = textBefore.match(/(?:\[\[|【【)[^\[\]【】\]|#\r\n]{0,40}$/)
  if (!m) return false
  const triggerFrom = pos - m[0].length
  return !isInCodeContext(state, pos, triggerFrom)
}

function findSubstringRanges(text: string, queryLower: string): number[] {
  if (!queryLower) return []
  const idx = text.toLowerCase().indexOf(queryLower)
  if (idx < 0) return []
  return [idx, idx + queryLower.length]
}

function findSubsequenceMatch(text: string, queryLower: string): { ranges: number[]; span: number } | null {
  if (!queryLower) return null
  const lower = text.toLowerCase()
  const positions: number[] = []
  let cursor = 0
  for (let i = 0; i < queryLower.length; i += 1) {
    const ch = queryLower[i]
    const found = lower.indexOf(ch, cursor)
    if (found < 0) return null
    positions.push(found)
    cursor = found + 1
  }
  const ranges: number[] = []
  let start = positions[0]
  let end = start + 1
  for (let i = 1; i < positions.length; i += 1) {
    if (positions[i] === end) {
      end += 1
    } else {
      ranges.push(start, end)
      start = positions[i]
      end = start + 1
    }
  }
  ranges.push(start, end)
  const span = positions[positions.length - 1] - positions[0] + 1
  return { ranges, span }
}

interface ScoredCandidate {
  candidate: WikilinkCandidate
  score: number
  matchedAlias: string | null
  titleRanges: number[]
}

function scoreCandidate(candidate: WikilinkCandidate, queryLower: string): ScoredCandidate | null {
  if (!queryLower) {
    return { candidate, score: 1, matchedAlias: null, titleRanges: [] }
  }

  const titleLower = candidate.title.toLowerCase()
  const pathLower = candidate.path.toLowerCase()
  const stemLower = candidate.stem.toLowerCase()

  // 1. Title prefix
  if (titleLower.startsWith(queryLower)) {
    return {
      candidate,
      score: 500 - Math.min(candidate.title.length, 40) * 0.1,
      matchedAlias: null,
      titleRanges: [0, queryLower.length],
    }
  }

  // 2. Alias prefix
  for (const alias of candidate.aliases) {
    if (alias.toLowerCase().startsWith(queryLower)) {
      return {
        candidate,
        score: 400 - Math.min(alias.length, 40) * 0.1,
        matchedAlias: alias,
        titleRanges: findSubstringRanges(candidate.title, queryLower),
      }
    }
  }

  // 3. Title substring
  const titleIdx = titleLower.indexOf(queryLower)
  if (titleIdx >= 0) {
    return {
      candidate,
      score: 300 - titleIdx * 0.2,
      matchedAlias: null,
      titleRanges: [titleIdx, titleIdx + queryLower.length],
    }
  }

  // 4. Alias substring
  for (const alias of candidate.aliases) {
    const aliasIdx = alias.toLowerCase().indexOf(queryLower)
    if (aliasIdx >= 0) {
      return {
        candidate,
        score: 250 - aliasIdx * 0.2,
        matchedAlias: alias,
        titleRanges: [],
      }
    }
  }

  // 5. Path / stem substring
  const stemIdx = stemLower.indexOf(queryLower)
  if (stemIdx >= 0) {
    return {
      candidate,
      score: 210 - stemIdx * 0.1,
      matchedAlias: null,
      titleRanges: [],
    }
  }
  const pathIdx = pathLower.indexOf(queryLower)
  if (pathIdx >= 0) {
    return {
      candidate,
      score: 190 - pathIdx * 0.05,
      matchedAlias: null,
      titleRanges: [],
    }
  }

  // 6. Subsequence fuzzy match (title > alias > path)
  const titleSub = findSubsequenceMatch(candidate.title, queryLower)
  if (titleSub) {
    return {
      candidate,
      score: 120 - Math.min(titleSub.span, 60) * 0.5,
      matchedAlias: null,
      titleRanges: titleSub.ranges,
    }
  }

  for (const alias of candidate.aliases) {
    const aliasSub = findSubsequenceMatch(alias, queryLower)
    if (aliasSub) {
      return {
        candidate,
        score: 100 - Math.min(aliasSub.span, 60) * 0.5,
        matchedAlias: alias,
        titleRanges: [],
      }
    }
  }

  const pathSub = findSubsequenceMatch(candidate.path, queryLower)
  if (pathSub) {
    return {
      candidate,
      score: 70 - Math.min(pathSub.span, 80) * 0.3,
      matchedAlias: null,
      titleRanges: [],
    }
  }

  return null
}

function buildLinkInnerText(candidate: WikilinkCandidate, matchedAlias: string | null): string {
  const cleanLink = (candidate.linkText || '').replace(/^\[\[|\]\]$/g, '').trim()
  if (!matchedAlias) {
    if (cleanLink) return cleanLink
    if (candidate.ambiguousStem) {
      const target = candidate.folder ? `${candidate.folder}/${candidate.stem}` : candidate.path.replace(/\.md$/i, '')
      return `${target}|${candidate.title}`
    }
    return candidate.stem === candidate.title ? candidate.stem : `${candidate.stem}|${candidate.title}`
  }

  // Alias match selected: keep folder disambiguation when ambiguousStem is true.
  let target = cleanLink ? cleanLink.split('|')[0].trim() : ''
  if (candidate.ambiguousStem) {
    if (!target.includes('/')) {
      target = candidate.folder ? `${candidate.folder}/${candidate.stem}` : candidate.path.replace(/\.md$/i, '')
    }
  } else if (!target) {
    target = candidate.stem
  }
  return `${target}|${matchedAlias}`
}

function applyWikilinkCompletion(
  view: EditorView,
  completion: Completion,
  from: number,
  to: number,
) {
  const item = completion as WikilinkCompletion
  const inner = buildLinkInnerText(item.candidate, item.matchedAlias)
  const insert = `[[${inner}]]`

  // Absorb immediately following ]] or 】】 to prevent double closing brackets.
  let replaceTo = to
  const trailing = view.state.doc.sliceString(to, Math.min(view.state.doc.length, to + 2))
  if (trailing === ']]' || trailing === '】】') {
    replaceTo = to + 2
  }

  view.dispatch({
    changes: { from, to: replaceTo, insert },
    selection: { anchor: from + insert.length },
    userEvent: 'input.complete',
    annotations: pickedCompletion.of(completion),
  })
}

export function createWikilinkAutocomplete(options: WikilinkCompletionOptions): Extension {
  const compact = Boolean(options.compact)
  const maxItems = options.maxItems ?? (compact ? 6 : 8)

  const source = (context: CompletionContext): CompletionResult | null => {
    // Never pop up while IME composition is active.
    if (options.isComposing?.() || context.view?.composing) return null
    if (isInFrontmatter(context.state, context.pos)) return null

    const match = context.matchBefore(TRIGGER_BEFORE_RE)
    if (!match) return null
    if (isInCodeContext(context.state, context.pos, match.from)) return null

    const candidates = options.getCandidates()
    if (!candidates.length) return null

    const queryLower = match.text.slice(2).trim().toLowerCase()
    const scored: ScoredCandidate[] = []
    for (const candidate of candidates) {
      const hit = scoreCandidate(candidate, queryLower)
      if (hit) scored.push(hit)
    }
    if (!scored.length) return null

    if (queryLower) {
      scored.sort((a, b) => {
        if (b.score !== a.score) return b.score - a.score
        if (a.candidate.title.length !== b.candidate.title.length) {
          return a.candidate.title.length - b.candidate.title.length
        }
        return a.candidate.title.localeCompare(b.candidate.title, 'zh-CN')
      })
    }

    const picked = scored.slice(0, maxItems)
    const completions: WikilinkCompletion[] = picked.map((entry) => ({
      label: entry.candidate.title,
      candidate: entry.candidate,
      matchedAlias: entry.matchedAlias,
      titleRanges: entry.titleRanges,
      apply: applyWikilinkCompletion,
    }))

    return {
      from: match.from,
      to: match.to,
      options: completions,
      filter: false,
      getMatch: (completion) => (completion as WikilinkCompletion).titleRanges ?? [],
      update: (_current, _from, _to, nextCtx) => source(nextCtx),
    }
  }

  const tooltipExt = tooltips({
    position: 'fixed',
    tooltipSpace: () => {
      const w = window.innerWidth
      const h = window.innerHeight
      if (compact) {
        return {
          top: 44,
          left: 8,
          right: Math.max(200, w - 8),
          bottom: Math.max(120, h - 54),
        }
      }
      return {
        top: 48,
        left: 12,
        right: Math.max(300, w - 12),
        bottom: Math.max(200, h - 36),
      }
    },
  })

  const autocompleteExt = autocompletion({
    override: [source],
    defaultKeymap: false,
    activateOnTyping: true,
    activateOnTypingDelay: 40,
    interactionDelay: 0,
    icons: false,
    maxRenderedOptions: maxItems,
    tooltipClass: () => (compact ? 'wikra-ac-tooltip wikra-ac-compact' : 'wikra-ac-tooltip'),
    addToOptions: [
      {
        position: 60,
        render(completion) {
          const item = completion as WikilinkCompletion
          if (!item.candidate?.typeLabel) return null
          const pill = document.createElement('span')
          pill.className = 'cm-wikilink-type'
          pill.textContent = item.candidate.typeLabel
          return pill
        },
      },
      {
        position: 85,
        render(completion) {
          const item = completion as WikilinkCompletion
          const sub = document.createElement('span')
          sub.className = 'cm-wikilink-sub'
          if (item.matchedAlias) {
            sub.textContent = `别名命中：${item.matchedAlias}`
          } else {
            const folder = item.candidate.folder
            sub.textContent = folder ? `${folder.replace(/\/+$/, '')}/` : item.candidate.path
            sub.classList.add('mono')
          }
          return sub
        },
      },
    ],
  })

  const keymapExt = Prec.highest(
    keymap.of([
      { key: 'Tab', run: acceptCompletion },
      ...completionKeymap,
    ]),
  )

  return [tooltipExt, autocompleteExt, keymapExt]
}
