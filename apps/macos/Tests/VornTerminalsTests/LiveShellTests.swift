import Foundation
import Testing
import VornCore
@testable import VornTerminals

/// Opens a shell on a test vornd and types into it. Runs only when
/// `VORN_TEST_DATA_DIR` names a scratch vornd's data dir, never the real one.
@MainActor
@Suite(.enabled(if: ProcessInfo.processInfo.environment["VORN_TEST_DATA_DIR"] != nil))
struct LiveShellTests {
    @Test func typesIntoANewShell() async throws {
        let dir = URL(fileURLWithPath: ProcessInfo.processInfo.environment["VORN_TEST_DATA_DIR"]!)
        #expect(dir.standardizedFileURL != VornEndpoint.defaultDataDir(environment: [:]).standardizedFileURL)
        let store = VornStore(dataDir: dir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await until { store.phase == .connected })

        let shell = try await store.createShell(cwd: NSTemporaryDirectory())
        let engine = TerminalEngine(socket: try #require(store.endpoint).gridSocket, readOnly: false)
        defer { engine.shutdown() }
        let pane = engine.makePane(sessionID: shell.id)
        pane.attach(cols: 80, rows: 24)
        await engine.settle(timeout: 8)
        #expect(pane.view != nil, "state \(pane.state) connected \(engine.isConnected) error \(engine.lastError ?? "-") socket \(engine.socket)")

        let marker = "native-\(Int.random(in: 1000...9999))"
        pane.text("echo \(marker)-ok\r")
        let echoed = await until(timeout: 10) {
            await engine.settle(timeout: 0.1)
            pane.contentChanged()
            return pane.screenText().contains("\(marker)-ok\n") || pane.screenText().components(separatedBy: "\(marker)-ok").count > 2
        }
        try await store.close(shell.id)
        #expect(echoed, "screen: \(pane.screenText())")
    }

    @discardableResult
    private func until(timeout: TimeInterval = 8, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}
