import path from 'node:path'
import { app, Menu, BrowserWindow } from 'electron'

const isMac = process.platform === 'darwin'

export function createMenu(onToggleWidget?: () => void, onStopServer?: () => void): void {
  app.setAboutPanelOptions({
    applicationName: 'Vorn',
    applicationVersion: app.getVersion(),
    version: '',
    copyright: 'Vorn 2026',
    iconPath: path.join(__dirname, '../../resources/icon.png')
  })

  const template: Electron.MenuItemConstructorOptions[] = [
    ...(isMac
      ? [
          {
            label: app.name,
            submenu: [
              { role: 'about' as const },
              { type: 'separator' as const },
              { role: 'hide' as const },
              { role: 'hideOthers' as const },
              { role: 'unhide' as const },
              { type: 'separator' as const },
              { role: 'quit' as const }
            ]
          }
        ]
      : []),
    {
      label: 'File',
      submenu: [
        {
          label: 'New Session',
          accelerator: 'CmdOrCtrl+N',
          click: (): void => {
            const win = BrowserWindow.getFocusedWindow()
            win?.webContents.send('menu:new-agent')
          }
        },
        { type: 'separator' },
        {
          // The deliberate end, kept reachable now that quitting no longer ends
          // anything. Quitting is a window closing; this is the switch.
          label: 'Stop Sessions and Server',
          click: (): void => {
            onStopServer?.()
          }
        },
        { type: 'separator' },
        isMac ? { role: 'close' } : { role: 'quit' }
      ]
    },
    {
      label: 'Edit',
      submenu: [
        { role: 'undo' },
        { role: 'redo' },
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        { role: 'selectAll' }
      ]
    },
    {
      label: 'View',
      submenu: [
        { role: 'reload' },
        { role: 'forceReload' },
        { role: 'toggleDevTools' },
        { type: 'separator' },
        { role: 'togglefullscreen' }
      ]
    },
    {
      label: 'Window',
      submenu: [
        { role: 'minimize' },
        { role: 'zoom' },
        { type: 'separator' },
        {
          label: 'Toggle Widget',
          accelerator: 'CmdOrCtrl+Shift+W',
          click: (): void => {
            onToggleWidget?.()
          }
        }
      ]
    }
  ]

  Menu.setApplicationMenu(Menu.buildFromTemplate(template))
}
