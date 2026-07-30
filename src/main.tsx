import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx'
import ErrorBoundary from './components/ErrorBoundary'
import { ConnectivityProvider } from './hooks/useConnectivity'
import './index.css'

// Automatic DPI / resolution scaling.
// The Tauri window is fixed at 1395x901 (see tauri.conf.json); we want the
// same logical layout at any monitor size or DPI. We compute a scale factor
// from window.devicePixelRatio and apply it via the --app-scale CSS variable
// on :root, which is consumed by `zoom: var(--app-scale)` on #root in
// index.css. The scale is clamped to [0.5, 1] so the UI never becomes
// unusably small on extreme multi-monitor DPI setups.
function applyDpiScale() {
  const dpr = window.devicePixelRatio || 1
  const scale = Math.min(1, Math.max(0.5, 1 / dpr))
  document.documentElement.style.setProperty('--app-scale', String(scale))
}

applyDpiScale()
// Re-apply when the window is resized — covers dragging between monitors
// with different OS DPI scaling.
window.addEventListener('resize', applyDpiScale)

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <ConnectivityProvider>
        <App />
      </ConnectivityProvider>
    </ErrorBoundary>
  </React.StrictMode>,
)

