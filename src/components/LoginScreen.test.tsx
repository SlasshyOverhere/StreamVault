// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { LoginScreen } from './LoginScreen'

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    minimize: vi.fn(),
    hide: vi.fn(),
  }),
}))

vi.mock('lucide-react', () => ({
  Loader2: () => <span />,
  Film: () => <span />,
  Users: () => <span />,
  Shield: () => <span />,
  Zap: () => <span />,
  Minus: () => <span />,
  X: () => <span />,
}))

// Mock static asset import
vi.mock('@/assets/slasshyvault-icon-ui.png', () => ({ default: 'icon.png' }))

describe('LoginScreen', () => {
  it('renders login button when not loading', () => {
    render(<LoginScreen onLogin={() => {}} />)
    expect(screen.getByText('Continue with Google')).toBeTruthy()
    expect(screen.getAllByText('SlasshyVault').length).toBeGreaterThanOrEqual(1)
    expect(screen.getByText('Welcome')).toBeTruthy()
  })

  it('renders loading state', () => {
    render(<LoginScreen onLogin={() => {}} isLoading />)
    expect(screen.getByText(/Signing in/)).toBeTruthy()
  })

  it('calls onLogin on button click', () => {
    const onLogin = vi.fn()
    render(<LoginScreen onLogin={onLogin} />)
    fireEvent.click(screen.getByText('Continue with Google'))
    expect(onLogin).toHaveBeenCalled()
  })

  it('disables button when loading', () => {
    render(<LoginScreen onLogin={() => {}} isLoading />)
    const btn = screen.getByText(/Signing in/).closest('button')!
    expect(btn.disabled).toBe(true)
  })

  it('renders feature badges', () => {
    render(<LoginScreen onLogin={() => {}} />)
    expect(screen.getByText('Local & Cloud')).toBeTruthy()
    expect(screen.getByText('Watch Together')).toBeTruthy()
    expect(screen.getByText('Privacy-First')).toBeTruthy()
    expect(screen.getByText('Auto Sync')).toBeTruthy()
  })

  it('renders privacy links', () => {
    render(<LoginScreen onLogin={() => {}} />)
    expect(screen.getByText('Terms of Service')).toBeTruthy()
    expect(screen.getByText('Privacy Policy')).toBeTruthy()
  })
})
