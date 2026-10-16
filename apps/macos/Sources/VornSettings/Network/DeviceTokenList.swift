import SwiftUI
import VornCore
import VornUI

/// DeviceTokenList.tsx: the devices allowed to connect, and the only place to add one.
struct DeviceTokenList: View {
    @Bindable var net: RemoteAccessModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                NetCaption("Devices\(net.active.isEmpty ? "" : " · \(net.active.count)")")
                Spacer(minLength: 0)
                if net.adding == nil && net.minted == nil {
                    NetButton.small("Add device") { Task { await net.addDevice() } }.padding(.trailing, -8)
                }
            }
            .frame(minHeight: 24)

            if let minted = net.minted { mintedPanel(minted).padding(.top, 12) }
            if let asking = net.asking { askingPanel(asking).padding(.top, 12) }
            if net.adding == .scan && net.asking == nil {
                if net.showingCode, let pairUrl = net.pairUrl, let code = net.code {
                    scanPanel(pairUrl: pairUrl, code: code).padding(.top, 12)
                } else if !net.showingCode {
                    expiredPanel.padding(.top, 12)
                }
            }
            if net.adding == .token && net.asking == nil { tokenForm.padding(.top, 12) }

            if let error = net.error {
                Text(error).font(.ui(12)).foregroundStyle(Theme.danger).lineHeight(16, size: 12).padding(.top, 8)
            }
            if net.tokens != nil && net.active.isEmpty && net.adding == nil && net.minted == nil {
                Text("No devices yet. Add one, then open the address above on it.")
                    .font(.ui(12)).foregroundStyle(Theme.gray600).lineHeight(16, size: 12).padding(.top, 8)
            }
            if !net.active.isEmpty {
                VStack(spacing: 0) {
                    ForEach(Array(net.active.enumerated()), id: \.element.id) { i, token in
                        row(token)
                            .overlay(alignment: .top) { if i > 0 { Rectangle().fill(Theme.white(0.04)).frame(height: 1) } }
                    }
                }
                .padding(.top, 4)
            }
        }
        .netCard()
        .padding(.top, 12)
        .task { await net.load() }
        .task { await net.watch() }
        // Leaving the page puts any code away: it is only safe while watched.
        .onDisappear { net.cancelPairing() }
    }

    /// The one moment the token is readable, said plainly.
    private func mintedPanel(_ minted: (name: String, plaintext: String)) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Token for \u{201C}\(minted.name)\u{201D}").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200)
                .lineHeight(20, size: 14)
            Text("Copy it now. Only its hash is stored, so this is the only time it can be shown.")
                .font(.ui(12)).foregroundStyle(Theme.gray500).lineHeight(16, size: 12)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 4).padding(.bottom, 10)
            HStack(spacing: 0) {
                Text(minted.plaintext).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.ink)
                    .lineLimit(1).truncationMode(.tail).frame(maxWidth: .infinity, alignment: .leading)
                CopyButton(text: minted.plaintext)
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Color.black.opacity(0.3)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
            NetButton.small("Done", color: Theme.gray500) { net.minted = nil }
                .padding(.leading, -8).padding(.top, 10)
        }
        .netCallout(border: Theme.bronzo.opacity(0.3))
    }

    private func askingPanel(_ asking: PairingRequest) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Circle().fill(Theme.bronzo).frame(width: 6, height: 6)
                Text("A phone is asking to pair").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200).lineHeight(20, size: 14)
            }
            VStack(spacing: 4) {
                detail("Device", asking.deviceName)
                detail("Address", asking.address)
            }
            .padding(.top, 8)
            Text("This lets it run terminals on this machine. Only approve a phone you are holding.")
                .font(.ui(11)).foregroundStyle(Theme.gray500).lineHeight(16, size: 11)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 8)
            HStack(spacing: 8) {
                NetButton.outline("Approve") { Task { await net.decide(true) } }
                NetButton.ghost("Deny") { Task { await net.decide(false) } }
            }
            .padding(.top, 10)
        }
        .netCallout(border: Theme.bronzo.opacity(0.4))
    }

    private func detail(_ term: String, _ value: String) -> some View {
        HStack(spacing: 12) {
            Text(term).foregroundStyle(Theme.gray500).font(.ui(12))
            Spacer(minLength: 0)
            Text(value).foregroundStyle(Theme.gray300).font(.system(size: 12, design: .monospaced)).lineLimit(1).truncationMode(.tail)
        }
        .lineHeight(16, size: 12)
    }

    private func scanPanel(pairUrl: String, code: String) -> some View {
        HStack(alignment: .top, spacing: 16) {
            ScannableQRCode(url: RemoteAccess.pairingLink(url: pairUrl, code: code))
            VStack(alignment: .leading, spacing: 0) {
                Text("Scan this in the Vorn app.").font(.ui(14)).foregroundStyle(Theme.gray200).lineHeight(20, size: 14)
                Text("Then approve the phone here. It will ask by name.")
                    .font(.ui(12)).foregroundStyle(Theme.gray500).lineHeight(16, size: 12).padding(.top, 4)
                    .fixedSize(horizontal: false, vertical: true)
                if net.urls.count > 1 { addressChooser(pairUrl).padding(.top, 12) }
                Text("OR TYPE THIS CODE").font(.ui(10)).tracking(0.5).foregroundStyle(Theme.gray600).lineHeight(15, size: 10)
                    .padding(.top, 12)
                Text(code).font(.system(size: 16, design: .monospaced)).tracking(1.6).foregroundStyle(Theme.gray200)
                    .lineHeight(24, size: 16).padding(.top, 2)
                Text("Expires in \(RemoteAccess.formatRemaining(net.remaining))")
                    .font(.ui(11).monospacedDigit()).foregroundStyle(Theme.gray500).lineHeight(16.5, size: 11).padding(.top, 4)
                VStack(alignment: .leading, spacing: 4) {
                    NetButton.small("Pair manually instead") { net.pairManually() }
                    NetButton.small("Cancel", color: Theme.gray500) { net.closeAdding() }
                }
                .padding(.leading, -8)
                .padding(.top, 16)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Which address the code carries: the phone can only reach the one whose network it is on.
    private func addressChooser(_ pairUrl: String) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("ADDRESS IN THE CODE").font(.ui(10)).tracking(0.5).foregroundStyle(Theme.gray600).lineHeight(15, size: 10)
                .padding(.bottom, 4)
            FlowLayout(spacing: 6) {
                ForEach(net.urls, id: \.self) { url in
                    let chosen = url == pairUrl
                    Hovering { hover in
                        Button { net.chosenUrl = url } label: {
                            Text(RemoteAccess.addressHost(url)).font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(chosen ? Theme.gray200 : hover ? .white : Theme.gray500)
                                .padding(.horizontal, 8).padding(.vertical, 4)
                                .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                                    .fill(chosen ? Theme.white(0.08) : hover ? Theme.white(0.06) : .clear))
                                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                                    .strokeBorder(chosen ? Theme.white(0.2) : Theme.hairline, lineWidth: 1))
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .tooltip(url)
                    }
                }
            }
            Text(RemoteAccess.addressKind(pairUrl) == .tailnet
                 ? "Encrypted. The phone must be on your tailnet."
                 : "Unencrypted. The phone must be able to reach this address.")
                .font(.ui(11)).foregroundStyle(Theme.gray600).lineHeight(16.5, size: 11).padding(.top, 6)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var expiredPanel: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("That code expired.").font(.ui(12)).foregroundStyle(Theme.gray500).lineHeight(16, size: 12)
            HStack(spacing: 8) {
                NetButton.outline("Show another") { Task { await net.startScan() } }
                NetButton.ghost("Cancel", weight: .regular) { net.closeAdding() }
            }
            .padding(.top, 8)
        }
    }

    private var tokenForm: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("What is this device?").font(.ui(12)).foregroundStyle(Theme.gray400).lineHeight(16, size: 12).padding(.bottom, 6)
            NetInput(text: $net.name, placeholder: "My phone", autofocus: true) { Task { await net.create() } }
            HStack(spacing: 8) {
                NetButton.outline("Create token") { Task { await net.create() } }
                NetButton.ghost("Cancel") { net.closeAdding() }
            }
            .padding(.top, 10)
        }
    }

    private func row(_ token: DeviceToken) -> some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 0) {
                Text(token.name).font(.ui(14)).foregroundStyle(Theme.gray200).lineHeight(20, size: 14).lineLimit(1)
                Text(token.seen)
                    .font(.ui(10)).foregroundStyle(Theme.gray600).lineHeight(15, size: 10)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if net.confirmRevokeId == token.id {
                HStack(spacing: 6) {
                    NetButton(label: "Revoke", color: Theme.danger, border: Theme.danger.opacity(0.25),
                              hoverFill: Theme.danger.opacity(0.1), padding: (8, 4)) { Task { await net.revoke(token.id) } }
                    NetButton.small("Cancel", color: Theme.gray500) { net.confirmRevokeId = nil }
                }
            } else {
                NetButton.small("Remove", color: Theme.gray500) { net.confirmRevokeId = token.id }
            }
        }
        .padding(.vertical, 8)
    }
}
