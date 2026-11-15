import { useCallback, useEffect, useState } from 'react'

export type Theme = 'dark' | 'light'

/**
 * Theme, applied as a class on `<html>`.
 *
 * The stylesheet defines every colour twice under `:root` and `.light`; nothing here knows
 * what a colour is, so the palette can change without touching this file.
 *
 * The initial value is read from storage and applied *before* React paints, so the window
 * never flashes the wrong theme on launch.
 */
const STORAGE_KEY = 'dsh-desktop.theme'

function read(): Theme {
  const stored = window.localStorage.getItem(STORAGE_KEY)
  if (stored === 'light' || stored === 'dark') return stored
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

function apply(theme: Theme) {
  document.documentElement.classList.toggle('light', theme === 'light')
  document.documentElement.style.colorScheme = theme
}

export function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = useState<Theme>(read)

  useEffect(() => {
    apply(theme)
    window.localStorage.setItem(STORAGE_KEY, theme)
  }, [theme])

  const toggle = useCallback(() => {
    setTheme((current) => (current === 'dark' ? 'light' : 'dark'))
  }, [])

  return [theme, toggle]
}

/** Apply the stored theme before the first paint. Called from `main.tsx`. */
export function primeTheme(): void {
  apply(read())
}
