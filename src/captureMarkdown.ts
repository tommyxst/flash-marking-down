import { syntaxTree } from '@codemirror/language'
import type { Range } from '@codemirror/state'
import { Decoration, EditorView, ViewPlugin, WidgetType } from '@codemirror/view'
import type { ViewUpdate } from '@codemirror/view'

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
const title = Decoration.mark({ class: 'cm-md-title' })
const heading = Decoration.mark({ class: 'cm-md-heading' })
const link = Decoration.mark({ class: 'cm-md-link' })
const inlineStyles = new Map([
  ['StrongEmphasis', Decoration.mark({ class: 'cm-md-strong' })],
  ['Emphasis', Decoration.mark({ class: 'cm-md-emphasis' })],
  ['Strikethrough', Decoration.mark({ class: 'cm-md-strikethrough' })],
  ['InlineCode', Decoration.mark({ class: 'cm-md-code' })],
  ['CodeText', Decoration.mark({ class: 'cm-md-code' })],
])
const invisibleMarkers = new Set(['EmphasisMark', 'StrikethroughMark', 'CodeMark', 'CodeInfo'])

function preview(view: EditorView) {
  const { state, viewport } = view
  const { doc, selection } = state
  const ranges: Range<Decoration>[] = []

  function decorate(from: number, to: number, decoration: Decoration) {
    const end = Math.min(to, viewport.to)
    for (let pos = Math.max(from, viewport.from); pos < end;) {
      const line = doc.lineAt(pos)
      const lineEnd = Math.min(end, line.to)
      const editing = selection.ranges.some((range) => range.from <= line.to && range.to >= line.from)
      if (!editing && pos < lineEnd) ranges.push(decoration.range(pos, lineEnd))
      pos = line.to + 1
    }
  }

  function skipSpace(from: number) {
    const line = doc.lineAt(from)
    const rest = line.text.slice(from - line.from)
    return from + rest.length - rest.trimStart().length
  }

  syntaxTree(state).iterate({
    from: viewport.from,
    to: viewport.to,
    enter(node) {
      const { name, from, to } = node
      const style = inlineStyles.get(name)
      if (style) decorate(from, to, style)
      if (invisibleMarkers.has(name)) decorate(from, to, hidden)
      if (/^(?:ATX|Setext)Heading/.test(name)) decorate(from, to, name.endsWith('1') ? title : heading)

      switch (name) {
        case 'HeaderMark':
          decorate(from, from === node.node.parent?.from ? skipSpace(to) : to, hidden)
          break
        case 'QuoteMark':
          decorate(from, to, quote)
          break
        case 'ListMark':
          if (node.node.parent?.parent?.name === 'BulletList') {
            if (node.node.nextSibling?.name === 'Task') decorate(from, skipSpace(to), hidden)
            else decorate(from, to, bullet)
          }
          break
        case 'TaskMarker':
          decorate(from, to, doc.sliceString(from, to) === '[ ]' ? unchecked : checked)
          break
        case 'Link':
        case 'Autolink': {
          const markers = node.node.getChildren('LinkMark')
          // Lezer also parses undefined references as Link nodes.
          if (name === 'Link' && markers.length < 3) break
          const [opening, closing] = markers
          decorate(opening.from, opening.to, hidden)
          decorate(opening.to, closing.from, link)
          decorate(closing.from, to, hidden)
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
  return Decoration.set(ranges, true)
}

export const captureMarkdown = ViewPlugin.fromClass(class {
  decorations

  constructor(view: EditorView) { this.decorations = preview(view) }

  update(update: ViewUpdate) {
    if (update.docChanged || update.selectionSet || update.viewportChanged
      || syntaxTree(update.state) !== syntaxTree(update.startState)) {
      this.decorations = preview(update.view)
    }
  }
}, { decorations: (plugin) => plugin.decorations })
