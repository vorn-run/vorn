import SwiftUI
import VisionKit

/// Pairs a computer until one is saved, then shows `content` connected to the one in use.
struct Paired<Content: View>: View {
    @StateObject private var computers = Computers()
    @State private var core: Core?
    let content: (Core, Computers) -> Content

    var body: some View {
        Group {
            if let core {
                content(core, computers).id(ObjectIdentifier(core))
            } else if computers.current == nil {
                PairFlow(computers: computers)
            } else {
                ProgressView()
            }
        }
        .task(id: computers.active) { connect() }
    }

    private func connect() {
        core = nil
        guard let c = computers.current else { return }
        guard let token = computers.token(c) else { return computers.remove(c.id) }
        core = Core(host: c.host, token: token)
    }
}

/// The address and code exchange, shared by the scanner and the form.
enum Pairing {
    struct Payload { let url: String; let code: String }

    /// Reads `vorn://pair?url=…&code=…` (or `vorn:/pair?…`); anything else isn't ours.
    static func payload(_ raw: String) -> Payload? {
        guard let u = URLComponents(string: raw), u.scheme == "vorn" else { return nil }
        let target = u.host?.isEmpty == false ? u.host! : u.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard target == "pair",
              let url = u.queryItems?.first(where: { $0.name == "url" })?.value, !url.isEmpty,
              let code = u.queryItems?.first(where: { $0.name == "code" })?.value, !code.isEmpty else { return nil }
        return Payload(url: url, code: code)
    }

    /// The server's HTTP origin from what was typed or scanned, e.g. `http://host:port`.
    static func base(_ input: String) -> URL? {
        let t = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return nil }
        var u = URLComponents(string: t.contains("://") ? t : "http://\(t)")
        switch u?.scheme {
        case "http", "ws": u?.scheme = "http"
        case "https", "wss": u?.scheme = "https"
        default: return nil
        }
        guard let host = u?.host, !host.isEmpty else { return nil }
        u?.path = ""; u?.query = nil; u?.fragment = nil
        return u?.url
    }

    /// What the core connects to: `host:port`, or a `wss://` base for a secure server.
    static func coreHost(_ base: URL) -> String {
        let hostPort = base.port.map { "\(base.host!):\($0)" } ?? base.host!
        return base.scheme == "https" ? "wss://\(hostPort)" : hostPort
    }

    static func hostOf(_ host: String) -> String {
        host.replacingOccurrences(of: "wss://", with: "").replacingOccurrences(of: "ws://", with: "")
    }

    enum Failure: Error { case expired, spent, refused, unreachable, denied, timedOut }

    static func message(_ f: Failure, machine: String) -> String {
        switch f {
        case .expired: "That code has expired. Show a new one on your computer."
        case .spent: "That code was already used. Show a new one on your computer."
        case .refused: "That code was not accepted. Show a new one on your computer."
        case .unreachable: "Could not reach \(machine). Is this phone on the same network?"
        case .denied: "That computer refused this phone."
        case .timedOut: "Nobody answered before the code expired."
        }
    }

    private static func post(_ base: URL, _ path: String, _ body: [String: Any]) async throws -> (Int, [String: Any]) {
        var req = URLRequest(url: base.appendingPathComponent(path))
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        return (status, (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:])
    }

    /// Trades a code for a place in the computer's queue.
    static func redeem(_ base: URL, code: String) async -> Result<String, Failure> {
        do {
            let (status, r) = try await post(base, "api/pair/redeem", ["code": code, "deviceName": UIDevice.current.name])
            if (200..<300).contains(status), let id = r["requestId"] as? String { return .success(id) }
            switch r["error"] as? String {
            case "expired": return .failure(.expired)
            case "spent": return .failure(.spent)
            default: return .failure(.refused)
            }
        } catch {
            return .failure(.unreachable)
        }
    }

    /// Waits up to five minutes for someone to approve the phone; a dropped poll is retried.
    static func decision(_ base: URL, request: String) async -> Result<(token: String, name: String?), Failure> {
        let deadline = Date().addingTimeInterval(300)
        while Date() < deadline, !Task.isCancelled {
            if let (status, p) = try? await post(base, "api/pair/poll", ["requestId": request]) {
                if !(200..<300).contains(status) { return .failure(.timedOut) }
                switch p["status"] as? String {
                case "approved":
                    if let t = p["token"] as? String { return .success((t, p["name"] as? String)) }
                case "denied": return .failure(.denied)
                case "expired": return .failure(.timedOut)
                default: break
                }
            }
            try? await Task.sleep(for: .seconds(2))
        }
        return .failure(.timedOut)
    }
}

