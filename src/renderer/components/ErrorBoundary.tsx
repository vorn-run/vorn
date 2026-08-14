import React from 'react'

interface State {
  error: Error | null
}

export class ErrorBoundary extends React.Component<{ children: React.ReactNode }, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    console.error('[ErrorBoundary]', error, info.componentStack)
  }

  render(): React.ReactNode {
    if (!this.state.error) return this.props.children

    const error = this.state.error

    return (
      <div
        style={{
          background: 'var(--color-surface-overlay)',
          color: '#e5e5e5',
          height: '100vh',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: '2rem',
          fontFamily: 'system-ui, -apple-system, sans-serif'
        }}
      >
        <div style={{ maxWidth: 560 }}>
          <h1 style={{ fontSize: 18, fontWeight: 600, marginBottom: 8, color: '#f87171' }}>
            Something went wrong
          </h1>
          <p style={{ fontSize: 13, color: '#9ca3af', marginBottom: 16 }}>
            The app encountered an unexpected error. You can try reloading or check the details
            below.
          </p>
          <pre
            style={{
              background: '#111113',
              border: '1px solid rgba(255,255,255,0.06)',
              borderRadius: 8,
              padding: 16,
              fontSize: 12,
              fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
              color: '#d1d5db',
              overflow: 'auto',
              maxHeight: 300,
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              marginBottom: 16
            }}
          >
            {error.message}
            {error.stack && '\n\n' + error.stack}
          </pre>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              onClick={() => window.location.reload()}
              style={{
                background: 'rgba(255,255,255,0.08)',
                border: '1px solid rgba(255,255,255,0.1)',
                borderRadius: 6,
                color: '#e5e5e5',
                padding: '6px 16px',
                fontSize: 13,
                cursor: 'pointer'
              }}
            >
              Reload
            </button>
            <button
              onClick={() => {
                navigator.clipboard.writeText(
                  error.message + (error.stack ? '\n\n' + error.stack : '')
                )
              }}
              style={{
                background: 'transparent',
                border: '1px solid rgba(255,255,255,0.06)',
                borderRadius: 6,
                color: '#9ca3af',
                padding: '6px 16px',
                fontSize: 13,
                cursor: 'pointer'
              }}
            >
              Copy error
            </button>
          </div>
        </div>
      </div>
    )
  }
}
