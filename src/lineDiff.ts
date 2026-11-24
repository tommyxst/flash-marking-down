export type DiffLineRow = {
  kind: 'context' | 'add' | 'del'
  oldLine: number | null
  newLine: number | null
  text: string
}

export type DiffRow = DiffLineRow | { kind: 'hunk'; skipped: number }

export type LineDiffSummary = {
  added: number
  removed: number
  rows: DiffLineRow[]
  compactRows: DiffRow[]
}

function splitLines(text: string): string[] {
  if (!text) return []
  return text.replaceAll('\r\n', '\n').split('\n')
}

export function computeLineDiff(before: string, after: string, contextLines = 3): LineDiffSummary {
  const a = splitLines(before)
  const b = splitLines(after)
  let prefix = 0
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix += 1
  let suffix = 0
  while (
    suffix < a.length - prefix
    && suffix < b.length - prefix
    && a[a.length - 1 - suffix] === b[b.length - 1 - suffix]
  ) {
    suffix += 1
  }

  const rows: DiffLineRow[] = []
  for (let i = 0; i < prefix; i += 1) {
    rows.push({ kind: 'context', oldLine: i + 1, newLine: i + 1, text: a[i] })
  }

  const aMid = a.slice(prefix, a.length - suffix)
  const bMid = b.slice(prefix, b.length - suffix)
  let added = 0
  let removed = 0

  if (aMid.length * bMid.length <= 160_000) {
    const n = aMid.length
    const m = bMid.length
    const dp = new Uint32Array((n + 1) * (m + 1))
    const stride = m + 1
    for (let i = n - 1; i >= 0; i -= 1) {
      for (let j = m - 1; j >= 0; j -= 1) {
        dp[i * stride + j] = aMid[i] === bMid[j]
          ? dp[(i + 1) * stride + (j + 1)] + 1
          : Math.max(dp[(i + 1) * stride + j], dp[i * stride + (j + 1)])
      }
    }
    let i = 0
    let j = 0
    while (i < n || j < m) {
      if (i < n && j < m && aMid[i] === bMid[j]) {
        rows.push({ kind: 'context', oldLine: prefix + i + 1, newLine: prefix + j + 1, text: aMid[i] })
        i += 1
        j += 1
      } else if (i < n && (j === m || dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)])) {
        rows.push({ kind: 'del', oldLine: prefix + i + 1, newLine: null, text: aMid[i] })
        removed += 1
        i += 1
      } else {
        rows.push({ kind: 'add', oldLine: null, newLine: prefix + j + 1, text: bMid[j] })
        added += 1
        j += 1
      }
    }
  } else {
    for (let i = 0; i < aMid.length; i += 1) {
      rows.push({ kind: 'del', oldLine: prefix + i + 1, newLine: null, text: aMid[i] })
      removed += 1
    }
    for (let j = 0; j < bMid.length; j += 1) {
      rows.push({ kind: 'add', oldLine: null, newLine: prefix + j + 1, text: bMid[j] })
      added += 1
    }
  }

  for (let i = 0; i < suffix; i += 1) {
    const oldIdx = a.length - suffix + i
    const newIdx = b.length - suffix + i
    rows.push({ kind: 'context', oldLine: oldIdx + 1, newLine: newIdx + 1, text: a[oldIdx] })
  }

  const keep = new Uint8Array(rows.length)
  for (let i = 0; i < rows.length; i += 1) {
    if (rows[i].kind !== 'context') {
      const start = Math.max(0, i - contextLines)
      const end = Math.min(rows.length - 1, i + contextLines)
      for (let k = start; k <= end; k += 1) keep[k] = 1
    }
  }

  const compactRows: DiffRow[] = []
  let skipped = 0
  for (let i = 0; i < rows.length; i += 1) {
    if (keep[i]) {
      if (skipped > 0) {
        compactRows.push({ kind: 'hunk', skipped })
        skipped = 0
      }
      compactRows.push(rows[i])
    } else {
      skipped += 1
    }
  }
  if (skipped > 0) compactRows.push({ kind: 'hunk', skipped })

  return { added, removed, rows, compactRows }
}
