// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { HttpRequestConfig } from '../src/shared/types'
import { HttpRequestNode } from '../src/renderer/components/workflow-editor/nodes/HttpRequestNode'
import { stepPreview } from '../src/renderer/components/workflow-editor/node-visuals'

const config = (over: Partial<HttpRequestConfig> = {}): HttpRequestConfig => ({
  nodeType: 'httpRequest',
  method: 'POST',
  url: 'https://x.test/a',
  headers: {},
  body: '',
  ...over
})

describe('HttpRequestNode', () => {
  it('shows method and URL in the subtitle', () => {
    const { container } = render(
      <HttpRequestNode label="Call API" config={config()} onClick={vi.fn()} />
    )
    expect(container.textContent).toContain('POST https://x.test/a')
  })

  it('renders the body footer exactly when the height estimate charges for one', () => {
    const withBody = config({ body: '{"n": 1}' })
    const { container } = render(
      <HttpRequestNode label="Call API" config={withBody} onClick={vi.fn()} />
    )
    expect(container.textContent).toContain('{"n": 1}')
    // The layout's preview check and the card's footer must agree, or the
    // declared height drifts from the drawn card and edges detach.
    const node = {
      id: 'h',
      type: 'httpRequest',
      label: 'x',
      config: withBody,
      position: { x: 0, y: 0 }
    }
    expect(stepPreview(node as never)).toBeTruthy()

    const { container: bare } = render(
      <HttpRequestNode label="Call API" config={config()} onClick={vi.fn()} />
    )
    expect(bare.textContent).not.toContain('{')
    const bareNode = { ...node, config: config() }
    expect(stepPreview(bareNode as never)).toBeUndefined()
  })
})
