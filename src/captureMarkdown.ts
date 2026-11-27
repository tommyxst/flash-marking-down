import { syntaxTree } from '@codemirror/language'
import type { EditorSelection, EditorState, Line, Range } from '@codemirror/state'
import { Decoration, EditorView, ViewPlugin, WidgetType } from '@codemirror/view'
import type { DecorationSet, ViewUpdate } from '@codemirror/view'

class MarkdownMarker extends WidgetType {
  constructor(readonly glyph: string, readonly label?: string) { super() }

  toDOM(view: EditorView) {
    const marker = view.dom.ownerDocument.createElement('span')
    marker.className = 'cm-md-marker'
    marker.textContent = this.glyph
    if (this.label) {
      marker.setAttribute('role', 'img')
      marker.setAttribute('aria-label', this.label)
    } else {
      marker.setAttribute('aria-hidden', 'true')
    }
    return marker
  }

  ignoreEvent() { return false }
}

const hidden = Decoration.replace({})
const bullet = Decoration.replace({ widget: new MarkdownMarker('•') })
const quote = Decoration.replace({ widget: new MarkdownMarker('│') })
const checked = Decoration.replace({ widget: new MarkdownMarker('☑', '已完成') })
const unchecked = Decoration.replace({ widget: new MarkdownMarker('☐', '未完成') })
const syntax = Decoration.mark({ class: 'cm-md-syntax' })
const h1 = Decoration.line({ class: 'cm-md-h1' })
const h2 = Decoration.line({ class: 'cm-md-h2' })
const h3 = Decoration.line({ class: 'cm-md-h3' })
const h4 = Decoration.line({ class: 'cm-md-h4' })
const lineStyles = new Map([
  ['ATXHeading1', h1],
  ['ATXHeading2', h2],
  ['ATXHeading3', h3],
  ['ATXHeading4', h4],
  ['ATXHeading5', h4],
  ['ATXHeading6', h4],
  ['SetextHeading1', h1],
  ['SetextHeading2', h2],
  ['Blockquote', Decoration.line({ class: 'cm-md-quote-line' })],
])
const link = Decoration.mark({ class: 'cm-md-link' })
const wikilink = Decoration.mark({ class: 'cm-md-wikilink' })
const WIKILINK_RE = /\[\[[^\]\r\n]+\]\]/g
const inlineStyles = new Map([
  ['StrongEmphasis', Decoration.mark({ class: 'cm-md-strong' })],
  ['Emphasis', Decoration.mark({ class: 'cm-md-emphasis' })],
  ['Strikethrough', Decoration.mark({ class: 'cm-md-strikethrough' })],
  ['InlineCode', Decoration.mark({ class: 'cm-md-code' })],
  ['CodeText', Decoration.mark({ class: 'cm-md-code' })],
])
const invisibleMarkers = new Set(['EmphasisMark', 'StrikethroughMark', 'CodeMark', 'CodeInfo'])

function isLineActive(selection: EditorSelection, lineFrom: number, lineTo: number) {
  return selection.ranges.some((range) => range.from <= lineTo && range.to >= lineFrom)
}

