import CoreGraphics
import Foundation

/// One iOS simulator, as `device:list` answers it.
public struct Device: Equatable, Sendable, Identifiable {
    public var udid: String
    public var name: String
    public var runtime: String
    public var booted: Bool
    public var claimedBy: String?
    public var id: String { udid }

    public init(udid: String, name: String, runtime: String, booted: Bool, claimedBy: String? = nil) {
        self.udid = udid; self.name = name; self.runtime = runtime; self.booted = booted; self.claimedBy = claimedBy
    }

    public var json: JSONValue {
        var o: [String: JSONValue] = ["udid": .string(udid), "name": .string(name), "runtime": .string(runtime), "booted": .bool(booted)]
        if let claimedBy { o["claimedBy"] = .string(claimedBy) }
        return .object(o)
    }
}

/// Simulators for the device pane: the picker's order, when a card offers the button, and simctl.
public enum Devices {
    public struct Listed: Equatable, Sendable {
        public var device: Device
        public var selectable: Bool
        public var subtitle: String?
    }

    public struct Claimed: Equatable, Sendable {
        public var udid: String
        public var name: String
        public var bootedHere: Bool
    }

    /// Free, or already this session's.
    public static func isSelectable(_ d: Device, session: String) -> Bool {
        d.claimedBy == nil || d.claimedBy == session
    }

    /// Selectable first, then booted, then by name and runtime; a subtitle only where the name is ambiguous.
    public static func order(_ devices: [Device], session: String) -> [Listed] {
        var byName: [String: Int] = [:], byBoth: [String: Int] = [:]
        for d in devices {
            byName[d.name, default: 0] += 1
            byBoth[d.name + "\u{0}" + d.runtime, default: 0] += 1
        }
        let out = devices.map { d -> Listed in
            var subtitle: String?
            if byName[d.name, default: 0] > 1 {
                subtitle = byBoth[d.name + "\u{0}" + d.runtime, default: 0] > 1
                    ? "\(d.runtime) \u{b7} \(d.udid.prefix(8))" : d.runtime
            }
            return Listed(device: d, selectable: isSelectable(d, session: session), subtitle: subtitle)
        }
        func localeLess(_ a: String, _ b: String) -> Bool? {
            let la = a.lowercased(), lb = b.lowercased()
            if la != lb { return la < lb }
            if a != b { return a < b }
            return nil
        }
        return out.enumerated().sorted { x, y in
            let a = x.element, b = y.element
            if a.selectable != b.selectable { return a.selectable }
            if a.device.booted != b.device.booted { return a.device.booted }
            if let r = localeLess(a.device.name, b.device.name) { return r }
            if let r = localeLess(a.device.runtime, b.device.runtime) { return r }
            return x.offset < y.offset
        }.map(\.element)
    }

    /// An open pane always keeps its button; a shell never offers one; otherwise only a mobile project does.
    public static func shouldShowButton(mobile: Bool?, hasPane: Bool, shell: Bool) -> Bool {
        hasPane || (!shell && mobile == true)
    }

    /// `com.apple.CoreSimulator.SimRuntime.iOS-26-2` reads `iOS 26.2`.
    public static func formatRuntime(_ id: String) -> String {
        let prefix = "com.apple.CoreSimulator.SimRuntime."
        let bare = id.hasPrefix(prefix) ? String(id.dropFirst(prefix.count)) : id
        guard let dash = bare.firstIndex(of: "-") else { return bare }
        return "\(bare[..<dash]) \(bare[bare.index(after: dash)...].replacingOccurrences(of: "-", with: "."))"
    }

    /// The iOS simulators in `simctl list devices available -j`'s output.
    public static func parseSimctl(_ json: JSONValue, claims: [String: String]) -> [Device] {
        guard let runtimes = json["devices"]?.objectValue else { return [] }
        var out: [Device] = []
        for runtime in runtimes.keys.sorted() where runtime.contains("iOS") {
            for d in runtimes[runtime]?.arrayValue ?? [] where d["isAvailable"]?.boolValue == true {
                let udid = d["udid"]?.stringValue ?? ""
                out.append(Device(udid: udid, name: d["name"]?.stringValue ?? "", runtime: formatRuntime(runtime),
                                  booted: d["state"]?.stringValue == "Booted", claimedBy: claims[udid]))
            }
        }
        return out
    }

