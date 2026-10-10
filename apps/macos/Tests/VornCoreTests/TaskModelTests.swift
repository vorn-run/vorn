import Foundation
import XCTest
@testable import VornCore

final class TaskModelTests: XCTestCase {
    func testDecodesLeniently() throws {
        let json = #"{"id":"ab12cd34","projectName":"vorn","title":"T","status":"in_progress","order":3,"assignedAgent":"","createdAt":"2025-10-09T10:00:00.000Z"}"#
        let task = try JSONDecoder().decode(VornTask.self, from: Data(json.utf8))
        XCTAssertEqual(task.status, .inProgress)
        XCTAssertEqual(task.order, 3)
        XCTAssertNil(task.assignedAgent)
        XCTAssertEqual(task.description, "")
        XCTAssertEqual(task.updatedAt, task.createdAt)
        XCTAssertFalse(task.isArchived)
    }

    func testUnknownStatusFallsBackToTodo() throws {
        let task = try JSONDecoder().decode(VornTask.self, from: Data(#"{"id":"x","status":"later"}"#.utf8))
        XCTAssertEqual(task.status, .todo)
    }

    func testShortId() {
        XCTAssertEqual(VornTask(id: "a1f3c2d0", projectName: "vorn", title: "", createdAt: "").shortId, "VOR-A1F3")
        XCTAssertEqual(VornTask(id: "a1f3c2d0", projectName: "my-app", title: "", createdAt: "").shortId, "MYA-A1F3")
        XCTAssertEqual(VornTask(id: "a1f3c2d0", projectName: "42", title: "", createdAt: "").shortId, "TSK-A1F3")
    }

    func testPatchOmitsUnsetFields() throws {
        let object = try encoded(TaskPatch(title: "New"))
        XCTAssertEqual(object as NSDictionary, ["title": "New"] as NSDictionary)
    }

    func testPatchClearsAgentWithEmptyString() throws {
        var patch = TaskPatch()
        patch.clearsAgent = true
        XCTAssertEqual(try encoded(patch) as NSDictionary, ["assignedAgent": ""] as NSDictionary)
        patch.assignedAgent = .codex
        XCTAssertEqual(try encoded(patch) as NSDictionary, ["assignedAgent": "codex"] as NSDictionary)
    }

    func testViewModeFromConfig() {
        let config = JSONValue.object(["defaults": .object(["taskViewMode": .string("kanban")])])
        XCTAssertEqual(VornClient.viewMode(in: config), .kanban)
        XCTAssertEqual(VornClient.viewMode(in: .object([:])), .list)
    }

    func testReadOnlyClientRefusesWritesBeforeSending() async {
        let client = VornClient(endpoint: VornEndpoint(port: 1, token: "t"), readOnly: true)
        do {
            try await client.call("task:delete", .object(["id": .string("x")]))
            XCTFail("a write went through a read-only client")
        } catch let error as VornError {
            guard case .refused = error else { return XCTFail("\(error)") }
        } catch {
            XCTFail("\(error)")
        }
    }

    func testEndpointReadsPortAndToken() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try #"{"port":51234,"pid":1}"#.write(to: dir.appendingPathComponent("ws-port"), atomically: true, encoding: .utf8)
        try "secret\n".write(to: dir.appendingPathComponent("local-token"), atomically: true, encoding: .utf8)
        let endpoint = try VornEndpoint.read(dataDirectory: dir)
        XCTAssertEqual(endpoint.port, 51234)
        XCTAssertEqual(endpoint.token, "secret")
    }

    private func encoded(_ patch: TaskPatch) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(patch)) as? [String: Any] ?? [:]
    }
}
