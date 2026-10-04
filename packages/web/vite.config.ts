import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'
import path from 'node:path'

export default defineConfig({
  plugins: [
    react(),
    VitePWA({
      registerType: 'autoUpdate',
      manifest: false, // We maintain our own manifest.webmanifest in public/
      workbox: {
        navigateFallback: '/app/index.html',
        navigateFallbackAllowlist: [/^\/app/],
        runtimeCaching: [
          {
            // Cache static assets (JS, CSS, images, fonts)
            urlPattern: /\.(?:js|css|png|jpg|jpeg|svg|gif|woff2?|ttf|eot)$/,
            handler: 'CacheFirst',
            options: {
              cacheName: 'vorn-static',
              expiration: { maxEntries: 100, maxAgeSeconds: 30 * 24 * 60 * 60 }
            }
          },
          {
            // Network-first for API calls
            urlPattern: /\/api\//,
            handler: 'NetworkFirst',
            options: {
              cacheName: 'vorn-api',
              networkTimeoutSeconds: 10,
              expiration: { maxEntries: 50, maxAgeSeconds: 5 * 60 }
            }
          },
          {
            // Network-first for health check
            urlPattern: /\/health/,
            handler: 'NetworkFirst',
            options: {
              cacheName: 'vorn-health',
              networkTimeoutSeconds: 5
            }
          }
        ],
        // Offline fallback
        offlineGoogleAnalytics: false
      }
    })
  ],
  // Module workers, so the Shiki worker keeps its grammars as lazy chunks
  // instead of inlining every language into one file.
  worker: { format: 'es' },
  base: '/app/',
  resolve: {
    alias: {
      // The renderer uses relative imports like '../../shared/types'
      // which resolve to src/shared/ barrel files that re-export @vornrun/shared.
      // We alias them so Vite can resolve from the web package context.
      '@renderer': path.resolve(__dirname, '../../src/renderer'),
      '@shared': path.resolve(__dirname, '../../src/shared')
    }
  },
  server: {
    port: 5173,
    proxy: {
      '/ws': {
        target: 'ws://127.0.0.1:3456',
        ws: true
      },
      '/health': {
        target: 'http://127.0.0.1:3456'
      },
      '/api': {
        target: 'http://127.0.0.1:3456'
      }
    }
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true
  }
})