/// The first screen without a computer: why, and the two ways to pair.
struct PairFlow: View {
    @ObservedObject var computers: Computers
    var cancel: (() -> Void)?
    @State private var help = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    VornIcon(name: "monitor", ink: .secondary)
                        .frame(width: 48, height: 48)
                        .background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 16))
                        .padding(.bottom, 28)
                    Text("Connect your computer").font(.largeTitle.weight(.semibold))
                    Text("Watch your agents, answer one that is waiting, and drive a terminal from your phone.")
                        .font(.title3).foregroundStyle(.secondary).padding(.top, 16)
                    Spacer(minLength: 48)
                    NavigationLink(value: Step.scan) {
                        Label { Text("Scan the code") } icon: { VornIcon(name: "scan") }
                            .font(.headline).frame(maxWidth: .infinity, minHeight: 52)
                    }
                    .buttonStyle(.borderedProminent).buttonBorderShape(.capsule).tint(Palette.ink)
                    .foregroundStyle(Palette.bg)
                    NavigationLink(value: Step.manual) {
                        HStack(spacing: 6) { Text("Enter the details by hand"); VornIcon(name: "chevron-right").scaleEffect(0.7) }
                            .font(.subheadline.weight(.medium)).frame(maxWidth: .infinity, minHeight: 52)
                    }
                    .foregroundStyle(Palette.ink)
                    Spacer(minLength: 32)
                    Label { Text("Codes expire in five minutes · Nothing is saved") } icon: { VornIcon(name: "lock").scaleEffect(0.8) }
                        .font(.footnote).foregroundStyle(.tertiary)
                    DisclosureGroup(isExpanded: $help) {
                        VStack(alignment: .leading, spacing: 10) {
                            HelpStep(n: 1, text: "Open Vorn on your computer, then Settings, then Remote Access.")
                            HelpStep(n: 2, text: "Turn on sharing. Pairing only appears once it is on.")
                            HelpStep(n: 3, text: "Add device, then show a code to scan.")
                            Text("Pairing lets this phone run terminals on that machine, so approve it there before it works.")
                                .font(.footnote).foregroundStyle(.tertiary)
                        }
                        .padding(.top, 12)
                    } label: {
                        Label { Text("Where do I find the code?") } icon: { VornIcon(name: "circle-help") }
                            .font(.subheadline.weight(.medium)).foregroundStyle(Palette.ink)
                    }
                    .padding(.horizontal, 16).padding(.vertical, 12)
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(.white.opacity(0.1)))
                    .padding(.top, 20)
                }
                .padding(24)
                .frame(maxWidth: 520)
                .frame(maxWidth: .infinity)
            }
            .background(Palette.bg)
            .toolbar {
                if let cancel { Button("Cancel", action: cancel) }
            }
            .navigationDestination(for: Step.self) { step in
                switch step {
                case .scan: ScanScreen(computers: computers)
                case .manual: ManualScreen(computers: computers)
                }
            }
        }
    }

    enum Step: Hashable { case scan, manual }
}

private struct HelpStep: View {
    let n: Int
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("\(n)").font(.caption.weight(.semibold).monospacedDigit())
                .frame(width: 20, height: 20).background(.white.opacity(0.08), in: Circle())
            Text(text).font(.subheadline).foregroundStyle(.secondary)
        }
    }
}

/// Where a pairing stands, for the scanner and the form.
enum Stage: Equatable {
    case idle, waiting(String), failed(String)
}

/// Reads the code the desktop shows and pairs with it.
struct ScanScreen: View {
    @ObservedObject var computers: Computers
    @State private var stage = Stage.idle
    @State private var handled = false
    @State private var job: Task<Void, Never>?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            switch stage {
            case .failed(let message):
                Spacer()
                Text("That did not pair").font(.title2.weight(.semibold))
                Text(message).foregroundStyle(.secondary).padding(.top, 12)
                Button { handled = false; stage = .idle } label: {
                    Text("Scan again").font(.headline).frame(maxWidth: .infinity, minHeight: 50)
                }
                .buttonStyle(.borderedProminent).tint(Palette.ink).foregroundStyle(Palette.bg).padding(.top, 24)
                Spacer()
            case .waiting(let machine):
                Spacer()
                Text("Waiting for \(machine)").font(.title2.weight(.semibold))
                Text("Approve this phone on that computer. Scanning the code is not enough on its own.")
                    .foregroundStyle(.secondary).padding(.top, 12)
                ProgressView().padding(.top, 24)
                Spacer()
            case .idle:
                Text("Scan the code").font(.title2.weight(.semibold))
                Text("Settings, then Remote Access, then Pair a phone.")
                    .font(.footnote).foregroundStyle(.tertiary).padding(.top, 6)
                Group {
                    if DataScannerViewController.isSupported {
                        Scanner { scanned($0) }
                    } else {
                        ContentUnavailableView("Vorn needs the camera", image: "scan",
                                               description: Text("It reads the pairing code shown on your computer. Nothing is recorded and nothing leaves this phone."))
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .padding(.top, 20)
                Text("Hold steady").font(.footnote).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity).padding(.vertical, 16)
            }
        }
        .padding(.horizontal, 16)
        .background(Palette.bg)
        .navigationTitle("Pair")
        .navigationBarTitleDisplayMode(.inline)
        .onDisappear { job?.cancel() }
    }

    private func scanned(_ raw: String) {
        guard !handled, let p = Pairing.payload(raw) else { return }
        handled = true
        guard let base = Pairing.base(p.url) else {
            stage = .failed("That code carries an address Vorn cannot use.")
            return
        }
        job = Task { @MainActor in
            let host = Pairing.coreHost(base)
            let machine = Pairing.hostOf(host)
            if let failure = await pair(computers, base: base, code: p.code, name: nil, waiting: { stage = .waiting(machine) }) {
                stage = .failed(Pairing.message(failure, machine: machine))
            }
        }
    }
}

