import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'
import tauriConfig from './src-tauri/tauri.conf.json' with { type: 'json' }

const devUrl = new URL(tauriConfig.build.devUrl)

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    host: devUrl.hostname,
    port: Number(devUrl.port),
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
})
