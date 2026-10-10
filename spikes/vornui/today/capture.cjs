// Captures today's main screen with the window never shown: run as
//   HOME=<tmp> VORN_VORND_PATH=<vornd> electron capture.cjs <out/main/index.js> <out.png> <tmp>
// HOME and userData point at <tmp>, so the app and the vornd it starts never see ~/.vorn.
const { app, BrowserWindow } = require('electron')
const fs = require('fs')
const path = require('path')

const [mainJs, out, tmp] = process.argv.slice(-3)
app.setPath('userData', path.join(tmp, 'userData'))
if (app.dock) app.dock.hide()
for (const m of ['show', 'maximize', 'focus', 'showInactive']) BrowserWindow.prototype[m] = () => {}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
let taken = false
app.on('browser-window-created', (_, win) => {
  win.webContents.once('did-finish-load', async () => {
    if (taken) return
    taken = true
    win.setContentSize(1440, 900)
    await sleep(Number(process.env.CAPTURE_WAIT_MS || 8000))
    // A fresh profile opens the first-run guide and the sidebar; close both.
    await win.webContents.executeJavaScript(`(async () => {
      const wait = (ms) => new Promise((r) => setTimeout(r, ms))
      const byText = (t) => [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === t)
      byText('Skip guide')?.click()
      await wait(800)
      document.querySelector('svg path[d="M9 3v18"]')?.closest('button')?.click()
      // The agent chip names a product; show it as a generic "Agent".
      const btns = [...document.querySelectorAll('button')]
      const chip = btns[btns.findIndex((b) => b.textContent.trim().startsWith('Select project')) + 1]
      if (chip) {
        const walk = document.createTreeWalker(chip, NodeFilter.SHOW_TEXT)
        for (let n; (n = walk.nextNode()); ) if (n.nodeValue.trim()) { n.nodeValue = 'Agent'; break }
        chip.querySelector('img, svg')?.remove()
      }
      await wait(800)
    })()`)
    const img = await win.webContents.capturePage(undefined, { stayHidden: true })
    fs.writeFileSync(out, img.toPNG())
    console.log(`captured ${out} ${JSON.stringify(img.getSize())}`)
    app.exit(0)
  })
})
setTimeout(() => {
  console.error('capture timed out')
  app.exit(1)
}, 60000)
require(path.resolve(mainJs))
