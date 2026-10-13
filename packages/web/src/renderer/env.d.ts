/// <reference types="vite/client" />
import type { VornAPI } from './api'

declare global {
  interface Window {
    api: VornAPI
  }
}
