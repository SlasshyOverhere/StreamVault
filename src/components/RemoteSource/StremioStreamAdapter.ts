/**
 * Adapter that converts a Stremio stream object into the existing
 * `RemoteStreamData` shape used by the playback pipeline.
 *
 * The Stremio stream shape (https://github.com/Stremio/stremio-addon-sdk)
 * is loosely typed: a stream may have `url`, `infoHash`, `ytId`, `title`,
 * `name`, and `behaviorHints`. We normalize into the form expected by the
 * existing player modal.
 */
import type { RemoteStreamData } from './remote.types'

export interface StremioRawStream {
  url?: string | null
  infoHash?: string | null
  ytId?: string | null
  title?: string | null
  name?: string | null
  description?: string | null
  behaviorHints?: {
    notWebReady?: boolean
    bingeGroup?: string
    proxyHeaders?: Record<string, string>
    filename?: string
  }
  [key: string]: unknown
}

/**
 * Build a `RemoteStreamData` from a raw Stremio stream.
 *
 * Resolution of `infoHash`-only or debrid-CDN URLs is the caller's
 * responsibility: this function only adapts the shape.
 */
export function toRemoteStreamData(
  stream: StremioRawStream,
  options: { addonName: string; resolvedUrl?: string | null }
): RemoteStreamData | null {
  const url = options.resolvedUrl ?? stream.url ?? ''
  if (!url && !stream.infoHash && !stream.ytId) return null

  const name = (stream.name || stream.title || stream.behaviorHints?.filename || '').toString()
  const title = stream.title?.toString() || ''
  const description = stream.description?.toString() || ''
  const merged = [name, title, description].filter(Boolean).join(' — ')

  return {
    name: name || title || `${options.addonName} stream`,
    description: description || merged,
    url: url || '',
    videoSize: 0,
    notWebReady: stream.behaviorHints?.notWebReady ?? false,
    parsedQuality: guessQuality(merged || name || title),
    parsedSource: options.addonName,
    recommended: false,
    isHubdrive: false,
  }
}

function guessQuality(text: string): string {
  const lower = text.toLowerCase()
  if (lower.includes('2160p') || lower.includes('4k')) return '4K'
  if (lower.includes('1080p') || lower.includes('full hd') || lower.includes('fhd')) return '1080p'
  if (lower.includes('720p') || lower.includes('hd')) return '720p'
  if (lower.includes('480p')) return '480p'
  return 'auto'
}

/**
 * Determine whether a Stremio stream needs debrid resolution (i.e. has
 * `infoHash` but no usable `url`).
 */
export function streamNeedsDebrid(stream: StremioRawStream): boolean {
  if (stream.url && stream.url.length > 0) return false
  return Boolean(stream.infoHash)
}
