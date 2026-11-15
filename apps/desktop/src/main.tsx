import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { Toaster } from 'sonner'
import { App } from './App.tsx'
import { primeTheme } from './lib/theme.ts'
import './styles/globals.css'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // 桌面端没有网络抖动这回事，重试只会让错误看起来更晚。
      retry: false,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
})

const rootElement = document.getElementById('root')
if (rootElement === null) throw new Error('index.html is missing #root')

// Before the first paint: a dark window flashing to light is worse than either theme.
primeTheme()

createRoot(rootElement).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
      <Toaster position="bottom-right" theme="dark" closeButton />
    </QueryClientProvider>
  </StrictMode>,
)
