// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ConnectorIcon } from '../src/renderer/components/ConnectorIcon'
import { connectionIcon } from '../src/renderer/lib/connection-icon'

const conn = (filters: Record<string, unknown>) => ({ filters }) as never

describe('connectionIcon', () => {
  it('reads the glyph a packaged connector stored on its connection', () => {
    expect(
      connectionIcon(
        conn({ sdkIcon: JSON.stringify({ viewBox: '0 0 16 16', paths: ['M1 1h4z'] }) })
      )
    ).toEqual({ viewBox: '0 0 16 16', paths: ['M1 1h4z'] })
  })

  it('falls back to a 24-unit viewBox when none was stored', () => {
    expect(connectionIcon(conn({ sdkIcon: JSON.stringify({ paths: ['M1 1h4z'] }) }))?.viewBox).toBe(
      '0 0 24 24'
    )
  })

  it.each([
    ['a connection with no icon', {}],
    ['an empty string', { sdkIcon: '' }],
    ['a non-string value', { sdkIcon: { paths: ['M1 1h4z'] } }],
    ['unparseable JSON', { sdkIcon: '{' }],
    ['JSON that is not an object', { sdkIcon: '"M1 1h4z"' }],
    ['an empty paths array', { sdkIcon: JSON.stringify({ paths: [] }) }],
    ['a non-string path', { sdkIcon: JSON.stringify({ paths: [42] }) }],
    ['an empty path', { sdkIcon: JSON.stringify({ paths: [''] }) }]
  ])('returns nothing for %s, so the built-in icon is used', (_label, filters) => {
    expect(connectionIcon(conn(filters))).toBeUndefined()
  })

  it('tolerates a missing connection, which happens before the cache warms up', () => {
    expect(connectionIcon(null)).toBeUndefined()
    expect(connectionIcon(undefined)).toBeUndefined()
  })
})

describe('ConnectorIcon', () => {
  it('draws a packaged connector glyph as paths inside an svg it owns', () => {
    const { container } = render(
      <ConnectorIcon
        connectorId="mcp"
        icon={{ viewBox: '0 0 16 16', paths: ['M1 1h4z', 'M8 8h2'] }}
      />
    )

    const svg = container.querySelector('svg')!
    expect(svg).toHaveAttribute('viewBox', '0 0 16 16')
    expect(svg.getAttribute('fill')).toBe('currentColor')
    const paths = [...container.querySelectorAll('path')].map((p) => p.getAttribute('d'))
    expect(paths).toEqual(['M1 1h4z', 'M8 8h2'])
  })

  it('never injects markup, so a hostile path stays inert text', () => {
    const { container } = render(
      <ConnectorIcon
        connectorId="mcp"
        icon={{ viewBox: '0 0 24 24', paths: ['"><script>x</script>'] }}
      />
    )

    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('path')).toHaveAttribute('d', '"><script>x</script>')
  })

  it('prefers a packaged glyph over the built-in one for the same id', () => {
    const builtin = render(<ConnectorIcon connectorId="github" />)
    const custom = render(
      <ConnectorIcon connectorId="github" icon={{ viewBox: '0 0 24 24', paths: ['M1 1h4z'] }} />
    )

    expect(custom.container.querySelector('path')).toHaveAttribute('d', 'M1 1h4z')
    expect(builtin.container.innerHTML).not.toBe(custom.container.innerHTML)
  })

  it('falls back to the built-in icon when no glyph is supplied', () => {
    const { container } = render(<ConnectorIcon connectorId="github" />)
    expect(container.querySelector('svg')).toHaveAttribute('viewBox', '0 0 16 16')
  })

  it('applies the requested size and class to a packaged glyph', () => {
    const { container } = render(
      <ConnectorIcon
        connectorId="mcp"
        icon={{ viewBox: '0 0 24 24', paths: ['M1 1h4z'] }}
        size={20}
        className="text-red-400"
      />
    )

    const svg = container.querySelector('svg')!
    expect(svg).toHaveAttribute('width', '20')
    expect(svg).toHaveClass('text-red-400')
  })
})
