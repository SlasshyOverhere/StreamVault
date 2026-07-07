// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'

vi.mock('lucide-react', () => ({ Shield: () => <span /> }))

import { ReauthModal } from './ReauthModal'

beforeEach(() => {
  vi.resetAllMocks()
})

describe('ReauthModal', () => {
  it('does not render when open=false', () => {
    render(<ReauthModal open={false} onReauth={vi.fn()} onClose={vi.fn()} />)
    expect(screen.queryByText('Re-authenticate with Google')).toBeNull()
  })

  it('renders the action button when open=true', () => {
    render(<ReauthModal open onReauth={vi.fn()} onClose={vi.fn()} />)
    expect(screen.getByText('Re-authenticate with Google')).toBeTruthy()
  })

  it('clicking the action button calls onReauth', () => {
    const onReauth = vi.fn().mockResolvedValue(undefined)
    render(<ReauthModal open onReauth={onReauth} onClose={vi.fn()} />)
    fireEvent.click(screen.getByText('Re-authenticate with Google'))
    expect(onReauth).toHaveBeenCalled()
  })

  it('pressing Escape calls onClose', () => {
    const onClose = vi.fn()
    render(<ReauthModal open onReauth={vi.fn()} onClose={onClose} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalled()
  })
})
