// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { DeveloperConsole } from './DeveloperConsole'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
}))

vi.mock('framer-motion', () => ({
  LazyMotion: ({ children }: any) => <div>{children}</div>,
  m: { div: ({ children, ...p }: any) => <div {...p}>{children}</div> },
  AnimatePresence: ({ children }: any) => <div>{children}</div>,
  domAnimation: {},
}))

vi.mock('lucide-react', () => ({
  Trash2: () => <span />,
  Terminal: () => <span />,
  ChevronDown: () => <span />,
  ChevronUp: () => <span />,
}))

describe('DeveloperConsole', () => {
  it('renders toggle button', () => {
    render(<DeveloperConsole />)
    expect(screen.getByText('Console')).toBeTruthy()
  })

  it('toggles open state on click', () => {
    const { container } = render(<DeveloperConsole />)
    fireEvent.click(screen.getByText('Console'))
    // Should have more content after opening
    expect(container.querySelectorAll('button').length).toBeGreaterThan(1)
  })
})
