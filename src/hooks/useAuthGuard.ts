import { useState, useCallback } from 'react'
import { useAuth } from './useAuth'

export class AuthCancelledError extends Error {
  constructor(reason: 'not_authenticated' | 'reauth_cancelled') {
    super(reason)
    this.name = 'AuthCancelledError'
  }
}

/**
 * Wraps Drive write actions so that, when the user is in the "soft_lost"
 * state, a focused re-auth modal is triggered and the action runs only
 * after re-auth completes. Read-only flows should bypass this hook.
 *
 * State machine:
 *  - unauthenticated -> throws AuthCancelledError('not_authenticated')
 *  - valid          -> runs fn() immediately
 *  - soft_lost      -> opens modal, awaits reauth(), runs fn() iff reauth OK
 *
 * Task 6 (banner + modal) mounts a dialog reflecting `modalOpen`. The modal
 * also exposes explicit Cancel / Re-authenticate buttons so the user can
 * override the auto-reauth flow; those land on `onModalClose` / `onModalReauth`.
 */
export function useAuthGuard() {
  const { state, reauth } = useAuth()
  const [modalOpen, setModalOpen] = useState(false)

  const guard = useCallback(
    async <T,>(fn: () => Promise<T>): Promise<T> => {
      if (state === 'unauthenticated') {
        throw new AuthCancelledError('not_authenticated')
      }
      if (state !== 'soft_lost') {
        return fn()
      }
      // soft_lost: open the modal for UX feedback, then drive reauth ourselves.
      setModalOpen(true)
      let ok = false
      try {
        ok = await reauth()
      } finally {
        setModalOpen(false)
      }
      if (!ok) {
        throw new AuthCancelledError('reauth_cancelled')
      }
      return fn()
    },
    [state, reauth],
  )

  const onModalReauth = useCallback(async () => {
    await reauth()
  }, [reauth])

  const onModalClose = useCallback(() => {
    setModalOpen(false)
  }, [])

  return {
    needsReauth: state === 'soft_lost',
    guard,
    modalOpen,
    onModalReauth,
    onModalClose,
  }
}
