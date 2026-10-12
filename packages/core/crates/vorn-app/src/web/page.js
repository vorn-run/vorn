// Seated in every pane page: reports console and requests, and answers the agent's reads and actions.
/* global CSS, Event, Highlight, InputEvent, KeyboardEvent, MouseEvent, NodeFilter, Range, XMLHttpRequest, console, document, getComputedStyle, history, location, window */
;(() => {
  if (window.__vorn) return
  const post = (m) => {
    try {
      window.ipc.postMessage(JSON.stringify(m))
    } catch {
      // The host is gone while the page unloads.
    }
  }
  const show = (v) => {
    if (typeof v === 'string') return v
    try {
      return JSON.stringify(v)
    } catch {
      return String(v)
    }
  }
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const orig = console[level]
    console[level] = function (...args) {
      post({
        t: 'console',
        level: level === 'warn' ? 'warning' : level,
        text: args.map(show).join(' ').slice(0, 2000),
        timestamp: Date.now()
      })
      return orig.apply(this, args)
    }
  }
  window.addEventListener('error', (e) =>
    post({ t: 'console', level: 'error', text: String(e.message), timestamp: Date.now() })
  )
  const net = (method, url, status) =>
    post({
      t: 'network',
      method: String(method || 'GET').toUpperCase(),
      url: String(url),
      status,
      timestamp: Date.now()
    })
  const origFetch = window.fetch
  if (origFetch) {
    window.fetch = function (input, init) {
      const url = typeof input === 'string' ? input : input && input.url
      const method = (init && init.method) || (input && input.method) || 'GET'
      return origFetch.apply(this, arguments).then(
        (r) => (net(method, url, r.status), r),
        (e) => (net(method, url, undefined), Promise.reject(e))
      )
    }
  }
  const open = XMLHttpRequest.prototype.open
  XMLHttpRequest.prototype.open = function (method, url) {
    this.addEventListener('loadend', () => net(method, url, this.status || undefined))
    return open.apply(this, arguments)
  }
  document.addEventListener('selectionchange', () => {
    const s = document.getSelection()
    const text = s ? String(s).trim() : ''
    if (!text || !s.rangeCount) return post({ t: 'selection', quote: '' })
    const r = s.getRangeAt(0).getBoundingClientRect()
    const all = document.body ? document.body.innerText : ''
    const at = all.indexOf(text)
    post({
      t: 'selection',
      quote: text.slice(0, 1000),
      prefix: at > 0 ? all.slice(Math.max(0, at - 32), at) : '',
      suffix: at >= 0 ? all.slice(at + text.length, at + text.length + 32) : '',
      rect: { x: r.x, y: r.y, width: r.width, height: r.height }
    })
  })

  const INTERACTIVE = new Set([
    'button',
    'link',
    'textbox',
    'searchbox',
    'combobox',
    'checkbox',
    'radio',
    'switch',
    'slider',
    'menuitem',
    'menuitemcheckbox',
    'menuitemradio',
    'option',
    'tab',
    'spinbutton'
  ])
  const INPUT_ROLES = {
    button: 'button',
    submit: 'button',
    reset: 'button',
    image: 'button',
    checkbox: 'checkbox',
    radio: 'radio',
    range: 'slider',
    number: 'spinbutton',
    search: 'searchbox',
    email: 'textbox',
    tel: 'textbox',
    url: 'textbox',
    text: 'textbox',
    password: 'textbox'
  }
  const TAG_ROLES = {
    button: 'button',
    select: 'combobox',
    textarea: 'textbox',
    h1: 'heading',
    h2: 'heading',
    h3: 'heading',
    h4: 'heading',
    h5: 'heading',
    h6: 'heading',
    p: 'paragraph',
    li: 'listitem',
    nav: 'navigation',
    main: 'main',
    img: 'img',
    option: 'option',
    summary: 'button',
    label: 'LabelText'
  }
  const roleOf = (el) => {
    const explicit = el.getAttribute('role')
    if (explicit) return explicit.split(' ')[0]
    const tag = el.tagName.toLowerCase()
    if (tag === 'a') return el.hasAttribute('href') ? 'link' : null
    if (tag === 'input') {
      const type = (el.getAttribute('type') || 'text').toLowerCase()
      return type === 'hidden' ? null : INPUT_ROLES[type] || 'textbox'
    }
    if (tag === 'img') return el.getAttribute('alt') === '' ? null : 'img'
    return TAG_ROLES[tag] || null
  }
  const nameOf = (el) => {
    const label = el.getAttribute('aria-label')
    if (label) return label.trim()
    const by = el.getAttribute('aria-labelledby')
    if (by) {
      const t = by
        .split(/\s+/)
        .map((id) => document.getElementById(id))
        .filter(Boolean)
        .map((n) => n.innerText)
        .join(' ')
        .trim()
      if (t) return t
    }
    if (el.labels && el.labels.length)
      return Array.from(el.labels)
        .map((l) => l.innerText)
        .join(' ')
        .trim()
    const alt = el.getAttribute('alt') || el.getAttribute('title') || el.getAttribute('placeholder')
    if (alt) return alt.trim()
    const tag = el.tagName.toLowerCase()
    if (tag === 'input' || tag === 'select' || tag === 'textarea') return ''
    return (el.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 200)
  }
  const visible = (el) => {
    if (el.closest('[aria-hidden="true"],[hidden]')) return false
    const s = getComputedStyle(el)
    return s.display !== 'none' && s.visibility !== 'hidden'
  }
  let generation = 0
  const refs = new Map()
  const reset = (gen) => {
    if (gen !== generation) {
      generation = gen
      refs.clear()
    }
  }
  const nodeFor = (el) => {
    const role = roleOf(el)
    if (!role || !visible(el)) return null
    const name = nameOf(el)
    const interactive = INTERACTIVE.has(role)
    if (!interactive && !name) return null
    const node = { role }
    if (name) node.name = name
    if ('value' in el && typeof el.value === 'string' && el.value && role !== 'button')
      node.value = el.value
    if (el.disabled === true || el.getAttribute('aria-disabled') === 'true') node.disabled = true
    if (interactive) {
      node.ref = 'g' + generation + '_ref_' + (refs.size + 1)
      refs.set(node.ref, el)
    }
    return node
  }
  const elements = () => (document.body ? Array.from(document.body.querySelectorAll('*')) : [])
  const target = (t) => {
    if (!t) return null
    if (typeof t.x === 'number') return { el: document.elementFromPoint(t.x, t.y), x: t.x, y: t.y }
    const el = refs.get(t.ref)
    if (!el || !el.isConnected)
      throw new Error(
        'Ref "' +
          t.ref +
          '" is stale — the page changed since it was read. Call read_page again for fresh refs.'
      )
    el.scrollIntoView({ block: 'center', inline: 'center' })
    const r = el.getBoundingClientRect()
    if (!r.width && !r.height) throw new Error('Ref "' + t.ref + '" is not visible on the page.')
    return { el, x: r.x + r.width / 2, y: r.y + r.height / 2 }
  }
  const mouse = (el, type, x, y) =>
    el.dispatchEvent(
      new MouseEvent(type, {
        bubbles: true,
        cancelable: true,
        view: window,
        clientX: x,
        clientY: y,
        button: 0
      })
    )
  const click = (p) => {
    if (!p.el) return
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup'])
      mouse(p.el, type, p.x, p.y)
    if (p.el.focus) p.el.focus()
    p.el.click()
  }
  const wrap = (f) => {
    try {
      return { ok: f() }
    } catch (e) {
      return { error: String((e && e.message) || e) }
    }
  }
  const marks = new Map()
  window.__vorn = {
    read: (gen, all, start, budget) =>
      wrap(() => {
        reset(gen)
        const els = elements()
        const nodes = []
        let i = start
        for (; i < els.length && nodes.length < budget; i++) {
          const n = nodeFor(els[i])
          if (!n || (!all && !n.ref)) continue
          nodes.push(n)
        }
        return { url: location.href, title: document.title, nodes, next: i < els.length ? i : null }
      }),
    find: (gen, needle, limit) =>
      wrap(() => {
        reset(gen)
        const q = String(needle).toLowerCase()
        const out = []
        for (const el of elements()) {
          if (out.length >= limit) break
          const role = roleOf(el)
          if (!role || !nameOf(el).toLowerCase().includes(q)) continue
          const n = nodeFor(el)
          if (n) out.push(n)
        }
        return out
      }),
    text: () =>
      wrap(() => ({ url: location.href, text: document.body ? document.body.innerText : '' })),
    act: (p) =>
      wrap(() => {
        if (p.action === 'type') {
          const at = target(p.target)
          if (at) click(at)
          const el = document.activeElement
          if (el && 'value' in el) {
            el.value += p.text || ''
            el.dispatchEvent(new InputEvent('input', { bubbles: true, data: p.text || '' }))
            el.dispatchEvent(new Event('change', { bubbles: true }))
          }
          return { ok: true }
        }
        if (p.action === 'key') {
          const el = document.activeElement || document.body
          for (const type of ['keydown', 'keyup'])
            el.dispatchEvent(
              new KeyboardEvent(type, { key: p.text || '', bubbles: true, cancelable: true })
            )
          if (p.text === 'Enter' && el.form) {
            if (el.form.requestSubmit) el.form.requestSubmit()
            else el.form.submit()
          }
          return { ok: true }
        }
        if (p.action === 'scroll') {
          const at = p.target ? target(p.target) : null
          const box =
            at && at.el && at.el !== document.body
              ? at.el
              : document.scrollingElement || document.body
          box.scrollBy(0, typeof p.deltaY === 'number' ? p.deltaY : 400)
          return { ok: true }
        }
        if (!p.target) throw new Error('"' + p.action + '" needs a target (a ref or x/y).')
        const at = target(p.target)
        if (p.action === 'hover') {
          if (at.el)
            for (const type of ['pointerover', 'mouseover', 'mouseenter', 'mousemove'])
              mouse(at.el, type, at.x, at.y)
          return { ok: true }
        }
        click(at)
        return { ok: true }
      }),
    manifest: () =>
      wrap(() => {
        const el = document.getElementById('artifact')
        window.__artifactHost = 'vorn'
        let live = null
        const t = window.__artifact && window.__artifact.tweaks
        if (t && typeof t === 'object') {
          live = {}
          let kept = 0
          for (const k in t) {
            if (kept >= 24) break
            if (!Object.prototype.hasOwnProperty.call(t, k)) continue
            const v = t[k]
            if (
              (typeof v === 'number' && isFinite(v)) ||
              typeof v === 'boolean' ||
              (typeof v === 'string' && v.length <= 200)
            ) {
              live[k] = v
              kept++
            }
          }
        }
        return { declared: el && el.textContent ? el.textContent : '', live }
      }),
    tweak: (values) =>
      wrap(() => {
        if (!window.__artifact) return false
        window.__artifact.tweaks = Object.assign(window.__artifact.tweaks || {}, values)
        if (typeof window.__artifactRender === 'function') window.__artifactRender()
        return true
      }),
    marks: (list) =>
      wrap(() => {
        if (!window.CSS || !CSS.highlights) return 0
        for (const name of ['vorn-draft', 'vorn-sent', 'vorn-focus']) CSS.highlights.delete(name)
        marks.clear()
        const groups = { draft: [], sent: [], focus: [] }
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
        const texts = []
        for (let n = walker.nextNode(); n; n = walker.nextNode()) texts.push(n)
        for (const m of list) {
          for (const n of texts) {
            const at = n.data.indexOf(m.quote)
            if (at < 0) continue
            const r = new Range()
            r.setStart(n, at)
            r.setEnd(n, at + m.quote.length)
            groups[m.state].push(r)
            marks.set(m.id, r)
            break
          }
        }
        const found = {}
        for (const m of list) found[m.id] = marks.has(m.id)
        if (!document.getElementById('vorn-marks')) {
          const style = document.createElement('style')
          style.id = 'vorn-marks'
          style.textContent =
            '::highlight(vorn-draft){background:rgba(96,165,250,.28)}::highlight(vorn-sent){background:rgba(148,163,184,.22)}::highlight(vorn-focus){background:rgba(96,165,250,.55)}'
          document.head.appendChild(style)
        }
        for (const k in groups)
          if (groups[k].length) CSS.highlights.set('vorn-' + k, new Highlight(...groups[k]))
        return found
      }),
    reveal: (id) =>
      wrap(() => {
        const r = marks.get(id)
        if (!r) return false
        const el = r.startContainer.parentElement
        if (el) el.scrollIntoView({ block: 'center' })
        return true
      }),
    history: () => wrap(() => history.length),
    // CommentPopover, drawn in the page so the page stays visible around it.
    popover: (quote, x, y, width) =>
      wrap(() => {
        layer().innerHTML = ''
        const box = document.createElement('div')
        box.setAttribute('role', 'dialog')
        box.setAttribute('aria-label', 'Comment on the selection')
        box.className = 'pop'
        box.style.cssText = 'left:' + x + 'px;top:' + y + 'px;width:' + width + 'px'
        box.innerHTML =
          '<q></q><textarea rows="3" aria-label="Comment" placeholder="What should change here?"></textarea><div class="row"><span>\u2318\u21b5 to add</span><button class="cancel" type="button">Cancel</button><button class="add" type="button" disabled>Add</button></div>'
        box.querySelector('q').textContent = quote
        const area = box.querySelector('textarea')
        const add = box.querySelector('.add')
        const send = () => area.value.trim() && post({ t: 'comment', body: area.value.trim() })
        area.addEventListener('input', () => (add.disabled = !area.value.trim()))
        area.addEventListener('keydown', (e) => {
          e.stopPropagation()
          if (e.key === 'Escape') post({ t: 'cancel' })
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) send()
        })
        box.querySelector('.cancel').addEventListener('click', () => post({ t: 'cancel' }))
        add.addEventListener('click', send)
        layer().appendChild(box)
        area.focus()
        return true
      }),
    // DesignCanvas's pins on one artboard, and its crosshair while pinning.
    pins: (pinning, list) =>
      wrap(() => {
        const root = layer()
        root.innerHTML = ''
        root.host.style.pointerEvents = pinning ? 'auto' : 'none'
        root.host.style.cursor = pinning ? 'crosshair' : ''
        for (const p of list) {
          const b = document.createElement('button')
          b.className = 'pin ' + p.state
          b.textContent = String(p.n)
          b.setAttribute('aria-label', 'Comment ' + p.n)
          b.style.cssText = 'left:' + p.x + 'px;top:' + (p.y - 20) + 'px'
          b.addEventListener('click', (e) => {
            e.stopPropagation()
            post({ t: 'pin', id: p.id })
          })
          root.appendChild(b)
        }
        return true
      }),
    clear: () => wrap(() => ((layer().innerHTML = ''), true))
  }
  let shadow = null
  const layer = () => {
    if (shadow && shadow.host.isConnected) return shadow
    const host = document.createElement('vorn-layer')
    host.style.cssText = 'position:fixed;inset:0;z-index:2147483647;pointer-events:none'
    shadow = host.attachShadow({ mode: 'closed' })
    const style = document.createElement('style')
    style.textContent =
      '.pop{position:absolute;pointer-events:auto;box-sizing:border-box;display:flex;flex-direction:column;gap:8px;padding:10px;border-radius:4px;border:1px solid rgba(255,255,255,.12);background:#1c1c20;color:#faf9f7;font:12px system-ui,sans-serif}' +
      'q{display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden;font:11.5px/1.375 ui-monospace,Menlo,monospace;color:rgba(255,255,255,.35);border-left:2px solid #6f8faf;padding-left:8px;quotes:none}' +
      'textarea{box-sizing:border-box;width:100%;resize:none;border-radius:4px;border:1px solid rgba(255,255,255,.12);background:#141416;padding:6px 8px;font:12px system-ui,sans-serif;color:#faf9f7;outline:none}textarea:focus{border-color:rgba(255,255,255,.24)}textarea::placeholder{color:rgba(255,255,255,.35)}' +
      '.row{display:flex;align-items:center;gap:6px}.row span{flex:1;font:10.5px ui-monospace,Menlo,monospace;color:rgba(255,255,255,.35)}' +
      'button{height:24px;border:0;border-radius:4px;font:12px system-ui,sans-serif;cursor:pointer}.cancel{padding:0 8px;background:transparent;color:rgba(255,255,255,.55)}.cancel:hover{background:rgba(255,255,255,.06)}' +
      '.add{padding:0 10px;background:#faf9f7;color:#0d0d0f;font-weight:600}.add:disabled{opacity:.4}' +
      '.pin{position:absolute;pointer-events:auto;width:20px;height:20px;padding:0;display:grid;place-items:center;border-radius:50% 50% 50% 0;font:bold 10px ui-monospace,Menlo,monospace;color:#0d0d0f;background:#6f8faf}.pin.sent{background:#7d8590}.pin.focus{box-shadow:0 0 0 2px #faf9f7}'
    shadow.appendChild(style)
    host.addEventListener('click', (e) => {
      if (e.target !== host) return
      const el = document.elementsFromPoint(e.clientX, e.clientY).find((n) => n !== host)
      post({
        t: 'point',
        x: Math.round(e.clientX),
        y: Math.round(e.clientY),
        element: el ? describe(el) : '',
        ...(el ? detail(el) : {})
      })
    })
    ;(document.body || document.documentElement).appendChild(host)
    return shadow
  }
  const path = (n) => {
    const parts = []
    for (; n && n.nodeType === 1 && parts.length < 8; n = n.parentElement) {
      let p = n.tagName.toLowerCase()
      if (n.id) {
        parts.unshift(p + '#' + n.id)
        break
      }
      if (n.classList.length) p += '.' + Array.from(n.classList).slice(0, 3).join('.')
      parts.unshift(p)
    }
    return parts.join(' > ')
  }
  // What the element picker hands the agent, as the desktop's pick reads it.
  const detail = (el) => {
    let componentName, source
    const key = Object.keys(el).find((k) => k.startsWith('__reactFiber$'))
    if (key) {
      let f = el[key]
      for (let i = 0; f && i < 10; i++, f = f.return) {
        if (typeof f.type === 'function') {
          componentName = f.type.displayName || f.type.name
          break
        }
        if (f._debugSource && !source)
          source = f._debugSource.fileName + ':' + f._debugSource.lineNumber
      }
    }
    return {
      url: location.href,
      selector: path(el),
      text: (el.innerText || el.value || el.getAttribute('aria-label') || '').slice(0, 500).trim(),
      html: el.outerHTML.slice(0, 2000),
      componentName,
      source
    }
  }
  const describe = (el) => {
    const role = roleOf(el)
    const name = nameOf(el).slice(0, 60)
    const tag = el.tagName.toLowerCase()
    return (role || tag) + (name ? ' "' + name + '"' : '')
  }
})()
