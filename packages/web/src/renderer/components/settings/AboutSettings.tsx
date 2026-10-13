import vornLogo from '../../assets/vorn-logo.png'
import { NativeCoreStatus } from './NativeCoreStatus'

export function AboutSettings() {
  return (
    <div>
      <h2 className="text-xl font-semibold text-white mb-6">About</h2>

      <div className="space-y-4">
        <div>
          <img src={vornLogo} alt="Vorn" className="h-6 mb-3 opacity-60" draggable={false} />
          <p className="text-sm text-gray-300">Vorn v{window.api.getAppVersion()}</p>
          <p className="text-xs text-gray-600 mt-1">&copy; 2026 Javier Canizalez</p>
        </div>
      </div>
      <NativeCoreStatus />
    </div>
  )
}
