#!/usr/bin/env python3
"""store.js: settings audit — drop dead notifyStarted, add theme/media prefs."""

P = '/home/z/my-project/download-engine/web/assets/store.js'
s = open(P).read()

old = """export const DEFAULT_PREFS = {
  notifyCompleted: true,
  notifyFailed: true,
  notifyStarted: false,
  confirmFileDelete: true,
  confirmRemove: false,
  density: 'comfortable',        // comfortable | compact
  animations: true,
  sidebarExpanded: true,
  openFolderOnComplete: false,"""

new = """export const DEFAULT_PREFS = {
  notifyCompleted: true,
  notifyFailed: true,
  confirmFileDelete: true,
  confirmRemove: false,
  density: 'comfortable',        // comfortable | compact
  animations: true,
  sidebarExpanded: true,
  openFolderOnComplete: false,
  theme: 'dark',                 // dark | light | purple | blue | <custom theme id>
  startDownloadsImmediately: true, // Add dialog: start right away vs add paused
  mediaFormat: 'mp4',            // default media format: mp4 | mp3
  mediaQuality: 'best',          // default quality: 'best' | height (480/720/…)"""

assert old in s, 'DEFAULT_PREFS block not found'
s = s.replace(old, new, 1)
open(P, 'w').write(s)
print('store.js prefs updated (notifyStarted removed; theme + media prefs added)')
