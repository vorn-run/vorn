// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import {
  PropertyRow,
  PropertySection
} from '../packages/web/src/renderer/components/workflow-editor/panels/PropertyRow'

describe('PropertyRow', () => {
  it('renders label and children', () => {
    render(
      <PropertyRow label="Status">
        <span>Enabled</span>
      </PropertyRow>
    )
    expect(screen.getByText('Status')).toBeInTheDocument()
    expect(screen.getByText('Enabled')).toBeInTheDocument()
  })

  it('renders an action node when provided', () => {
    render(
      <PropertyRow label="Project" action={<button>Edit</button>}>
        <span>vorn</span>
      </PropertyRow>
    )
    expect(screen.getByRole('button', { name: 'Edit' })).toBeInTheDocument()
  })

  it('does not render an action wrapper when omitted', () => {
    const { container } = render(
      <PropertyRow label="Project">
        <span>vorn</span>
      </PropertyRow>
    )
    expect(container.querySelectorAll('div').length).toBe(3)
  })
})

describe('PropertySection', () => {
  it('renders section label and children', () => {
    render(
      <PropertySection label="Advanced">
        <div>row</div>
      </PropertySection>
    )
    expect(screen.getByText('Advanced')).toBeInTheDocument()
    expect(screen.getByText('row')).toBeInTheDocument()
  })

  it('omits the label header when no label is given', () => {
    render(
      <PropertySection>
        <div>row</div>
      </PropertySection>
    )
    expect(screen.getByText('row')).toBeInTheDocument()
  })
})
