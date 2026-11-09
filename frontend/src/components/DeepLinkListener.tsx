'use client'

import { useEffect, useRef } from 'react'
import { useRouter } from 'next/navigation'
import { invoke } from '@tauri-apps/api/core'
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link'
import { toast } from 'sonner'
import { parseMeetingLink } from '@/lib/meetingLink'

// specs/0087 — opens `nixon://meeting/<id>?t=<s>` links. Links only navigate: the single
// command called is the read-only resolver, and navigation is a plain router.push exactly
// like clicking a meeting in the sidebar (a recording in progress carries on).
//
// A cold start delivers the URL through onOpenUrl (getCurrent() can be empty), so both
// paths are wired and an identical URL seen twice within DEDUPE_MS opens once.
const DEDUPE_MS = 2000

interface LinkTarget {
  exists: boolean
  segmentId: string | null
}

export function DeepLinkListener() {
  const router = useRouter()
  const routerRef = useRef(router)
  routerRef.current = router
  const seen = useRef(new Map<string, number>())

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const open = async (url: string) => {
      const now = Date.now()
      const last = seen.current.get(url)
      if (last !== undefined && now - last < DEDUPE_MS) return
      seen.current.set(url, now)

      const link = parseMeetingLink(url)
      if (!link) {
        toast.error("Nixon couldn't open that link.")
        return
      }
      try {
        const target = await invoke<LinkTarget>('api_resolve_meeting_link', {
          meetingId: link.meetingId,
          t: link.t,
        })
        if (!target.exists) {
          toast.error("That meeting isn't in Nixon anymore.")
          return
        }
        const id = encodeURIComponent(link.meetingId)
        routerRef.current.push(
          target.segmentId
            ? `/meeting-details?id=${id}&segment=${encodeURIComponent(target.segmentId)}`
            : `/meeting-details?id=${id}`,
        )
      } catch (e) {
        console.error('[DeepLink] resolve failed:', e)
        toast.error("Nixon couldn't open that link.")
      }
    }

    const openAll = (urls: string[] | null | undefined) => {
      if (cancelled) return
      for (const url of urls ?? []) void open(url)
    }

    // Register the listener first so a cold-start delivery is never missed.
    onOpenUrl(openAll)
      .then((un) => {
        if (cancelled) un()
        else unlisten = un
      })
      .catch((e) => console.error('[DeepLink] onOpenUrl failed:', e))
    getCurrent()
      .then(openAll)
      .catch((e) => console.error('[DeepLink] getCurrent failed:', e))

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  return null
}
