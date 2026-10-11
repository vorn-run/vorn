import SwiftUI
import VornCore
import VornUI

/// LogReplayModal.tsx: a step's full output, replayed in a terminal-like sheet.
public struct LogReplayModal: View {
    let logs: String
    let onClose: () -> Void

    public init(logs: String, onClose: @escaping () -> Void) {
        self.logs = logs
        self.onClose = onClose
    }

    public var body: some View {
        Text(logs).onTapGesture(perform: onClose)
    }
}
