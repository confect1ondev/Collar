import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 4222,
    proxy: {
      '/api': 'http://localhost:4221',
      '/ws': {
        target: 'ws://localhost:4221',
        ws: true,
      },
    },
  },
  preview: {
    port: 4222,
    host: '127.0.0.1',
    allowedHosts: ['collar.kxra.me'],
  },
})
