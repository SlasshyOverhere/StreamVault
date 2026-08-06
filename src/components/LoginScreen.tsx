/**
 * SlasshyVault Login Screen
 *
 * Forces users to sign in with Google before accessing the app.
 * Data is stored in user's own Google Drive for privacy.
 */

import { useState, useEffect } from 'react'
import { Loader2, Film, Users, Shield, Zap, Minus, X, Server } from 'lucide-react'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getConfig, saveConfig } from '@/services/api'
import slasshyvaultIcon from '@/assets/slasshyvault-icon-ui.png'
const appWindow = getCurrentWebviewWindow()

interface LoginScreenProps {
  onLogin: () => void
  isLoading?: boolean
}

export function LoginScreen({ onLogin, isLoading = false }: LoginScreenProps) {
  const [isHovered, setIsHovered] = useState(false)
  const [showBackendInput, setShowBackendInput] = useState(false)
  const [backendUrl, setBackendUrl] = useState('')

  useEffect(() => {
    getConfig().then(c => {
      if (c.dev_backend_url) setBackendUrl(c.dev_backend_url)
    }).catch(() => {})
  }, [])

  const saveBackendUrl = async () => {
    const trimmed = backendUrl.trim().replace(/\/+$/, '')
    if (trimmed && !trimmed.startsWith('http')) return
    try {
      const cfg = await getConfig()
      await saveConfig({ ...cfg, dev_backend_url: trimmed || undefined })
      setShowBackendInput(false)
    } catch {}
  }

  return (
    <div className="fixed inset-0 bg-[#0a0a0a] flex flex-col">
      {/* Custom Title Bar */}
      <header className="fixed top-0 left-0 right-0 h-9 z-[300] bg-background">
        <div className="relative h-full w-full flex items-center justify-between">
          <div
            data-tauri-drag-region
            className="absolute left-0 top-0 bottom-0 right-[120px]"
          />
          <div className="flex items-center gap-2 pl-3 select-none">
            <img
              data-tauri-drag-region
              src={slasshyvaultIcon}
              alt=""
              draggable={false}
              className="pointer-events-none size-4 object-contain"
            />
            <span data-tauri-drag-region className="pointer-events-none text-[10px] font-semibold uppercase tracking-[0.2em] text-neutral-400">
              SlasshyVault
            </span>
          </div>
          <div className="flex items-center gap-1 pr-1.5">
            <button
              type="button"
              onClick={() => appWindow.minimize()}
              className="h-7 w-8 rounded-md border border-transparent text-neutral-400 transition-colors hover:border-white/10 hover:bg-white/10 hover:text-white"
              title="Minimize"
              aria-label="Minimize window"
            >
              <Minus className="mx-auto size-3.5" />
            </button>
            <button
              type="button"
              onClick={async () => {
                await appWindow.hide()
              }}
              className="h-7 w-8 rounded-md border border-transparent text-neutral-400 transition-colors hover:border-rose-500/40 hover:bg-rose-500/20 hover:text-rose-200"
              title="Close"
              aria-label="Hide window"
            >
              <X className="mx-auto size-3.5" />
            </button>
          </div>
        </div>
      </header>

      {/* Content */}
      <div className="flex-1 flex pt-9">
        {/* Left Side - Branding */}
        <div className="flex-1 flex flex-col justify-center px-16 bg-gradient-to-br from-[#0a0a0a] via-[#111] to-[#0a0a0a]">
        {/* Logo and App Name */}
        <div className="flex items-center gap-4 mb-8">
          <div className="size-14 rounded-xl border border-white/20 bg-white/5 flex items-center justify-center shadow-lg shadow-white/10">
            <img
              src={slasshyvaultIcon}
              alt="SlasshyVault logo"
              draggable={false}
              className="size-10 object-contain"
            />
          </div>
          <div>
            <h1 className="text-2xl font-bold text-white">SlasshyVault</h1>
            <p className="text-sm text-neutral-500">Your Personal Media Library</p>
          </div>
        </div>

        {/* Tagline */}
        <h2 className="text-5xl font-bold text-white leading-tight mb-2">
          Watch anything.
        </h2>
        <h2 className="text-5xl font-bold text-neutral-500 leading-tight mb-8">
          From anywhere.
        </h2>

        {/* Description */}
        <p className="text-neutral-400 text-lg mb-10 max-w-md">
          Sign in with your Google account to sync your library across devices,
          watch with friends, and track your progress.
        </p>

        {/* Feature Badges */}
        <div className="flex flex-wrap gap-3">
          <FeatureBadge icon={<Film className="size-4" />} text="Local & Cloud" />
          <FeatureBadge icon={<Users className="size-4" />} text="Watch Together" />
          <FeatureBadge icon={<Shield className="size-4" />} text="Privacy-First" />
          <FeatureBadge icon={<Zap className="size-4" />} text="Auto Sync" />
        </div>
      </div>

      {/* Right Side - Login Form */}
      <div className="flex-1 flex flex-col items-center justify-center bg-[#111] border-l border-neutral-800/50">
        <div className="w-full max-w-sm px-8">
          {/* Welcome Text */}
          <h3 className="text-3xl font-bold text-white text-center mb-3">
            Welcome
          </h3>
          <p className="text-neutral-400 text-center mb-8">
            Sign in with your Google account to continue
          </p>

          {/* Google Sign In Button */}
          <button
            type="button"
            onClick={onLogin}
            disabled={isLoading}
            onMouseEnter={() => setIsHovered(true)}
            onMouseLeave={() => setIsHovered(false)}
            className={`
              w-full py-4 px-6 rounded-xl font-medium text-base
              flex items-center justify-center gap-3
              transition-all duration-200 ease-out
              ${isLoading
                ? 'bg-neutral-800 text-neutral-400 cursor-not-allowed'
                : 'bg-white text-neutral-900 hover:bg-neutral-100 hover:shadow-lg hover:shadow-white/10'
              }
              ${isHovered && !isLoading ? 'scale-[1.02]' : 'scale-100'}
            `}
          >
            {isLoading ? (
              <>
                <Loader2 className="size-5 animate-spin" />
                <span>Signing in…</span>
              </>
            ) : (
              <>
                <GoogleIcon />
                <span>Continue with Google</span>
              </>
            )}
          </button>

          {/* Privacy Notice */}
          <p className="text-neutral-500 text-xs text-center mt-6 leading-relaxed">
            By signing in, you agree to our{' '}
            <a href="https://slasshyoverhere.github.io/SlasshyVault/legal/terms" target="_blank" rel="noopener noreferrer" className="text-neutral-400 hover:text-white cursor-pointer underline underline-offset-2">Terms of Service</a>
            {' '}and{' '}
            <a href="https://slasshyoverhere.github.io/SlasshyVault/legal/privacy" target="_blank" rel="noopener noreferrer" className="text-neutral-400 hover:text-white cursor-pointer underline underline-offset-2">Privacy Policy</a>.
            <br />
            Your data is stored securely in your own Google Drive.
          </p>

          {/* Self-hosted backend option */}
          <div className="mt-4 text-center">
            {showBackendInput ? (
              <div className="flex flex-col gap-2 p-3 rounded-lg bg-neutral-800/50 border border-neutral-700/50">
                <div className="flex items-center gap-2">
                  <Server className="size-3.5 text-neutral-400 flex-shrink-0" />
                  <input
                    type="text"
                    value={backendUrl}
                    onChange={e => setBackendUrl(e.target.value)}
                    placeholder="https://your-backend.com"
                    className="flex-1 bg-transparent border border-neutral-700 rounded px-2 py-1.5 text-xs text-neutral-200 outline-none focus:border-neutral-500"
                    onKeyDown={e => e.key === 'Enter' && saveBackendUrl()}
                  />
                </div>
                <div className="flex gap-2">
                  <button onClick={saveBackendUrl} className="flex-1 text-xs py-1 rounded bg-neutral-700 text-neutral-200 hover:bg-neutral-600 transition-colors">Save</button>
                  <button onClick={() => { setShowBackendInput(false); setBackendUrl('') }} className="text-xs py-1 px-3 rounded text-neutral-500 hover:text-neutral-300 transition-colors">Reset</button>
                </div>
              </div>
            ) : (
              <button onClick={() => setShowBackendInput(true)} className="text-neutral-400 hover:text-white text-xs font-medium transition-colors underline underline-offset-2 decoration-neutral-600 hover:decoration-neutral-400">
                Self-hosted backend?
              </button>
            )}
          </div>

          {/* Divider */}
          <div className="flex items-center gap-4 mt-6">
            <div className="flex-1 h-px bg-neutral-800" />
            <span className="text-neutral-600 text-xs">Privacy-First Design</span>
            <div className="flex-1 h-px bg-neutral-800" />
          </div>
        </div>
      </div>
      </div>
    </div>
  )
}

function FeatureBadge({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <div className="flex items-center gap-2 px-4 py-2 rounded-full bg-neutral-800/50 border border-neutral-700/50 text-neutral-300 text-sm">
      {icon}
      <span>{text}</span>
    </div>
  )
}

function GoogleIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24">
      <path
        fill="#4285F4"
        d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"
      />
      <path
        fill="#34A853"
        d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
      />
      <path
        fill="#FBBC05"
        d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
      />
      <path
        fill="#EA4335"
        d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
      />
    </svg>
  )
}
