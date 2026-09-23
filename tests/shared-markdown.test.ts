import { describe, it, expect } from 'vitest'
import { markdownToHtml, escapeHtml } from '../packages/shared/src/markdown'

describe('markdownToHtml', () => {
  it('renders nothing for blank input', () => {
    expect(markdownToHtml('  \n ')).toBe('')
  })

  it('renders headings, rules and paragraphs', () => {
    expect(markdownToHtml('# Title\n\n## Part\n\n---\n\nPlain text.')).toBe(
      '<h1>Title</h1><h2>Part</h2><hr><p>Plain text.</p>'
    )
  })

  it('escapes code fences and their language', () => {
    expect(markdownToHtml('```ts"x\nconst a = "<b>"\n```')).toBe(
      '<pre><code class="language-ts&quot;x">const a = &quot;&lt;b&gt;&quot;</code></pre>'
    )
    expect(markdownToHtml('```\nplain\n```')).toBe('<pre><code>plain</code></pre>')
  })

  it('joins blockquote lines', () => {
    expect(markdownToHtml('> one\n> *two*')).toBe(
      '<blockquote><p>one<br><em>two</em></p></blockquote>'
    )
  })

  it('renders task, bullet and numbered lists', () => {
    const tasks = markdownToHtml('- [x] done\n- [ ] open')
    expect(tasks).toContain('<ul data-type="taskList">')
    expect(tasks).toContain('data-checked="true"><label><input type="checkbox" checked>')
    expect(tasks).toContain('data-checked="false"')
    expect(markdownToHtml('- a\n- b')).toBe('<ul><li><p>a</p></li><li><p>b</p></li></ul>')
    expect(markdownToHtml('1. a\n2. b')).toBe('<ol><li><p>a</p></li><li><p>b</p></li></ol>')
  })

  it('applies inline marks after escaping', () => {
    expect(markdownToHtml('**b** *i* ~~s~~ `c` <script>')).toBe(
      '<p><strong>b</strong> <em>i</em> <s>s</s> <code>c</code> &lt;script&gt;</p>'
    )
    expect(markdownToHtml("[mail](mailto:a@b.c) it's")).toBe(
      '<p><a href="mailto:a@b.c">mail</a> it&#39;s</p>'
    )
  })

  it('escapes every special character', () => {
    expect(escapeHtml(`&<>"'`)).toBe('&amp;&lt;&gt;&quot;&#39;')
  })
})
