import { describe, it, expect } from 'vitest'
import { mergeDocEdit, paragraphs } from '../src/renderer/lib/doc-edits'

const DOC = [
  '# Fine-tuning a small model',
  'That works, but it always bothered me a little: cost, privacy and accuracy.',
  'So in this experiment I wanted to see how far a small model can go.',
  '* one\n* two'
].join('\n\n')

describe('mergeDocEdit', () => {
  it('finds no edits when only the Markdown spelling changed', () => {
    const rewritten = DOC.replace('* one\n* two', '- one\n- two')
    const { body, edits } = mergeDocEdit(DOC, rewritten)
    expect(edits).toEqual([])
    expect(body).toContain('* one')
  })

  it('reports a changed paragraph as before and after, and keeps the rest as the source wrote it', () => {
    const edited = DOC.replace(
      'it always bothered me a little',
      'a few things kept me wondering whether it really scales'
    ).replace('* one\n* two', '- one\n- two')
    const { body, edits } = mergeDocEdit(DOC, edited)
    expect(edits).toEqual([
      {
        before: 'That works, but it always bothered me a little: cost, privacy and accuracy.',
        after:
          'That works, but a few things kept me wondering whether it really scales: cost, privacy and accuracy.'
      }
    ])
    expect(paragraphs(body)).toHaveLength(4)
    expect(body).toContain('* one')
    expect(body).toContain('really scales')
  })

  it('reports an added and a removed paragraph with the other side empty', () => {
    const added = mergeDocEdit(DOC, DOC + '\n\nA new closing line.')
    expect(added.edits).toEqual([{ before: '', after: 'A new closing line.' }])
    const removed = mergeDocEdit(DOC, DOC.replace(/\n\nSo in this[^\n]+/, ''))
    expect(removed.edits).toEqual([
      { before: 'So in this experiment I wanted to see how far a small model can go.', after: '' }
    ])
    expect(paragraphs(removed.body)).toHaveLength(3)
  })
})