/// Redeems `code`, waits for approval and saves the computer; returns why it failed, if it did.
@MainActor
private func pair(_ computers: Computers, base: URL, code: String, name: String?, waiting: () -> Void) async -> Pairing.Failure? {
    let host = Pairing.coreHost(base)
    switch await Pairing.redeem(base, code: code) {
    case .failure(let f): return f
    case .success(let request):
        waiting()
        switch await Pairing.decision(base, request: request) {
        case .failure(let f): return f
        case .success(let ok):
            if Task.isCancelled { return nil }
            computers.add(name: name ?? ok.name ?? Pairing.hostOf(host), host: host, token: ok.token)
            return nil
        }
    }
}

/// The camera, reporting each QR code it reads.
private struct Scanner: UIViewControllerRepresentable {
    let found: (String) -> Void

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let vc = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])],
                                           qualityLevel: .balanced, isHighlightingEnabled: true)
        vc.delegate = context.coordinator
        try? vc.startScanning()
        return vc
    }

    func updateUIViewController(_ vc: DataScannerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        init(found: @escaping (String) -> Void) { self.found = found }

        func dataScanner(_ s: DataScannerViewController, didAdd items: [RecognizedItem], allItems: [RecognizedItem]) {
            for case .barcode(let b) in items {
                if let v = b.payloadStringValue { found(v) }
            }
        }
    }
}

/// Pairing without a camera: an address and a device token, or a pairing code.
struct ManualScreen: View {
    @ObservedObject var computers: Computers
    @State private var name = ""
    @State private var address = Bundle.main.object(forInfoDictionaryKey: "VornHost") as? String ?? ""
    @State private var secret = ""
    @State private var stage = Stage.idle
    @State private var job: Task<Void, Never>?

    var body: some View {
        Form {
            Section {
                Text("Enter the details").font(.title2.weight(.semibold))
                Text("Both are on the same screen as the code: Settings, then Remote Access.")
                    .foregroundStyle(.secondary)
            }
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets())
            Section("Name") {
                TextField(Pairing.base(address).map { Pairing.hostOf(Pairing.coreHost($0)) } ?? "This computer", text: $name)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
            }
            Section("Address") {
                TextField("192.168.0.4:61601", text: $address)
                    .keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    .fontDesign(.monospaced)
            }
            Section {
                SecureField("vorn_… or a pairing code", text: $secret)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .fontDesign(.monospaced)
            } header: {
                Text("Device token")
            } footer: {
                switch stage {
                case .failed(let m): Text(m).foregroundStyle(.red)
                case .waiting(let machine): Text("Waiting for \(machine). Approve this phone on that computer.")
                case .idle: EmptyView()
                }
            }
            Section {
                Button(action: submit) {
                    Group {
                        if case .waiting = stage { ProgressView() } else { Text("Connect").font(.headline) }
                    }
                    .frame(maxWidth: .infinity, minHeight: 44)
                }
                .disabled(isWaiting)
            } footer: {
                Text("A token is kept in this phone's keychain and never expires. It authorises running terminals on that machine, so treat it the way you would an SSH key.")
                    .padding(.top, 16)
            }
        }
        .scrollContentBackground(.hidden)
        .background(Palette.bg)
        .navigationTitle("Pair")
        .navigationBarTitleDisplayMode(.inline)
        .onDisappear { job?.cancel() }
    }

    private var isWaiting: Bool { if case .waiting = stage { true } else { false } }

    private func submit() {
        let value = secret.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let base = Pairing.base(address), !value.isEmpty else {
            stage = .failed("Both an address and a token are needed.")
            return
        }
        let host = Pairing.coreHost(base)
        let machine = Pairing.hostOf(host)
        let named = name.trimmingCharacters(in: .whitespaces)
        if value.hasPrefix("vorn_") {
            computers.add(name: named.isEmpty ? machine : named, host: host, token: value)
            return
        }
        stage = .waiting(machine)
        job = Task { @MainActor in
            if let f = await pair(computers, base: base, code: value, name: named.isEmpty ? nil : named,
                                  waiting: { stage = .waiting(machine) }) {
                stage = .failed(Pairing.message(f, machine: machine))
            }
        }
    }
}

/// Writes the metrics to Documents/metrics.json and the log every few seconds.
enum Report {
    static func start(_ core: Core, label: String) {
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak core] timer in
            guard let core else { return timer.invalidate() }
            write(core, label: label)
        }
    }

    static func write(_ core: Core, label: String, extra: [String: Any] = [:]) {
        var m = core.metrics()
        m["app"] = label
        m.merge(extra) { $1 }
        guard let data = try? JSONSerialization.data(withJSONObject: m, options: [.sortedKeys]) else { return }
        let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? data.write(to: dir.appendingPathComponent("metrics.json"))
        print("vorn-metrics", String(decoding: data, as: UTF8.self))
    }
}
