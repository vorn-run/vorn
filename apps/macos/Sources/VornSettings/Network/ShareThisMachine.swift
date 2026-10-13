import AppKit
import SwiftUI
import VornCore
import VornUI

/// ShareThisMachine.tsx: share this machine's server with other devices.
struct ShareThisMachine: View {
    @Bindable var net: RemoteAccessModel
    let enabled: Bool
    let onChange: (Bool) -> Void

    private var onTailnet: Bool { net.status?.running == true }

    private func apply(_ value: Bool) {
        net.confirming = false
        onChange(value)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SettingRow(label: "Share this machine", description: "Let other devices open Vorn in a browser") {
                ToggleSwitch(checked: enabled, label: "Share this machine") { value in
                    // Off needs no confirmation, and neither does on over an encrypted tailnet.
                    if !value || onTailnet { return apply(value) }
                    net.confirming = true
                }
            }
            if net.confirming { confirmPanel.padding(.top, 12) }
            if enabled {
                AddressBlock(net: net, onTailnet: onTailnet, tailscaleInstalled: net.status?.installed == true)
                DeviceTokenList(net: net)
            }
        }
    }

    /// Names what the wide bind costs; deliberately heavier than everything around it.
    private var confirmPanel: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Enable without Tailscale?").font(.ui(14, weight: .medium)).foregroundStyle(Theme.amber300)
                .lineHeight(20, size: 14).padding(.bottom, 4)
            VStack(alignment: .leading, spacing: 6) {
                Text("Vorn will accept connections from anything on this network, over plain HTTP. A device token is still required, but it travels unencrypted, so anyone who can read this network's traffic can take it. A token runs commands on this machine.")
                Text("Fine on a home or office network you trust. Not on shared or public wifi.")
            }
            .font(.ui(12)).foregroundStyle(Theme.gray400).lineHeight(16, size: 12)
            .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                NetButton.outline("Enable anyway") { apply(true) }
                NetButton.ghost("Cancel") { net.confirming = false }
            }
            .padding(.top, 12)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.amber500.opacity(0.04)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.amber500.opacity(0.2), lineWidth: 1))
    }
}

/// ShareThisMachine.tsx `AddressBlock`: where this Vorn can be reached, led by one address.
private struct AddressBlock: View {
    @Bindable var net: RemoteAccessModel
    let onTailnet: Bool
    let tailscaleInstalled: Bool

    var body: some View {
        if let urls = net.reachable?.urls, let primary = urls.first {
            let rest = Array(urls.dropFirst())
            VStack(alignment: .leading, spacing: 0) {
                NetCaption("Open in a browser").padding(.bottom, 6)
                HStack(spacing: 0) {
                    Text(primary).font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.green400)
                        .lineHeight(20, size: 14).lineLimit(1).truncationMode(.tail)
                    CopyButton(text: primary)
                    Spacer(minLength: 0)
                }
                qualifier(rest).padding(.top, 8)
                if net.showAllAddresses && !rest.isEmpty {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(rest, id: \.self) { url in
                            HStack(spacing: 0) {
                                Text(url).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray400)
                                    .lineLimit(1).truncationMode(.tail).frame(maxWidth: .infinity, alignment: .leading)
                                CopyButton(text: url)
                            }
                            .padding(.vertical, 4)
                        }
                        Text("Use whichever address the other device shares a network with.")
                            .font(.ui(10)).foregroundStyle(Theme.gray600).lineHeight(15, size: 10).padding(.top, 6)
                    }
                    .padding(.top, 12)
                    .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
                    .padding(.top, 12)
                }
            }
            .netCard()
            .padding(.top, 12)
        }
    }

    /// The honest difference between the addresses, under the first one.
    private func qualifier(_ rest: [String]) -> some View {
        HStack(spacing: 0) {
            Group {
                if onTailnet {
                    Text("Encrypted via Tailscale")
                } else {
                    Text("This network only, unencrypted")
                    if !tailscaleInstalled {
                        Text(" · ")
                        InlineLink("Add Tailscale") { NSWorkspace.shared.open(URL(string: "https://tailscale.com/download")!) }
                    }
                }
                if !rest.isEmpty {
                    Text(" · ")
                    InlineLink(net.showAllAddresses ? "Hide" : "\(rest.count) other address\(rest.count > 1 ? "es" : "")") {
                        net.showAllAddresses.toggle()
                    }
                }
            }
            .font(.ui(12)).foregroundStyle(Theme.gray600).lineHeight(16, size: 12)
        }
    }
}

/// `text-gray-400 hover:text-white` text inside a sentence.
struct InlineLink: View {
    let label: String
    let action: () -> Void
    init(_ label: String, action: @escaping () -> Void) {
        self.label = label
        self.action = action
    }
    var body: some View {
        Hovering { hover in
            Button(action: action) {
                Text(label).foregroundStyle(hover ? .white : Theme.gray400).animation(.easeInOut(duration: 0.15), value: hover)
            }
            .buttonStyle(.plain)
        }
    }
}
