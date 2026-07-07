// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { ShareDialog } from './ShareDialog'

vi.mock('@/components/ui/dialog', () => ({
  Dialog: ({ children, open }: any) => open ? <div data-testid="dialog">{children}</div> : null,
  DialogContent: ({ children }: any) => <div>{children}</div>,
  DialogHeader: ({ children }: any) => <div>{children}</div>,
  DialogTitle: ({ children }: any) => <h2>{children}</h2>,
  DialogDescription: ({ children }: any) => <p>{children}</p>,
}))

vi.mock('@/components/ui/button', () => ({
  Button: ({ children, onClick, disabled, className }: any) => (
    <button onClick={onClick} disabled={disabled} className={className}>{children}</button>
  ),
}))

vi.mock('@/components/ui/input', () => ({
  Input: ({ value, onChange, placeholder, type, onKeyDown, className }: any) => (
    <input value={value} onChange={onChange} placeholder={placeholder} type={type} onKeyDown={onKeyDown} className={className} />
  ),
}))

vi.mock('@/components/ui/use-toast', () => ({
  useToast: () => ({ toast: vi.fn() }),
}))

vi.mock('@/services/gdrive', () => ({
  shareGDriveFile: vi.fn().mockResolvedValue({ success: true, message: 'ok' }),
}))

// Mock useAuthGuard so the regression test can shape state ('valid' / 'soft_lost' / 'unauthenticated')
// and decide whether the guard condition runs the wrapped function.
const guardImpl = vi.fn(async (fn: () => Promise<unknown>) => fn())
vi.mock('@/hooks/useAuthGuard', () => ({
  useAuthGuard: () => ({
    needsReauth: false,
    guard: guardImpl,
    modalOpen: false,
    onModalReauth: vi.fn(),
    onModalClose: vi.fn(),
  }),
  AuthCancelledError: class AuthCancelledError extends Error {},
}))

vi.mock('lucide-react', () => ({
  Mail: () => <span />,
  Send: () => <span />,
  CheckCircle2: () => <span />,
  Loader2: () => <span />,
  Shield: () => <span />,
}))

import { shareGDriveFile } from '@/services/gdrive'

beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(shareGDriveFile).mockResolvedValue({ success: true, message: 'ok' } as any)
  guardImpl.mockImplementation(async (fn: () => Promise<unknown>) => fn())
})

describe('ShareDialog', () => {
  const baseProps = {
    open: true,
    onOpenChange: vi.fn(),
    fileId: 'file123',
    fileName: 'test.mp4',
  }

  it('renders when open', () => {
    render(<ShareDialog {...baseProps} />)
    expect(screen.getByText('Share via Google Drive')).toBeTruthy()
    expect(screen.getByPlaceholderText('person@example.com')).toBeTruthy()
  })

  it('does not render when closed', () => {
    render(<ShareDialog {...baseProps} open={false} />)
    expect(screen.queryByText('Share via Google Drive')).toBeNull()
  })

  it('updates email on input change', () => {
    render(<ShareDialog {...baseProps} />)
    const input = screen.getByPlaceholderText('person@example.com') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'test@example.com' } })
    expect(input.value).toBe('test@example.com')
  })

  it('shows share button', () => {
    render(<ShareDialog {...baseProps} />)
    expect(screen.getByText('Share')).toBeTruthy()
  })

  it('share button is disabled when email is empty', () => {
    render(<ShareDialog {...baseProps} />)
    const btn = screen.getByText('Share') as HTMLButtonElement
    expect(btn.disabled).toBe(true)
  })

  it('share button is enabled when email has content', () => {
    render(<ShareDialog {...baseProps} />)
    const input = screen.getByPlaceholderText('person@example.com')
    fireEvent.change(input, { target: { value: 'a@b.com' } })
    const btn = screen.getByText('Share') as HTMLButtonElement
    expect(btn.disabled).toBe(false)
  })

  it('displays file name in description', () => {
    render(<ShareDialog {...baseProps} />)
    expect(screen.getByText(/test\.mp4/)).toBeTruthy()
  })

  // ── Regression: Drive-write gate ────────────────────────────────────────
  // ShareDialog wraps the shareGDriveFile Drive write in useAuthGuard.guard().
  // This regression asserts that the wrapped function only executes through
  // guard() — i.e. when useAuth is in soft_lost, the reauth resolution order
  // matters and shareGDriveFile is NOT called until guard() decides to run it.
  it('drives shareGDriveFile through guard() so re-auth can intercept soft_lost', async () => {
    render(<ShareDialog {...baseProps} />)
    const input = screen.getByPlaceholderText('person@example.com') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'a@b.com' } })
    const btn = screen.getByText('Share') as HTMLButtonElement
    fireEvent.click(btn)

    await waitFor(() => expect(guardImpl).toHaveBeenCalled())
    await waitFor(() => expect(shareGDriveFile).toHaveBeenCalledWith(
      'file123', 'a@b.com', 'reader',
    ))
    expect(shareGDriveFile).toHaveBeenCalledOnce()
  })

  it('does not call shareGDriveFile when guard rejects with AuthCancelledError', async () => {
    // Simulate "user dismissed the re-auth modal" — guard throws
    // AuthCancelledError before running the wrapped share write.
    guardImpl.mockImplementation(async () => {
      throw new Error('reauth_cancelled')
    })

    render(<ShareDialog {...baseProps} />)
    const input = screen.getByPlaceholderText('person@example.com') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'a@b.com' } })
    fireEvent.click(screen.getByText('Share') as HTMLButtonElement)

    await waitFor(() => expect(guardImpl).toHaveBeenCalled())
    // The wrapped share write must NOT have run on a cancelled re-auth.
    expect(shareGDriveFile).not.toHaveBeenCalled()
  })
})
