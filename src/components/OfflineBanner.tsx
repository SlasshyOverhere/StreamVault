import { useEffect, useState } from 'react'
import { WifiOff, X } from 'lucide-react'
import { useConnectivity } from '@/hooks/useConnectivity'

/**
 * Persistent amber warning banner shown when the app detects it is offline.
 * Dismissable by the user and auto-hides once connectivity is restored.
 */
export function OfflineBanner() {
  const { online } = useConnectivity()
  const [dismissed, setDismissed] = useState(false)

  useEffect(() => {
    if (online) {
      // Reset dismissal so the banner appears again on the next outage.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setDismissed(false)
    }
  }, [online])

  if (online || dismissed) {
    return null
  }

  return (
    <div
      role="alert"
      aria-live="polite"
      className="fixed top-0 left-0 right-0 z-50 flex items-center justify-between gap-3 bg-amber-600 px-4 py-2 text-white shadow-md"
    >
      <div className="flex items-center gap-2 text-sm font-medium">
        <WifiOff className="size-4" aria-hidden="true" />
        <span>No internet connection. Some features may be unavailable.</span>
      </div>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        aria-label="Dismiss offline notice"
        className="rounded p-1 hover:bg-white/20 focus:outline-none focus:ring-2 focus:ring-white/60"
      >
        <X className="size-4" />
      </button>
    </div>
  )
}
