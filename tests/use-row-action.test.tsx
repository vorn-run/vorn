// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { useRowAction, rowState } from '../src/renderer/lib/use-row-action'

function Probe({ work }: { work: () => Promise<void | { ok: boolean; error?: string }> }) {
  const activity = useRowAction()
  const state = activity.state('slack', ['remove'])
  return (
    <div>
      <button onClick={() => void activity.run('remove', 'slack', work)}>go</button>
      <span data-testid="busy">{state.phrase ?? 'idle'}</span>
      <span data-testid="failed">{state.error ?? 'none'}</span>
    </div>
  )
}

describe('a row action reporting itself', () => {
  it('says what it is doing until the call answers', async () => {
    let finish = (): void => {}
    const work = vi.fn(
      () =>
        new Promise<{ ok: true }>((resolve) => {
          finish = () => resolve({ ok: true })
        })
    )
    render(<Probe work={work} />)

    fireEvent.click(screen.getByText('go'))
    await waitFor(() => expect(screen.getByTestId('busy')).toHaveTextContent('Removing…'))

    finish()
    await waitFor(() => expect(screen.getByTestId('busy')).toHaveTextContent('idle'))
    expect(screen.getByTestId('failed')).toHaveTextContent('none')
  })

  it('keeps a refusal the call answered with, on the row it was pressed', async () => {
    render(<Probe work={async () => ({ ok: false, error: 'The files are in use' })} />)

    fireEvent.click(screen.getByText('go'))

    await waitFor(() =>
      expect(screen.getByTestId('failed')).toHaveTextContent('The files are in use')
    )
    expect(screen.getByTestId('busy')).toHaveTextContent('idle')
  })

  it('names a refusal that arrived without a reason', async () => {
    render(<Probe work={async () => ({ ok: false })} />)

    fireEvent.click(screen.getByText('go'))

    await waitFor(() =>
      expect(screen.getByTestId('failed')).toHaveTextContent('It did not say why.')
    )
  })

  it('keeps a thrown failure, and stops saying it is working', async () => {
    render(
      <Probe
        work={() => {
          throw new Error('The server went away')
        }}
      />
    )

    fireEvent.click(screen.getByText('go'))

    await waitFor(() =>
      expect(screen.getByTestId('failed')).toHaveTextContent('The server went away')
    )
    expect(screen.getByTestId('busy')).toHaveTextContent('idle')
  })

  it('forgets the last failure when the same action runs again', async () => {
    let fail = true
    render(<Probe work={async () => (fail ? { ok: false, error: 'Nope' } : { ok: true })} />)

    fireEvent.click(screen.getByText('go'))
    await waitFor(() => expect(screen.getByTestId('failed')).toHaveTextContent('Nope'))

    fail = false
    fireEvent.click(screen.getByText('go'))
    await waitFor(() => expect(screen.getByTestId('failed')).toHaveTextContent('none'))
  })

  it('answers a call that reports nothing as a success', async () => {
    render(<Probe work={async () => undefined} />)

    fireEvent.click(screen.getByText('go'))

    await waitFor(() => expect(screen.getByTestId('busy')).toHaveTextContent('idle'))
    expect(screen.getByTestId('failed')).toHaveTextContent('none')
  })
})

describe('what a row shows for the actions it owns', () => {
  it('says the phrase of whichever action is working', () => {
    expect(rowState({ 'delete:c1': 'Removing…' }, {}, 'c1', ['backfill', 'delete'])).toEqual({
      phrase: 'Removing…'
    })
  })

  it('prefers a working action over a stale failure', () => {
    const state = rowState({ 'backfill:c1': 'Importing…' }, { 'delete:c1': 'It refused' }, 'c1', [
      'backfill',
      'delete'
    ])

    expect(state).toEqual({ phrase: 'Importing…' })
  })

  it('reads only the actions it was asked about, and only its own row', () => {
    const busy = { 'run:c2': 'Polling…', 'backfill:c1': 'Importing…' }

    expect(rowState(busy, {}, 'c1', ['delete'])).toEqual({})
    expect(rowState(busy, {}, 'c2', ['run'])).toEqual({ phrase: 'Polling…' })
  })
})
