// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { useRowAction, rowKey } from '../src/renderer/lib/use-row-action'

const KEY = rowKey('remove', 'slack')

function Probe({ work }: { work: () => Promise<unknown> }) {
  const activity = useRowAction()
  return (
    <div>
      <button onClick={() => void activity.run(KEY, 'Removing…', work)}>go</button>
      <span data-testid="busy">{activity.busy[KEY] ?? 'idle'}</span>
      <span data-testid="failed">{activity.failed[KEY] ?? 'none'}</span>
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
