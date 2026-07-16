import { resolve } from 'path'
import { defineConfig } from 'electron-vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  main: {
    build: {
      externalizeDeps: true
    }
  },
  preload: {
    build: {
      externalizeDeps: true,
      rollupOptions: {
        input: {
          index: resolve(__dirname, 'src/preload/index.ts'),
          'widget-preload': resolve(__dirname, 'src/preload/widget-preload.ts')
        },
        external: ['electron'],
        output: {
          format: 'cjs',
          entryFileNames: '[name].js',
          chunkFileNames: 'chunks/[name]-[hash].js'
        }
      }
    }
  },
  renderer: {
    build: {
      rollupOptions: {
        input: {
          index: resolve(__dirname, 'src/renderer/index.html'),
          widget: resolve(__dirname, 'src/renderer/widget.html')
        },
        output: {
          manualChunks(id) {
            if (!id.includes('node_modules')) return
            if (/[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/.test(id)) {
              return 'vendor-react'
            }
            if (id.includes('@xterm/')) return 'vendor-xterm'
            if (id.includes('framer-motion')) return 'vendor-motion'
          }
        }
      }
    },
    plugins: [react()]
  }
})
