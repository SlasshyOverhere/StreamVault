import { useState, useRef, useEffect, useCallback } from 'react'
import { X, Play, Pause, Volume2, VolumeX, Maximize2, Minimize2, SkipBack, SkipForward, Settings, Loader2, AlertTriangle } from 'lucide-react'
import { Slider } from '@/components/ui/slider'
import { invoke } from '@tauri-apps/api/core'

interface VideoPlayerProps {
    src: string
    title: string
    poster?: string
    onClose: () => void
    onProgress?: (currentTime: number, duration: number) => void
    initialTime?: number
    // Cloud streaming fields
    isCloud?: boolean
    accessToken?: string
    // Media ID for transcoding fallback
    mediaId?: number
}

export function VideoPlayer({ src, title, poster, onClose, onProgress, initialTime = 0, isCloud = false, accessToken, mediaId: _mediaId }: VideoPlayerProps) {
    const videoRef = useRef<HTMLVideoElement>(null)
    const containerRef = useRef<HTMLDivElement>(null)
    const progressReportRef = useRef<number>(0)

    const [isPlaying, setIsPlaying] = useState(false)
    const [currentTime, setCurrentTime] = useState(0)
    const [duration, setDuration] = useState(0)
    const [volume, setVolume] = useState(1)
    const [isMuted, setIsMuted] = useState(false)
    const [isFullscreen, setIsFullscreen] = useState(false)
    const [showControls, setShowControls] = useState(true)
    const [isLoading, setIsLoading] = useState(true)
    const [error, setError] = useState<string | null>(null)
    const [videoSrc, setVideoSrc] = useState<string | null>(null)
    const blobUrlRef = useRef<string | null>(null)

    // Transcoding state
    const [isTranscoding, setIsTranscoding] = useState(false)
    const transcodeAttemptedRef = useRef(false)

    const hideControlsTimeout = useRef<NodeJS.Timeout | null>(null)

    // Attempt to start transcoding when playback fails
    const attemptTranscode = useCallback(async (filePath: string) => {
        if (transcodeAttemptedRef.current || isCloud) {
            return false;
        }

        transcodeAttemptedRef.current = true;
        setIsTranscoding(true);
        setError(null);
        setIsLoading(true);

        try {
            const result = await invoke<{ session_id: number; stream_url: string }>('start_transcode_stream', {
                filePath: filePath,
                startTime: initialTime > 0 ? initialTime : null
            });

            setVideoSrc(result.stream_url);
            setIsTranscoding(false);
            return true;
        } catch (e) {
            console.error('[VideoPlayer] Transcode failed:', e)
            setIsTranscoding(false);
            setError(`Transcoding failed: ${e}. Please configure FFmpeg in Settings > Player, or use MPV/VLC player.`);
            setIsLoading(false);
            return false;
        }
    }, [isCloud, initialTime]);

    // Load video file as blob URL using Tauri commands
    useEffect(() => {
        let cancelled = false;

        async function loadVideo() {
            setError(null);
            setIsLoading(true);

            try {
                if (src.startsWith('http://') || src.startsWith('https://') || src.startsWith('blob:')) {
                    setVideoSrc(src);
                    return;
                }

                const ext = src.split('.').pop()?.toLowerCase();

                const needsTranscode = ['mkv', 'avi', 'wmv', 'flv', 'mov', 'm2ts', 'ts', 'vob', 'divx', 'xvid', 'rmvb', 'rm'];

                if (needsTranscode.includes(ext || '')) {
                    const success = await attemptTranscode(src);
                    if (!success && !cancelled) {
                        setError(`This video format (${ext?.toUpperCase()}) requires FFmpeg transcoding. Please configure FFmpeg in Settings > Player, or use MPV/VLC player instead.`);
                    }
                    return;
                }

                const fileSize = await invoke<number>('get_video_file_size', { filePath: src });

                const chunkSize = 10 * 1024 * 1024;
                const chunks: Uint8Array[] = [];
                let offset = 0;

                while (offset < fileSize) {
                    if (cancelled) return;

                    const chunk = await invoke<number[]>('read_video_chunk', {
                        filePath: src,
                        offset: offset,
                        chunkSize: Math.min(chunkSize, fileSize - offset)
                    });

                    chunks.push(new Uint8Array(chunk));
                    offset += chunk.length;
                }

                if (cancelled) return;

                let mimeType = 'video/mp4';
                if (ext === 'webm') mimeType = 'video/webm';
                else if (ext === 'm4v') mimeType = 'video/mp4';

                const blob = new Blob(chunks as BlobPart[], { type: mimeType });
                const url = URL.createObjectURL(blob);

                if (blobUrlRef.current) {
                    URL.revokeObjectURL(blobUrlRef.current);
                }
                blobUrlRef.current = url;

                setVideoSrc(url);
            } catch (e) {
                if (!cancelled) {
                    setError(`Failed to load video: ${e}`);
                }
            } finally {
                if (!cancelled) {
                    setIsLoading(false);
                }
            }
        }

        loadVideo();

        return () => {
            cancelled = true;
            if (blobUrlRef.current) {
                URL.revokeObjectURL(blobUrlRef.current);
                blobUrlRef.current = null;
            }
        };
    }, [src, isCloud, accessToken, attemptTranscode]);

    // Handle video playback error - try transcoding as fallback
    const handleVideoError = useCallback(async (e: React.SyntheticEvent<HTMLVideoElement>) => {
        const video = e.currentTarget;
        const errorCode = video.error?.code;
        const errorMessage = video.error?.message || 'Unknown error';

        if (errorCode === 4 && !transcodeAttemptedRef.current && !isCloud && src && !src.startsWith('http')) {
            const success = await attemptTranscode(src);
            if (success) {
                return;
            }
        }

        setError(`Failed to play video: ${errorMessage}. Try using MPV or VLC player instead.`);
        setIsLoading(false);
    }, [src, isCloud, attemptTranscode]);

    // Format time helper
    const formatTime = (seconds: number): string => {
        const hrs = Math.floor(seconds / 3600)
        const mins = Math.floor((seconds % 3600) / 60)
        const secs = Math.floor(seconds % 60)
        if (hrs > 0) {
            return `${hrs}:${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`
        }
        return `${mins}:${secs.toString().padStart(2, '0')}`
    }

    // Start hiding controls timer
    const startHideControlsTimer = useCallback(() => {
        if (hideControlsTimeout.current) {
            clearTimeout(hideControlsTimeout.current)
        }
        hideControlsTimeout.current = setTimeout(() => {
            if (isPlaying) {
                setShowControls(false)
            }
        }, 3000)
    }, [isPlaying])

    // Show controls on mouse move
    const handleMouseMove = useCallback(() => {
        setShowControls(true)
        startHideControlsTimer()
    }, [startHideControlsTimer])

    // Toggle play/pause
    const togglePlay = useCallback(() => {
        if (videoRef.current) {
            if (isPlaying) {
                videoRef.current.pause()
            } else {
                videoRef.current.play()
            }
        }
    }, [isPlaying])

    // Toggle fullscreen
    const toggleFullscreen = useCallback(() => {
        if (!containerRef.current) return

        if (!document.fullscreenElement) {
            containerRef.current.requestFullscreen()
            setIsFullscreen(true)
        } else {
            document.exitFullscreen()
            setIsFullscreen(false)
        }
    }, [])

    // Toggle mute
    const toggleMute = useCallback(() => {
        if (videoRef.current) {
            videoRef.current.muted = !isMuted
            setIsMuted(!isMuted)
        }
    }, [isMuted])

    // Seek video
    const handleSeek = useCallback((value: number[]) => {
        if (videoRef.current && duration) {
            const newTime = (value[0] / 100) * duration
            videoRef.current.currentTime = newTime
            setCurrentTime(newTime)
        }
    }, [duration])

    // Change volume
    const handleVolumeChange = useCallback((value: number[]) => {
        if (videoRef.current) {
            const newVolume = value[0] / 100
            videoRef.current.volume = newVolume
            setVolume(newVolume)
            setIsMuted(newVolume === 0)
        }
    }, [])

    // Skip forward/backward
    const skip = useCallback((seconds: number) => {
        if (videoRef.current) {
            videoRef.current.currentTime = Math.max(0, Math.min(duration, videoRef.current.currentTime + seconds))
        }
    }, [duration])

    // Keyboard controls
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            switch (e.key) {
                case ' ':
                case 'k':
                    e.preventDefault()
                    togglePlay()
                    break
                case 'f':
                    e.preventDefault()
                    toggleFullscreen()
                    break
                case 'm':
                    e.preventDefault()
                    toggleMute()
                    break
                case 'ArrowLeft':
                    e.preventDefault()
                    skip(-10)
                    break
                case 'ArrowRight':
                    e.preventDefault()
                    skip(10)
                    break
                case 'Escape':
                    if (isFullscreen) {
                        document.exitFullscreen()
                    } else {
                        onClose()
                    }
                    break
            }
        }

        window.addEventListener('keydown', handleKeyDown)
        return () => window.removeEventListener('keydown', handleKeyDown)
    }, [togglePlay, toggleFullscreen, toggleMute, skip, isFullscreen, onClose])

    // Report progress periodically
    const onProgressRef = useRef(onProgress)
    onProgressRef.current = onProgress

    // Fullscreen change listener
    useEffect(() => {
        const handleFullscreenChange = () => {
            setIsFullscreen(!!document.fullscreenElement)
        }
        document.addEventListener('fullscreenchange', handleFullscreenChange)
        return () => document.removeEventListener('fullscreenchange', handleFullscreenChange)
    }, [])

    return (
        <section
            ref={containerRef}
            aria-label="Video player"
            className="fixed inset-0 z-50 bg-gray-950 flex items-center justify-center font-sans"
            onMouseMove={handleMouseMove}
            onClick={(e) => {
                if (e.target === containerRef.current) {
                    togglePlay()
                }
            }}
            onKeyDown={() => {}}
        >
            {/* Video element */}
            <video
                ref={videoRef}
                src={videoSrc || undefined}
                poster={poster}
                aria-label={title}
                className="w-full h-full object-contain"
                onLoadedMetadata={() => {
                    if (videoRef.current) {
                        setDuration(videoRef.current.duration)
                        if (initialTime > 0) {
                            videoRef.current.currentTime = initialTime
                        }
                    }
                }}
                onCanPlay={() => setIsLoading(false)}
                onWaiting={() => setIsLoading(true)}
                onPlaying={() => {
                    setIsLoading(false)
                    setIsPlaying(true)
                }}
                onPause={() => setIsPlaying(false)}
                onTimeUpdate={() => {
                    if (videoRef.current) {
                        setCurrentTime(videoRef.current.currentTime)
                        if (onProgressRef.current) {
                            const now = Date.now()
                            if (now - progressReportRef.current > 5000) {
                                progressReportRef.current = now
                                onProgressRef.current(videoRef.current.currentTime, videoRef.current.duration)
                            }
                        }
                    }
                }}
                onError={handleVideoError}
                onEnded={onClose}
                autoPlay
            >
                <track kind="captions" label="English" srcLang="en" />
            </video>

            {/* Loading spinner */}
            {(isLoading || isTranscoding) && (
                <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/50 backdrop-blur-sm z-10 transition-opacity duration-300">
                    <div className="relative">
                        <Loader2 className="size-16 animate-spin text-white" />
                        <div className="absolute inset-0 bg-white/20 blur-xl rounded-full" />
                    </div>
                    {isTranscoding && (
                        <p className="text-white mt-4 text-sm">Transcoding video for playback…</p>
                    )}
                </div>
            )}

            {/* Error display */}
            {error && (
                <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/90 backdrop-blur-md text-white z-20 p-8 text-center">
                    <div className="bg-destructive/10 p-4 rounded-full mb-4">
                        <AlertTriangle className="size-12 text-destructive" />
                    </div>
                    <p className="text-xl font-semibold mb-2">Video Error</p>
                    <p className="text-muted-foreground mb-6 max-w-md">{error}</p>
                    <button
                        type="button"
                        onClick={onClose}
                        className="px-6 py-2.5 bg-white text-black font-semibold rounded-full hover:bg-white/90 transition-all transform hover:scale-105 shadow-lg"
                    >
                        Close Player
                    </button>
                </div>
            )}

            {/* Controls overlay */}
            <div
                className={`absolute inset-0 flex flex-col justify-between transition-all duration-500 ease-in-out ${showControls ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-4 pointer-events-none'
                    }`}
            >
                {/* Top bar */}
                <div className="bg-gradient-to-b from-black/60 to-transparent p-6 flex items-start justify-between backdrop-blur-[2px]">
                    <div className="flex flex-col">
                        <h1 className="text-white text-xl font-bold tracking-tight drop-shadow-md">{title}</h1>
                    </div>
                    <button
                        type="button"
                        onClick={onClose}
                        className="p-2.5 hover:bg-white/10 rounded-full transition-colors backdrop-blur-md bg-black/20 border border-white/5"
                    >
                        <X className="size-6 text-white" />
                    </button>
                </div>

                {/* Center play button */}
                <div className="flex-1 flex items-center justify-center">
                    <button
                        type="button"
                        onClick={togglePlay}
                        className="group relative p-8 rounded-full transition-all duration-300 transform hover:scale-110 active:scale-95"
                    >
                        <div className="absolute inset-0 bg-black/40 backdrop-blur-sm rounded-full border border-white/10 transition-colors group-hover:bg-black/60" />
                        {isPlaying ? (
                            <Pause className="relative size-16 text-white drop-shadow-lg" />
                        ) : (
                            <Play className="relative size-16 text-white ml-2 drop-shadow-lg" />
                        )}
                    </button>
                </div>

                {/* Bottom controls */}
                <div className="bg-gradient-to-t from-black/90 via-black/50 to-transparent pt-20 pb-8 px-8 space-y-4">
                    {/* Progress bar */}
                    <div className="flex items-center gap-4">
                        <span className="text-white/90 text-sm font-medium font-mono min-w-[50px]">
                            {formatTime(currentTime)}
                        </span>
                        <Slider
                            value={[duration ? (currentTime / duration) * 100 : 0]}
                            onValueChange={handleSeek}
                            max={100}
                            step={0.1}
                            className="flex-1 cursor-pointer"
                        />
                        <span className="text-white/60 text-sm font-medium font-mono min-w-[50px] text-right">
                            {formatTime(duration)}
                        </span>
                    </div>

                    {/* Control buttons */}
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-4">
                            {/* Play/Pause */}
                            <button
                                type="button"
                                onClick={togglePlay}
                                className="p-2.5 hover:bg-white/10 rounded-full transition-colors text-white hover:text-white"
                            >
                                {isPlaying ? (
                                    <Pause className="size-6 fill-current" />
                                ) : (
                                    <Play className="size-6 fill-current" />
                                )}
                            </button>

                            {/* Skip backward */}
                            <button
                                type="button"
                                onClick={() => skip(-10)}
                                className="p-2 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white group"
                                title="Skip back 10 seconds"
                            >
                                <SkipBack className="size-5 group-hover:-translate-x-0.5 transition-transform" />
                            </button>

                            {/* Skip forward */}
                            <button
                                type="button"
                                onClick={() => skip(10)}
                                className="p-2 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white group"
                                title="Skip forward 10 seconds"
                            >
                                <SkipForward className="size-5 group-hover:translate-x-0.5 transition-transform" />
                            </button>

                            {/* Volume */}
                            <div className="flex items-center gap-2 group/vol">
                                <button
                                    type="button"
                                    onClick={toggleMute}
                                    className="p-2 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white"
                                >
                                    {isMuted || volume === 0 ? (
                                        <VolumeX className="size-5" />
                                    ) : (
                                        <Volume2 className="size-5" />
                                    )}
                                </button>
                                <div className="w-0 group-hover/vol:w-28 overflow-hidden transition-all duration-300 ease-out pl-2">
                                    <Slider
                                        value={[isMuted ? 0 : volume * 100]}
                                        onValueChange={handleVolumeChange}
                                        max={100}
                                        step={1}
                                        className="cursor-pointer py-2"
                                    />
                                </div>
                            </div>
                        </div>

                        <div className="flex items-center gap-3">
                            {/* Settings */}
                            <button
                                type="button"
                                className="p-2.5 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white"
                                title="Settings"
                            >
                                <Settings className="size-5" />
                            </button>

                            {/* Fullscreen */}
                            <button
                                type="button"
                                onClick={toggleFullscreen}
                                className="p-2.5 hover:bg-white/10 rounded-full transition-colors text-white/80 hover:text-white"
                                title={isFullscreen ? 'Exit fullscreen' : 'Fullscreen'}
                            >
                                {isFullscreen ? (
                                    <Minimize2 className="size-5" />
                                ) : (
                                    <Maximize2 className="size-5" />
                                )}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </section>
    )
}
