import SwiftUI

/// Asks for the computer to connect to until one is saved, then shows `content`.
struct Paired<Content: View>: View {
    @AppStorage("vornHost") private var host = ""
    @AppStorage("vornToken") private var token = ""
    @State private var core: Core?
    let content: (Core) -> Content

    var body: some View {
        if let core {
            content(core)
        } else if host.isEmpty {
            PairForm { h, t in host = h; token = t; core = Core(host: h, token: t) }
        } else {
            Color.clear.onAppear { core = Core(host: host, token: token) }
        }
    }
}

struct PairForm: View {
    @State private var host = ""
    @State private var token = ""
    let done: (String, String) -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("192.168.0.14:7777", text: $host)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("Device token", text: $token)
                } footer: {
                    Text("The address and a token from Settings › Devices on your computer.")
                }
            }
            .navigationTitle("Pair")
            .toolbar {
                Button("Connect") { done(host, token) }.disabled(host.isEmpty)
            }
        }
    }
}

/// Writes the metrics to Documents/metrics.json and the log every few seconds.
enum Report {
    static func start(_ core: Core, label: String) {
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in write(core, label: label) }
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
