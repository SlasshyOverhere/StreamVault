// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'

vi.mock('@/hooks/useAuth', () => ({ useAuth: vi.fn() }))
vi.mock('lucide-react', () => ({
  AlertTriangle: () => <span />,
  X: () => <span />,
  Shield: () => <span />,
}))

import { useAuth } from '@/hooks/useAuth'
import { ReauthBanner } from './ReauthBanner'

beforeEach(() => {
  vi.resetAllMocks()
  sessionStorage.clear()
})

describe('ReauthBanner', () => {
  it('renders when needsReauth is true', () => {
    vi.mocked(useAuth).mockReturnValue({ needsReauth: true, reauth: vi.fn() } as any)
    render(<ReauthBanner />)
    expect(screen.getByText(/Drive session expired/i)).toBeTruthy()
  })

  it('does not render when needsReauth is false', () => {
    vi.mocked(useAuth).mockReturnValue({ needsReauth: false, reauth: vi.fn() } as any)
    const { container } = render(<ReauthBanner />)
    expect(container.firstChild).toBeNull()
  })

  it('opens the modal when "Re-authenticate" is clicked', () => {
    vi.mocked(useAuth).mockReturnValue({ needsReauth: true, reauth: vi.fn() } as any)
    render(<ReauthBanner />)
    fireEvent.click(screen.getByText('Re-authenticate'))
    // The banner's button should toggle the modal's open state. Modal's primary action
    // appears only when open — verify by checking that the modal text is now in the document.
    expect(screen.getByText('Re-authenticate with Google')).toBeTruthy()
  })

  it('Dismiss button hides the banner via sessionStorage', () => {
    vi.mocked(useAuth).mockReturnValue({ needsReauth: true, reauth: vi.fn() } as any)
    render(<ReauthBanner />)
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(sessionStorage.getItem('slasshyvault.reauth-banner-dismissed')).toBe('1')
    expect(screen.queryByText(/Drive session expired/i)).toBeNull()
  })
})
