// The build a fresh install runs: the program is made, and says when.
import fs from 'node:fs'

fs.mkdirSync('dist', { recursive: true })
fs.copyFileSync('src/main.mjs', 'dist/main.mjs')
fs.writeFileSync('dist/built.txt', new Date().toISOString())
