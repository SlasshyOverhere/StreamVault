import { useState, useEffect } from 'react'
import { useAuth } from '@/hooks/useAuth'
import { AlertTriangle, X } from 'lucide-react'
import { ReauthModal } from './ReauthModal'

const DISMISS_KEY = 'slasshyvault.reauth-banner-dismissed'

export function ReauthBanner() {
  const { needsReauth, reauth } = useAuth()
  const [dismissed, setDismissed] = useState<boolean>(() => {
    try {
      return sessionStorage.getItem(DISMISS_KEY) === '1'
    } catch {
      return false
    }
  })
  const [modalOpen, setModalOpen] = useState(false)

  // When state recovers (someone else reauth'd), un-dismiss.
  useEffect(() => {
    if (!needsReauth) setDismissed(false)
  }, [needsReauth])

  if (!needsReauth || dismissed) return null

  const onDismiss = () => {
    try {
      sessionStorage.setItem(DISMISS_KEY, '1')
    } catch {
      /* sessionStorage unavailable: still hide for this render */
    }
    setDismissed(true)
  }

  return (
    <>
      <div
        role="alert"
        data-testid="reauth-banner"
        className="fixed left-0 right-0 top-0 z-[400] flex items-center justify-center gap-3 bg-amber-500/15 px-4 py-2 text-amber-100 backdrop-blur"
      >
        <AlertTriangle className="size-4 shrink-0" />
        <span className="text-sm">
          Your Drive session expired. Re-authenticate to continue syncing.
        </span>
        <button
          type="button"
          onClick={() => setModalOpen(true)}
          className="ml-2 rounded-md bg-amber-500/30 px-3 py-1 text-sm text-white hover:bg-amber-500/40"
        >
          Re-authenticate
        </button>
        <button
          type="button"
          aria-label="Dismiss"
          onClick={onDismiss}
          className="ml-1 rounded-md p-1 text-amber-100 hover:bg-amber-500/20"
        >
          <X className="size-4" />
        </button>
      </div>
      <ReauthModal
        open={modalOpen}
        onReauth={async () => {
          setModalOpen(false)
          await reauth()
        }}
        onClose={() => setModalOpen(false)}
      />
    </>
  )
}
