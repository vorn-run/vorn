import Foundation
import XCTest
@testable import VornCore
@testable import VornTasks

@MainActor
final class TasksStoreTests: XCTestCase {
    private var service: InMemoryTaskService!
    private var store: TasksStore!
    private var toasts: [String] = []

    override func setUp() async throws {
        service = InMemoryTaskService.sample()
        store = TasksStore(service: service)
        toasts = []
        store.onToast = { [weak self] message, _ in self?.toasts.append(message) }
        await store.reload()
    }

    private func id(_ title: String) -> String { store.tasks.first { $0.title == title }!.id }

    /// Waits for the store's queued writes to reach the service.
    private func settle(_ count: Int) async throws {
        for _ in 0..<200 {
            if await service.calls.count >= count { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("expected \(count) calls, saw \(await service.calls)")
    }

    func testFiltersHideArchivedAndScope() async throws {
        XCTAssertEqual(store.visibleTasks.count, 10)
        store.scope = .project("website")
        XCTAssertEqual(store.visibleTasks.map(\.title), ["Landing page hero copy"])
        store.scope = .all
        store.statusFilter = .status(.done)
        XCTAssertEqual(store.visibleTasks.count, 2)
        XCTAssertTrue(store.hasActiveFilters)

        store.statusFilter = .all
        store.archive(id("Theme tokens from theme.css"))
        XCTAssertEqual(store.visibleTasks.count, 9)
        store.includeArchived = true
        XCTAssertEqual(store.visibleTasks.count, 10)
    }

    func testDropRules() async throws {
        let todo = id("Port the task board to the native app")
        let review = id("Review the workflow runs panel")

        store.drop(taskId: review, on: .inProgress)
        XCTAssertEqual(store.task(id: review)?.status, .inReview, "only todo tasks may start")

        store.drop(taskId: todo, on: .inProgress)
        XCTAssertEqual(store.task(id: todo)?.status, .inProgress)

        store.drop(taskId: todo, on: .done)
        XCTAssertEqual(store.task(id: todo)?.status, .done)
        XCTAssertNotNil(store.task(id: todo)?.completedAt)
        XCTAssertEqual(toasts.last, "Task completed")

        store.drop(taskId: todo, on: .todo)
        XCTAssertNil(store.task(id: todo)?.completedAt)
        XCTAssertEqual(toasts.count, 1, "reopening by drag is silent")

        try await settle(3)
        let stored = await service.tasks.first { $0.id == todo }
        XCTAssertEqual(stored?.status, .todo)
    }

    func testMoveReordersWithinTheColumn() async throws {
        let first = id("Port the task board to the native app")
        let third = id("Archive tasks older than thirty days automatically")
        store.move(taskId: third, before: first)
        XCTAssertEqual(store.tasks(in: .todo, sortedByOrder: true).filter { $0.projectName == "vorn" }.map(\.title), [
            "Archive tasks older than thirty days automatically",
            "Port the task board to the native app",
            "Keyboard navigation between columns",
        ])
        try await settle(1)
        let calls = await service.calls
        XCTAssertEqual(calls, ["reorder"])
        let stored = await service.tasks.filter { $0.status == .todo && $0.projectName == "vorn" }
            .sorted { $0.order < $1.order }.map(\.id)
        XCTAssertEqual(stored.first, third)
    }

    func testMoveOntoAnotherColumnChangesStatus() async throws {
        let todo = id("Keyboard navigation between columns")
        store.move(taskId: todo, before: id("Theme tokens from theme.css"))
        XCTAssertEqual(store.task(id: todo)?.status, .done)
    }

    func testArchiveOnlyFinishedTasks() async throws {
        let open = id("Port the task board to the native app")
        store.archive(open)
        XCTAssertFalse(store.task(id: open)?.isArchived ?? true)

        let done = id("Lucide glyphs as SwiftUI paths")
        store.archive(done)
        XCTAssertTrue(store.task(id: done)?.isArchived ?? false)
        store.unarchive(done)
        XCTAssertFalse(store.task(id: done)?.isArchived ?? true)
        try await settle(2)
        let calls = await service.calls
        XCTAssertEqual(calls, ["archive", "unarchive"])
    }

    func testUpdateClearsAgent() async throws {
        let t = id("Port the task board to the native app")
        var patch = TaskPatch(title: "Port the board")
        patch.clearsAgent = true
        store.update(t, patch, toast: "Task updated")
        XCTAssertNil(store.task(id: t)?.assignedAgent)
        XCTAssertEqual(store.task(id: t)?.title, "Port the board")
        try await settle(1)
        let stored = await service.tasks.first { $0.id == t }
        XCTAssertNil(stored?.assignedAgent)
        XCTAssertEqual(stored?.title, "Port the board")
    }

    func testCreateAndDelete() async throws {
        let newId = await store.create(TaskDraft(projectName: "vorn", title: "Fresh"))
        XCTAssertNotNil(newId)
        XCTAssertTrue(store.tasks.contains { $0.id == newId })
        XCTAssertEqual(toasts.last, "Task created")

        store.selectedTaskId = newId
        store.delete(newId!)
        XCTAssertNil(store.selectedTaskId)
        XCTAssertFalse(store.tasks.contains { $0.id == newId })
    }

    func testRefusedWriteRestoresTheBoard() async throws {
        let failed = await store.create(TaskDraft(projectName: "nowhere", title: "X"))
        XCTAssertNil(failed)
        XCTAssertEqual(store.tasks.count, 10)
        XCTAssertNotNil(store.lastError)
    }

    func testLocalViewModeSurvivesReload() async throws {
        store.showViewMode(.kanban)
        await store.reload()
        XCTAssertEqual(store.viewMode, .kanban)
        let calls = await service.calls
        XCTAssertTrue(calls.isEmpty, "a local view mode is never saved")

        store.setViewMode(.list)
        try await settle(1)
        let saved = await service.calls
        XCTAssertEqual(saved, ["viewMode"])
    }
}