    /// Why a device call failed, said as what to do about it.
    static func explain(_ msg: String) -> String {
        let lower = msg.lowercased()
        if lower.contains("xcode-select") || lower.contains("command line tools") || lower.contains("developerdirectory") {
            return "Xcode is not selected \u{2014} the Command Line Tools are, and they carry no simulators.\nThis needs a password, so you have to run it yourself:\n  sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
        }
        if lower.contains("no runtime") || lower.contains("runtime profile") {
            return "\(msg)\nInstall an iOS runtime: Xcode \u{2192} Settings \u{2192} Components \u{2192} iOS Simulator."
        }
        return msg
    }

    /// Runs `xcrun simctl` off the caller's thread.
    static func simctl(_ args: [String]) async throws -> Data {
        try await Task.detached {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/bin/xcrun")
            p.arguments = ["simctl"] + args
            let out = Pipe(), err = Pipe()
            p.standardOutput = out
            p.standardError = err
            do { try p.run() } catch { throw DeviceError(explain(error.localizedDescription)) }
            let data = out.fileHandleForReading.readDataToEndOfFile()
            let errData = err.fileHandleForReading.readDataToEndOfFile()
            p.waitUntilExit()
            guard p.terminationStatus == 0 else {
                let text = String(decoding: errData, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
                throw DeviceError(explain(text))
            }
            return data
        }.value
    }

    public static func list(claims: [String: String]) async throws -> [Device] {
        let raw = try await simctl(["list", "devices", "available", "-j"])
        guard let json = try? JSONDecoder().decode(JSONValue.self, from: raw) else {
            throw DeviceError("simctl answered with something that is not JSON.")
        }
        return parseSimctl(json, claims: claims)
    }

    /// Takes `udid` for `session`, booting it when it is shut down.
    public static func claim(_ udid: String, session: String, claims: [String: String]) async throws -> Claimed {
        let devices = try await list(claims: claims)
        guard let d = devices.first(where: { $0.udid == udid }) else {
            throw DeviceError("No simulator with udid \(udid). Call device_list to see what is available.")
        }
        guard isSelectable(d, session: session) else {
            throw DeviceError("\(d.name) is in use by session \(d.claimedBy ?? "").")
        }
        if !d.booted {
            _ = try await simctl(["boot", udid])
            _ = try await simctl(["bootstatus", udid, "-b"])
        }
        return Claimed(udid: udid, name: d.name, bootedHere: !d.booted)
    }

    public static func shutdown(_ udid: String) async { _ = try? await simctl(["shutdown", udid]) }

    public static func launch(_ udid: String, bundle: String) async throws {
        do { _ = try await simctl(["launch", "--terminate-running-process", udid, bundle]) } catch let e as DeviceError {
            let lower = e.message.lowercased()
            if ["not installed", "no such", "cannot find", "not found"].contains(where: lower.contains) {
                throw DeviceError("\(bundle) is not installed on this device. Install it first.")
            }
            throw e
        }
    }

    public static func terminate(_ udid: String, bundle: String) async throws { _ = try await simctl(["terminate", udid, bundle]) }
    public static func openURL(_ udid: String, url: String) async throws { _ = try await simctl(["openurl", udid, url]) }
    public static func install(_ udid: String, path: String) async throws { _ = try await simctl(["install", udid, path]) }

    /// A PNG of the simulator's screen.
    public static func screenshot(_ udid: String) async throws -> Data {
        let path = FileManager.default.temporaryDirectory
            .appendingPathComponent("vorn-device-\(ProcessInfo.processInfo.processIdentifier)-\(udid)-\(UUID().uuidString).png")
        _ = try await simctl(["io", udid, "screenshot", "--type=png", path.path])
        defer { try? FileManager.default.removeItem(at: path) }
        return try Data(contentsOf: path)
    }

    public static let zoomMin = 0.25
    public static let zoomMax = 4.0
    static let zoomStep = 1.25

    /// One step in or out, rounded as `toFixed(3)` and clamped.
    public static func steppedZoom(_ current: Double, inward: Bool) -> Double {
        let next = inward ? current * zoomStep : current / zoomStep
        return min(max(toFixed3(next), zoomMin), zoomMax)
    }

    /// `Number(x.toFixed(3))` for a positive `x`, rounded from its exact decimal value.
    static func toFixed3(_ x: Double) -> Double {
        let exact = String(format: "%.60f", x)
        guard let dot = exact.firstIndex(of: ".") else { return x }
        let frac = Array(exact[exact.index(after: dot)...].utf8)
        guard frac.count > 3, let thousandths = Int(exact[..<dot] + String(decoding: frac[0..<3], as: UTF8.self)) else { return x }
        return Double(thousandths + (frac[3] >= UInt8(ascii: "5") ? 1 : 0)) / 1000
    }

    /// Image pixels per point: tall phones with 1000+ pixels across are 3x, every other simulator 2x.
    public static func pointScale(width w: Int, height h: Int) -> Double {
        let short = max(min(w, h), 1), long = max(w, h)
        return short >= 1000 && Double(long) / Double(short) > 1.9 ? 3 : 2
    }

    static let gutter = 8.0
    static let phoneAspect = 1.8

    static func isPhone(_ points: CGSize) -> Bool {
        let long = max(points.width, points.height), short = min(points.width, points.height)
        return short <= 0 || long / short >= phoneAspect
    }

    /// The drawn frame's width, from the pane so it keeps its weight while the device zooms.
    public static func bezelThickness(area: CGSize, points: CGSize) -> Double {
        let k = isPhone(points) ? 0.035 : 0.028
        return min(max((min(area.width, area.height) * k).rounded(), 6), 16)
    }

    /// The scale at which the screen and its frame fit `area`; 1 before either is known.
    public static func fitScale(points: CGSize, area: CGSize) -> Double {
        let inset = 2 * bezelThickness(area: area, points: points) + 2 * gutter
        let w = area.width - inset, h = area.height - inset
        guard points.width > 0, points.height > 0, w > 0, h > 0 else { return 1 }
        return min(w / points.width, h / points.height)
    }

    /// The plain frame's outer corner radius for a screen drawn at `drawn`.
    public static func outerRadius(drawn: CGSize, points: CGSize) -> Double {
        let k = isPhone(points) ? 0.085 : 0.035
        return (min(drawn.width, drawn.height) * k).rounded()
    }

    /// The device's name made safe for a file, and when, as `toISOString` gives it.
    public static func screenshotName(_ name: String, iso: String) -> String {
        let stamp = String(iso.prefix(19)).replacingOccurrences(of: "T", with: " ").replacingOccurrences(of: ":", with: "-")
        var slug = "", gap = false
        for ch in name {
            if ch.isASCII && (ch.isLetter || ch.isNumber) || "_.-".contains(ch) {
                if gap && !slug.isEmpty { slug.append("-") }
                gap = false
                slug.append(ch)
            } else {
                gap = true
            }
        }
        if slug.hasPrefix("-") { slug.removeFirst() }
        if slug.hasSuffix("-") { slug.removeLast() }
        return "\(slug.isEmpty ? "device" : slug) \(stamp).png"
    }

    /// A PNG's width and height, from its header.
    public static func pngSize(_ png: Data) -> (width: Int, height: Int)? {
        let b = [UInt8](png.prefix(24))
        guard b.count >= 24, b[0..<8].elementsEqual([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) else { return nil }
        func be(_ at: Int) -> Int { b[at..<at + 4].reduce(0) { $0 << 8 | Int($1) } }
        return (be(16), be(20))
    }
}

public struct DeviceError: Error, LocalizedError, Sendable {
    public var message: String
    public init(_ message: String) { self.message = message }
    public var errorDescription: String? { message }
}
