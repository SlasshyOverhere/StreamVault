import { createContext, useContext, useEffect, useState, useCallback } from 'react'
import { checkConnectivity } from '@/services/api'

interface ConnectivityContextValue {
  online: boolean
  isLoading: boolean
  check: () => Promise<void>
}

const ConnectivityContext = createContext<ConnectivityContextValue | null>(null)

const POLL_INTERVAL_MS = 30000

export function ConnectivityProvider({ children }: { children: React.ReactNode }) {
  const [online, setOnline] = useState(true)
  const [isLoading, setIsLoading] = useState(true)

  const check = useCallback(async () => {
    try {
      const connected = await checkConnectivity()
      setOnline(connected)
    } catch {
      setOnline(false)
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => {
    void check()

    const handleOnline = () => {
      // Re-verify with the backend probe; navigator.onLine is just a hint.
      void check()
    }
    const handleOffline = () => {
      void check()
    }

    window.addEventListener('online', handleOnline)
    window.addEventListener('offline', handleOffline)

    const interval = window.setInterval(() => {
      void check()
    }, POLL_INTERVAL_MS)

    return () => {
      window.removeEventListener('online', handleOnline)
      window.removeEventListener('offline', handleOffline)
      window.clearInterval(interval)
    }
  }, [check])

  return (
    <ConnectivityContext.Provider value={{ online, isLoading, check }}>
      {children}
    </ConnectivityContext.Provider>
  )
}

export function useConnectivity() {
  const context = useContext(ConnectivityContext)
  if (!context) {
    throw new Error('useConnectivity must be used within a ConnectivityProvider')
  }
  return context
}