export function buildCaptureDecorations(state: EditorState, from: number, to: number): DecorationSet {
  const { doc, selection } = state
  const ranges: Range<Decoration>[] = []
  const decoratedLines = new Map<Decoration, Set<number>>()

  function forEachLine(start: number, end: number, visit: (line: Line, start: number, end: number) => void) {
    const limit = Math.min(end, to)
    for (let pos = Math.max(start, from); pos < limit;) {
      const line = doc.lineAt(pos)
      visit(line, pos, Math.min(limit, line.to))
      pos = line.to + 1
    }
  }

  function decorateLines(start: number, end: number, decoration: Decoration) {
    const positions = decoratedLines.get(decoration) ?? new Set<number>()
    decoratedLines.set(decoration, positions)
    forEachLine(start, end, (line) => {
      if (positions.has(line.from)) return
      positions.add(line.from)
      ranges.push(decoration.range(line.from))
    })
  }

  function decorate(start: number, end: number, decoration: Decoration | null, activeDecoration = decoration) {
    forEachLine(start, end, (line, pos, lineEnd) => {
      if (pos >= lineEnd) return
      const style = activeDecoration !== decoration && isLineActive(selection, line.from, line.to)
        ? activeDecoration : decoration
      if (style) ranges.push(style.range(pos, lineEnd))
    })
  }

  function decorateMarker(start: number, end: number, replacement: Decoration | null = hidden) {
    decorate(start, end, replacement, syntax)
  }

  function skipSpace(from: number) {
    const line = doc.lineAt(from)
    const rest = line.text.slice(from - line.from)
    return from + rest.length - rest.trimStart().length
  }

  syntaxTree(state).iterate({
    from,
    to,
    enter(node) {
      const { name, from, to } = node
      const style = inlineStyles.get(name)
      if (style) decorate(from, to, style)
      if (invisibleMarkers.has(name)) decorateMarker(from, to)
      const lineStyle = lineStyles.get(name)
      if (lineStyle) {
        const underline = name.startsWith('SetextHeading') ? node.node.getChild('HeaderMark') : null
        decorateLines(from, underline ? doc.lineAt(underline.from).from : to, lineStyle)
      }

      switch (name) {
        case 'HeaderMark': {
          const parent = node.node.parent
          if (parent?.name.startsWith('SetextHeading')) decorateMarker(from, to)
          else if (from === parent?.from) decorateMarker(from, skipSpace(to))
          break
        }
        case 'QuoteMark':
          decorateMarker(from, to, quote)
          break
        case 'ListMark':
          if (node.node.parent?.parent?.name === 'BulletList') {
            if (node.node.nextSibling?.name === 'Task') decorateMarker(from, skipSpace(to))
            else decorateMarker(from, to, bullet)
          } else decorateMarker(from, to, null)
          break
        case 'TaskMarker':
          decorateMarker(from, to, doc.sliceString(from, to) === '[ ]' ? unchecked : checked)
          break
        case 'Link':
        case 'Autolink': {
          const markers = node.node.getChildren('LinkMark')
          // Lezer also parses undefined references as Link nodes.
          if (markers.length < (name === 'Link' ? 3 : 2)) break
          const [opening, closing] = markers
          decorateMarker(opening.from, opening.to)
          decorate(opening.to, closing.from, link)
          decorateMarker(closing.from, to)
          break
        }
        case 'URL':
          if (node.node.parent?.name !== 'Link' && node.node.parent?.name !== 'Autolink') decorate(from, to, link)
          break
        case 'Image':
        case 'LinkReference':
          return false
      }
    },
  })

  // Highlight closed [[wikilinks]] gently without concealing brackets.
  const codeSpans: [number, number][] = []
  syntaxTree(state).iterate({
    from,
    to,
    enter(node) {
      if (node.name === 'FencedCode' || node.name === 'CodeBlock' || node.name === 'InlineCode') {
        codeSpans.push([node.from, node.to])
        return false
      }
    },
  })
  const inCodeSpan = (pos: number) => codeSpans.some(([s, e]) => pos >= s && pos < e)
  forEachLine(from, to, (line, lineStart, lineEnd) => {
    const slice = doc.sliceString(lineStart, lineEnd)
    WIKILINK_RE.lastIndex = 0
    for (const match of slice.matchAll(WIKILINK_RE)) {
      const idx = match.index ?? 0
      const absFrom = lineStart + idx
      const absTo = absFrom + match[0].length
      if (!inCodeSpan(absFrom)) {
        ranges.push(wikilink.range(absFrom, absTo))
      }
    }
  })

  return Decoration.set(ranges, true)
}

export const captureMarkdown = ViewPlugin.fromClass(class {
  decorations

  constructor(view: EditorView) {
    this.decorations = buildCaptureDecorations(view.state, view.viewport.from, view.viewport.to)
  }

  update(update: ViewUpdate) {
    if (update.docChanged || update.selectionSet || update.viewportChanged
      || syntaxTree(update.state) !== syntaxTree(update.startState)) {
      const { from, to } = update.view.viewport
      this.decorations = buildCaptureDecorations(update.state, from, to)
    }
  }
}, { decorations: (plugin) => plugin.decorations })
