import SwiftUI
import VornCore
import VornUI

/// UseAnotherMachine.tsx: point this desktop at a Vorn running somewhere else. Applying restarts the app.
struct UseAnotherMachine: View {
    @Bindable var net: RemoteAccessModel
    let mode: ServerMode
    let currentUrl: String

    private var connected: Bool { mode == .host }

    var body: some View {
        if connected && !net.hostEditing { connectedCard } else { form }
    }

    private var connectedCard: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 8) {
                        Circle().fill(Theme.green400).frame(width: 6, height: 6)
                        Text("Connected").font(.ui(14)).foregroundStyle(Theme.gray200).lineHeight(20, size: 14)
                    }
                    Text(currentUrl).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray600)
                        .lineLimit(1).truncationMode(.tail).padding(.top, 6)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                HStack(spacing: 8) {
                    NetButton.outline("Change", color: Theme.gray300) { net.hostEditing = true }
                    NetButton.ghost("Disconnect") { Task { await net.useLocal() } }
                }
                .fixedSize()
            }
            .netCard()
            note("Sessions, tasks and projects live on that machine. Workflows and schedules run only while a desktop is attached to it.")
        }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                label("Server address")
                NetInput(text: $net.hostUrl, placeholder: "192.168.0.4:61601", mono: true)
                label("Device token from that machine").padding(.top, 12)
                NetInput(text: $net.hostToken, placeholder: "vorn_...", mono: true, secure: true)
                if let error = net.hostError {
                    Text(error).font(.ui(12)).foregroundStyle(Theme.amber400.opacity(0.8)).lineHeight(16, size: 12).padding(.top, 8)
                }
                HStack(spacing: 8) {
                    NetButton.outline("Connect and restart") { Task { await net.applyHost() } }
                    if connected {
                        NetButton.ghost("Cancel") {
                            net.hostEditing = false
                            net.hostError = nil
                            net.hostToken = ""
                        }
                    }
                }
                .padding(.top, 12)
            }
            .netCard()
            note("Vorn restarts to apply this. Workflows and schedules run only while a desktop is attached to the host, so a server on its own holds your data but fires no schedules.")
        }
    }

    private func label(_ text: String) -> some View {
        Text(text).font(.ui(12)).foregroundStyle(Theme.gray400).lineHeight(16, size: 12).padding(.bottom, 6)
    }

    private func note(_ text: String) -> some View {
        Text(text).font(.ui(12)).foregroundStyle(Theme.gray600).lineHeight(16, size: 12)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.top, 12)
    }
}
