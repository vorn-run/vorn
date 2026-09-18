import { describe, expect, it } from 'vitest'
import { isRecordList, jsonErrorLocation, toItemList } from '../packages/shared/src/item-list'

describe('toItemList', () => {
  it('takes a list, its JSON text, or an object holding one', () => {
    expect(toItemList([1, 2])).toEqual({ items: [1, 2] })
    expect(toItemList('[{"a":1}]')).toEqual({ items: [{ a: 1 }] })
    expect(toItemList({ findings: [{ a: 1 }], total: 1 })).toEqual({
      items: [{ a: 1 }],
      wrapperKey: 'findings'
    })
    expect(toItemList('   ')).toEqual({ items: [] })
  })

  it('says why something is not a list', () => {
    expect(toItemList('{"a": [1], "b": [2]}')).toEqual({
      error: expect.stringMatching(/several \(a, b\)/)
    })
    expect(toItemList(42)).toEqual({ error: expect.stringMatching(/expected a JSON array/) })
    expect(toItemList('[{"a": 1,}]')).toEqual({
      error: expect.stringMatching(/line 1, column \d+/)
    })
  })
})

describe('isRecordList', () => {
  it('is true only for a non-empty list of plain objects', () => {
    expect(isRecordList([{ a: 1 }, { b: 2 }])).toBe(true)
    expect(isRecordList([])).toBe(false)
    expect(isRecordList([{ a: 1 }, [1]])).toBe(false)
    expect(isRecordList(['x'])).toBe(false)
  })
})

describe('jsonErrorLocation', () => {
  it('counts lines and columns the way an editor shows them', () => {
    const text = '[\n  {"a": 1},\n  {"b": 2,}\n]'
    let err: unknown
    try {
      JSON.parse(text)
    } catch (e) {
      err = e
    }
    const where = jsonErrorLocation(text, err)
    expect(where.line).toBe(3)
    expect(where.column).toBeGreaterThan(1)
  })
})
