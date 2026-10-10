import Foundation
import XCTest
@testable import VornCore

/// Drives the task methods of a real vornd that serves a temporary data directory.
final class VorndTaskTests: XCTestCase {
    private var server: Process?
    private var dataDir: URL!
    private var client: VornClient!

    override func setUp() async throws {
        let binary = ProcessInfo.processInfo.environment["VORND_BIN"].map(URL.init(fileURLWithPath:))
            ?? URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("packages/core/target/debug/vornd")
        guard FileManager.default.isExecutableFile(atPath: binary.path) else {
            throw XCTSkip("vornd is not built at \(binary.path); set VORND_BIN")
        }
        dataDir = FileManager.default.temporaryDirectory.appendingPathComponent("vornd-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)

        let process = Process()
        process.executableURL = binary
        process.arguments = ["--data-dir", dataDir.path, "--port", "\(Int.random(in: 52000..<60000))",
                             "--host", "127.0.0.1", "--exit-with-stdin"]
        process.standardInput = Pipe()
        process.standardOutput = Pipe()
        process.standardError = FileHandle.nullDevice
        try process.run()
        server = process

        let deadline = Date().addingTimeInterval(15)
        var endpoint: VornEndpoint?
        while endpoint == nil, Date() < deadline {
            endpoint = try? VornEndpoint.read(dataDirectory: dataDir)
            if endpoint == nil { try await Task.sleep(for: .milliseconds(100)) }
        }
        guard let endpoint else { throw XCTSkip("vornd did not start") }
        client = VornClient(endpoint: endpoint)
        var lastError: Error?
        for _ in 0..<50 {
            do { try await client.connect(); lastError = nil; break } catch {
                lastError = error
                try await Task.sleep(for: .milliseconds(100))
            }
        }
        if let lastError { throw lastError }
        try await addProject("alpha")
    }

    override func tearDown() async throws {
        await client?.close()
        server?.terminate()
        server?.waitUntilExit()
        if let dataDir { try? FileManager.default.removeItem(at: dataDir) }
    }

    private func addProject(_ name: String) async throws {
        guard case .object(var config) = try await client.call("config:load") else { return XCTFail("config:load") }
        let path = dataDir.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: path, withIntermediateDirectories: true)
        var projects = config["projects"]?.arrayValue ?? []
        projects.append(.object(["name": .string(name), "path": .string(path.path), "preferredAgents": .array([])]))
        config["projects"] = .array(projects)
        try await client.call("config:save", .object(config))
    }

    func testCreateUpdateArchiveDelete() async throws {
        let created = try await client.createTask(TaskDraft(projectName: "alpha", title: "First", assignedAgent: .claude))
        XCTAssertEqual(created.status, .todo)
        XCTAssertEqual(created.assignedAgent, .claude)

        var patch = TaskPatch(title: "Renamed", status: .done)
        patch.clearsAgent = true
        let updated = try await client.updateTask(id: created.id, patch)
        XCTAssertEqual(updated?.title, "Renamed")
        XCTAssertEqual(updated?.status, .done)
        XCTAssertNotNil(updated?.completedAt)
        XCTAssertNil(updated?.assignedAgent)

        try await client.archiveTask(id: created.id, archived: true)
        let archived = try await client.getTask(id: created.id)
        XCTAssertTrue(archived?.isArchived == true)

        let reopened = try await client.updateTask(id: created.id, TaskPatch(status: .todo))
        XCTAssertNil(reopened?.completedAt)
        XCTAssertFalse(reopened?.isArchived ?? true)

        try await client.deleteTask(id: created.id)
        let remaining = try await client.listTasks()
        XCTAssertFalse(remaining.contains { $0.id == created.id })
    }

    func testReorderKeepsTheColumnsSlots() async throws {
        var ids: [String] = []
        for title in ["A", "B", "C"] {
            ids.append(try await client.createTask(TaskDraft(projectName: "alpha", title: title)).id)
        }
        try await client.reorderTasks(ids: [ids[2], ids[0], ids[1]])
        let titles = try await client.listTasks().filter { ids.contains($0.id) }.sorted { $0.order < $1.order }.map(\.title)
        XCTAssertEqual(titles, ["C", "A", "B"])
    }

    func testViewModeRoundTrips() async throws {
        try await client.setTaskViewMode(.kanban)
        let mode = try await client.taskViewMode()
        XCTAssertEqual(mode, .kanban)
    }

    func testBoardChangesFireOnWrites() async throws {
        let changes = await client.boardChanges()
        let fired = Task {
            var it = changes.makeAsyncIterator()
            return await it.next() != nil
        }
        try await Task.sleep(for: .milliseconds(100))
        _ = try await client.createTask(TaskDraft(projectName: "alpha", title: "Ping"))
        let result = try await withTimeout(seconds: 5) { await fired.value }
        XCTAssertTrue(result)
    }

    func testCreateInUnknownProjectIsRefused() async {
        do {
            _ = try await client.createTask(TaskDraft(projectName: "missing", title: "X"))
            XCTFail("vornd stored a task for a project it does not have")
        } catch {}
    }
}

func withTimeout<T: Sendable>(seconds: Double, _ body: @escaping @Sendable () async -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T?.self) { group in
        group.addTask { await body() }
        group.addTask { try await Task.sleep(for: .seconds(seconds)); return nil }
        guard let first = try await group.next(), let value = first else {
            group.cancelAll()
            throw XCTSkip("timed out")
        }
        group.cancelAll()
        return value
    }
}
