// Runs inside the artifact's page over CDP; whitespace-collapsed quotes, best prefix/suffix match wins.
export const ANCHOR_LIB = `(function () {
  var SKIP = { SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1 }
  function norm(s) { return String(s || '').replace(/\\s+/g, ' ') }
  function index() {
    var root = document.body || document.documentElement
    var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode: function (n) {
        var p = n.parentElement
        return p && SKIP[p.tagName] ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT
      }
    })
    var parts = [], nodes = [], offs = [], space = true, n
    while ((n = walker.nextNode())) {
      var d = n.data
      for (var i = 0; i < d.length; i++) {
        var c = d.charAt(i)
        if (/\\s/.test(c)) {
          if (space) continue
          parts.push(' ')
          space = true
        } else {
          parts.push(c)
          space = false
        }
        nodes.push(n)
        offs.push(i)
      }
    }
    return { text: parts.join(''), nodes: nodes, offs: offs }
  }
  function tailMatch(a, b) {
    a = a.trimEnd(); b = b.trimEnd()
    var k = 0
    while (k < a.length && k < b.length && a.charAt(a.length - 1 - k) === b.charAt(b.length - 1 - k)) k++
    return k
  }
  function headMatch(a, b) {
    a = a.trimStart(); b = b.trimStart()
    var k = 0
    while (k < a.length && k < b.length && a.charAt(k) === b.charAt(k)) k++
    return k
  }
  function locate(idx, anchor) {
    var q = norm(anchor.quote).trim()
    if (!q) return null
    var best = -1, bestScore = -1, from = 0
    for (;;) {
      var at = idx.text.indexOf(q, from)
      if (at < 0) break
      var score =
        tailMatch(idx.text.slice(Math.max(0, at - 64), at), norm(anchor.prefix)) +
        headMatch(idx.text.slice(at + q.length, at + q.length + 64), norm(anchor.suffix))
      if (score > bestScore) { bestScore = score; best = at }
      from = at + 1
    }
    return best < 0 ? null : { start: best, end: best + q.length }
  }
  function rangeOf(idx, hit) {
    var r = document.createRange()
    r.setStart(idx.nodes[hit.start], idx.offs[hit.start])
    r.setEnd(idx.nodes[hit.end - 1], idx.offs[hit.end - 1] + 1)
    return r
  }
  function selection() {
    var sel = window.getSelection()
    if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return null
    var range = sel.getRangeAt(0)
    var quote = norm(range.toString()).trim()
    if (!quote) return null
    var root = document.body || document.documentElement
    var before = document.createRange()
    before.setStart(root, 0)
    before.setEnd(range.startContainer, range.startOffset)
    var after = document.createRange()
    after.setStart(range.endContainer, range.endOffset)
    after.setEnd(root, root.childNodes.length)
    var box = range.getBoundingClientRect ? range.getBoundingClientRect() : null
    return {
      anchor: {
        kind: 'quote',
        quote: quote.slice(0, 500),
        prefix: norm(before.toString()).slice(-32),
        suffix: norm(after.toString()).slice(0, 32)
      },
      rect: box
        ? { x: box.left, y: box.top, width: box.width, height: box.height }
        : { x: 0, y: 0, width: 0, height: 0 }
    }
  }
  var STATES = ['draft', 'sent', 'focus']
  function paint(marks) {
    var idx = index(), groups = { draft: [], sent: [], focus: [] }, found = {}
    for (var i = 0; i < marks.length; i++) {
      var m = marks[i], hit = locate(idx, m)
      found[m.id] = !!hit
      if (hit && groups[m.state]) groups[m.state].push(rangeOf(idx, hit))
    }
    if (window.CSS && CSS.highlights && typeof Highlight === 'function') {
      if (!window.__vornMarkSheet && typeof CSSStyleSheet === 'function') {
        var sheet = new CSSStyleSheet()
        sheet.replaceSync(
          '::highlight(vorn-draft){background-color:rgba(111,143,175,.24);text-decoration:underline 2px #6f8faf}' +
          '::highlight(vorn-sent){background-color:rgba(111,143,175,.14)}' +
          '::highlight(vorn-focus){background-color:rgba(111,143,175,.42)}'
        )
        document.adoptedStyleSheets = document.adoptedStyleSheets.concat([sheet])
        window.__vornMarkSheet = sheet
      }
      for (var k = 0; k < STATES.length; k++) {
        var name = 'vorn-' + STATES[k], ranges = groups[STATES[k]]
        if (ranges.length) CSS.highlights.set(name, new Highlight(...ranges))
        else CSS.highlights.delete(name)
      }
    }
    return found
  }
  function reveal(anchor) {
    var idx = index(), hit = locate(idx, anchor)
    if (!hit) return false
    var el = idx.nodes[hit.start].parentElement
    if (el && el.scrollIntoView) el.scrollIntoView({ block: 'center', behavior: 'smooth' })
    return true
  }
  function clear() {
    var sel = window.getSelection()
    if (sel) sel.removeAllRanges()
  }
  return { norm: norm, index: index, locate: locate, selection: selection, paint: paint, reveal: reveal, clear: clear }
})()`

/** An expression calling one helper with JSON arguments, so a quote stays data and never becomes source. */
export function anchorCall(
  fn: 'selection' | 'paint' | 'reveal' | 'clear',
  ...args: unknown[]
): string {
  return `${ANCHOR_LIB}.${fn}(${args.map((a) => JSON.stringify(a ?? null)).join(', ')})`
}
