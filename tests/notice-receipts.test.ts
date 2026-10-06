import { describe, it, expect } from 'vitest'
import { NoticeReceipts, NOTICE_RECEIPT_MS } from '../src/main/notice-receipts'

describe('NoticeReceipts', () => {
  it('shows a notification once by its effect id, and one without an id every time', () => {
    const notices = new NoticeReceipts()
    const payload = { id: 'pane', title: 'Build', body: 'done', effectId: 'pane/3/41/0' }
    expect(notices.shows(payload)).toBe(true)
    expect(notices.shows({ ...payload })).toBe(false)
    expect(notices.shows({ ...payload, effectId: 'pane/3/42/0' })).toBe(true)
    const plain = { id: 'pane', title: 'Build', body: 'done' }
    expect(notices.shows(plain)).toBe(true)
    expect(notices.shows(plain)).toBe(true)
    expect(notices.shows(null)).toBe(true)
  })

  it('forgets an id after a day', () => {
    let now = 1_000
    const notices = new NoticeReceipts(10, NOTICE_RECEIPT_MS, () => now)
    expect(notices.first('a')).toBe(true)
    now += NOTICE_RECEIPT_MS - 1
    expect(notices.first('a')).toBe(false)
    // Counted from when it was shown, not from when it was heard again.
    now += 1
    expect(notices.first('a')).toBe(true)
  })

  it('keeps a bounded number, dropping the one heard least recently', () => {
    const notices = new NoticeReceipts(2)
    notices.first('a')
    notices.first('b')
    // `a` heard again: `b` is now the one to drop.
    expect(notices.first('a')).toBe(false)
    notices.first('c')
    expect(notices.first('a')).toBe(false)
    expect(notices.first('b')).toBe(true)
  })
})
