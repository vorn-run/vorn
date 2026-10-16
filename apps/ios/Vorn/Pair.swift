import SwiftUI

/// Asks for the computer to connect to until one is saved, then shows `content`, which can forget it.
struct Paired<Content: View>: View {
    @AppStorage("vornHost") private var host = ""
    @AppStorage("vornToken") private var token = ""
    @State private var core: Core?
    let content: (Core, @escaping () -> Void) -> Content

    var body: some View {
        if let core {
            content(core) { self.core = nil; host = ""; token = "" }
        } else if host.isEmpty {
            PairForm { h, t in host = h; token = t; core = Core(host: h, token: t) }
        } else {
            Color.clear.onAppear { core = Core(host: host, token: token) }
        }
    }
}

struct PairForm: View {
    @State private var host = Bundle.main.object(forInfoDictionaryKey: "VornHost") as? String ?? ""
    @State private var code = ""
    @State private var note = ""
    @State private var busy = false
    let done: (String, String) -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("192.168.0.14:56097", text: $host)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Pairing code", text: $code)
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                } footer: {
                    Text(note.isEmpty ? "The code from Settings › Devices on your computer, in any case, with or without hyphens. Approve the request there." : note)
                }
            }
            .navigationTitle("Pair")
            .toolbar {
                Button(busy ? "Waiting…" : "Connect") { Task { await pair() } }
                    .disabled(host.isEmpty || code.isEmpty || busy)
            }
        }
    }

    private func post(_ path: String, _ body: [String: Any]) async throws -> [String: Any] {
        var req = URLRequest(url: URL(string: "http://\(host)\(path)")!)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, _) = try await URLSession.shared.data(for: req)
        return (try JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    private func pair() async {
        busy = true; defer { busy = false }
        do {
            let r = try await post("/api/pair/redeem", ["code": code, "deviceName": UIDevice.current.name])
            guard let id = r["requestId"] as? String else { note = "Refused: \(r["error"] ?? "unknown")"; return }
            note = "Approve this phone in Vorn on your computer."
            for _ in 0..<120 {
                let p = try await post("/api/pair/poll", ["requestId": id])
                switch p["status"] as? String {
                case "approved": if let t = p["token"] as? String { done(host, t) }; return
                case "denied": note = "Denied on the computer."; return
                case "expired": note = "The code expired; make a new one."; return
                default: try await Task.sleep(nanoseconds: 1_000_000_000)
                }
            }
            note = "No answer from the computer."
        } catch {
            note = "Can't reach \(host): \(error.localizedDescription)"
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
