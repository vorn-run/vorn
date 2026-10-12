import AppKit
import Foundation
import UniformTypeIdentifiers
import VornCore

/// workflow-files.ts: a workflow as a file, on the way out and on the way back.
public enum WorkflowFiles {
    /// Names the file for what it is, so a directory of them reads.
    public static let suffix = ".vorn-workflow.json"
    /// As much text as a workflow can reasonably be, counted as JavaScript counts a string.
    public static let maxBytes = 500_000

    /// The project a workflow's paths are relative to: its own steps answer first, then the project in view.
    public static func projectForWorkflow(
        _ workflow: WorkflowDefinition, projects: [Project], activeProject: String?
    ) -> Project? {
        let named = workflow.nodes.lazy.compactMap { $0.config["projectName"]?.stringValue }.first { !$0.isEmpty }
        return projects.first { $0.name == named } ?? projects.first { $0.name == activeProject }
    }

    public static func fileName(_ workflow: WorkflowDefinition) -> String {
        "\(WorkflowPortability.slugify(workflow.name))\(suffix)"
    }

    /// The bytes to write, plus whatever refused to become portable.
    public static func fileFromWorkflow(
        _ workflow: WorkflowDefinition, projectPath: String, connections: [PortableConnection]
    ) -> (name: String, contents: String, residual: [String]) {
        file(workflow.portableJSON, name: fileName(workflow), projectPath: projectPath, connections: connections)
    }

    static func file(
        _ workflow: OObject, name: String, projectPath: String, connections: [PortableConnection]
    ) -> (name: String, contents: String, residual: [String]) {
        let portable = WorkflowPortability.toPortable(workflow, projectPath: projectPath, connections: connections)
        return (name, OJSON.object(portable).stringify(indent: 2) + "\n", WorkflowPortability.residualAbsolutePaths(portable))
    }

    /// A file read back into a workflow for this machine, or why it cannot be.
    public struct Imported: Sendable {
        public var definition: WorkflowDefinition
        public var unresolved: [PortableRequirement]
        /// The definition as the file carried it, so nothing this build does not model is lost on the way to vornd.
        var document: OObject
    }

    public static func definitionFromFile(
        _ text: String, project: (name: String, path: String), bundle: String,
        connections: [PortableConnection], existingWorkflows: [(id: String, name: String)] = []
    ) -> Result<Imported, ImportError> {
        if text.utf16.count > maxBytes { return .failure(ImportError("That file is too large to be a workflow")) }
        guard let parsed = try? OJSON.parse(text) else { return .failure(ImportError("That file is not valid JSON")) }
        let object = parsed.objectValue ?? OObject()
        if object["version"] != .number(Double(WorkflowPortability.formatVersion)) {
            let version = object["version"].flatMap { $0 == .null ? nil : WorkflowPortability.jsString($0) } ?? "unknown"
            return .failure(ImportError(
                "That file is version \(version); this build reads version \(WorkflowPortability.formatVersion)"))
        }
        guard WorkflowPortability.truthy(object["name"]), let nodes = object["nodes"]?.arrayValue,
              let edges = object["edges"]?.arrayValue
        else { return .failure(ImportError("That file is missing a name, nodes or edges")) }
        if let complaint = graphComplaint(nodes: nodes, edges: edges) { return .failure(ImportError(complaint)) }

        var portable = object
        if portable["slug"] == nil || portable["slug"] == .null {
            portable["slug"] = .string(WorkflowPortability.slugify(WorkflowPortability.jsString(object["name"])))
        }
        let slug = WorkflowPortability.jsString(portable["slug"])
        let name = WorkflowPortability.jsString(portable["name"])
        var document = WorkflowPortability.fromPortable(portable, bundle: bundle, project: project, connections: connections)
        document["id"] = .string(
            WorkflowPortability.importedWorkflowIdFor(bundle: bundle, slug: slug, name: name, existing: existingWorkflows))
        guard let definition = WorkflowDefinition(portableJSON: document) else {
            return .failure(ImportError("That file has a step this build cannot read"))
        }
        return .success(Imported(
            definition: definition,
            unresolved: WorkflowPortability.unresolvedRequirements(portable, connections: connections),
            document: document))
    }

    public struct ImportError: Error, Equatable, Sendable, CustomStringConvertible {
        public var message: String
        public init(_ message: String) { self.message = message }
        public var description: String { message }
    }

    /// What is wrong with the graph, if anything: an array of things is not yet a graph.
    static func graphComplaint(nodes: [OJSON], edges: [OJSON]) -> String? {
        var ids = Set<String>()
        for node in nodes {
            guard let id = node.objectValue?["id"]?.textValue, node.objectValue?["type"]?.textValue != nil else {
                return "That file has a step this build cannot read"
            }
            if !ids.insert(id).inserted { return "That file carries the same step twice" }
        }
        for edge in edges {
            guard let source = edge.objectValue?["source"]?.textValue, let target = edge.objectValue?["target"]?.textValue
            else { return "That file has a connection this build cannot read" }
            if !ids.contains(source) || !ids.contains(target) { return "That file connects a step it does not carry" }
        }
        return nil
    }

    /// Keep a re-imported workflow where it already lived, and as it was left; a first arrival stays off.
    public static func placeImportedWorkflow(
        _ definition: WorkflowDefinition, existing: WorkflowDefinition?, activeWorkspace: String
    ) -> WorkflowDefinition {
        var placed = definition
        placed.enabled = existing?.enabled ?? definition.enabled
        placed.workspaceId = existing?.workspaceId ?? activeWorkspace
        return placed
    }

    static func place(_ document: OObject, existing: WorkflowDefinition?, activeWorkspace: String) -> OObject {
        var placed = document
        if let existing { placed["enabled"] = .bool(existing.enabled) }
        placed["workspaceId"] = .string(existing?.workspaceId ?? activeWorkspace)
        return placed
    }

    /// What an unbound step is still waiting for, in a sentence a person can act on.
    public static func describeRequirement(_ requirement: PortableRequirement) -> String {
        if requirement.kind == .httpProfile {
            return requirement.name.isEmpty ? "an HTTP profile" : "an HTTP profile like \"\(requirement.name)\""
        }
        let connector = requirement.connectorId.isEmpty ? "connector" : requirement.connectorId
        if requirement.key == "secretsFrom" {
            return "a \(connector) key for its Secrets from\(requirement.name.isEmpty ? "" : " (it used \"\(requirement.name)\")")"
        }
        return requirement.name.isEmpty
            ? "a \(connector) connection" : "a \(connector) connection like \"\(requirement.name)\""
    }

    /// The first workflow file in a drop, refused by size before it is read.
    public static func readDroppedWorkflowFile(_ urls: [URL]) throws -> PickedWorkflowFile? {
        guard let url = urls.first(where: { $0.lastPathComponent.lowercased().hasSuffix(".json") }) else { return nil }
        return try read(url)
    }

    static func read(_ url: URL) throws -> PickedWorkflowFile {
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        if size > maxBytes { throw ImportError("That file is too large to be a workflow") }
        let data = try Data(contentsOf: url)
        return PickedWorkflowFile(name: url.lastPathComponent, contents: String(decoding: data, as: UTF8.self))
    }
}

public struct PickedWorkflowFile: Sendable, Equatable {
    public var name: String
    public var contents: String
    public init(name: String, contents: String) {
        self.name = name
        self.contents = contents
    }
}

/// The file dialogs around an export or import, swappable so tests and renders never show one.
public struct WorkflowFileIO: Sendable {
    /// Writes `contents` where the person chooses; false when they cancel.
    public var save: @MainActor @Sendable (_ defaultName: String, _ contents: String, _ title: String) async throws -> Bool
    /// The file the person picks, or nil when they cancel.
    public var pick: @MainActor @Sendable () async throws -> PickedWorkflowFile?

    public init(
        save: @escaping @MainActor @Sendable (String, String, String) async throws -> Bool,
        pick: @escaping @MainActor @Sendable () async throws -> PickedWorkflowFile?
    ) {
        self.save = save
        self.pick = pick
    }

    /// NSSavePanel and NSOpenPanel, as a sheet on the key window when there is one.
    public static let panels = WorkflowFileIO(
        save: { name, contents, title in
            let panel = NSSavePanel()
            panel.nameFieldStringValue = name
            panel.title = title
            panel.allowedContentTypes = [.json]
            guard await run(panel) == .OK, let url = panel.url else { return false }
            try contents.write(to: url, atomically: true, encoding: .utf8)
            return true
        },
        pick: {
            let panel = NSOpenPanel()
            panel.allowedContentTypes = [.json]
            panel.allowsMultipleSelection = false
            panel.canChooseDirectories = false
            guard await run(panel) == .OK, let url = panel.url else { return nil }
            return try WorkflowFiles.read(url)
        })

    /// Never shows anything: saves succeed into nowhere and picks come back empty.
    public static let none = WorkflowFileIO(save: { _, _, _ in false }, pick: { nil })

    @MainActor private static func run(_ panel: NSSavePanel) async -> NSApplication.ModalResponse {
        guard let window = NSApp.keyWindow else { return panel.runModal() }
        return await panel.beginSheetModal(for: window)
    }
}

/// The import's unbound steps, handed to the editor to show where the steps are.
public struct ImportedRequirements: Sendable, Equatable {
    public var workflowId: String
    public var requirements: [PortableRequirement]
}

/// WorkflowsSection.tsx's export and import handlers.
extension WorkflowsStore {
    /// Export as file…: writes the workflow as a portable file the user places.
    public func exportWorkflow(_ workflow: WorkflowDefinition) {
        perform { [self] in
            let project = WorkflowFiles.projectForWorkflow(workflow, projects: projects, activeProject: activeProjectName)
            let file = WorkflowFiles.fileFromWorkflow(
                workflow, projectPath: project?.path ?? "", connections: await portableConnections())
            guard try await workflowFiles.save(file.name, file.contents, "Export workflow") else { return }
            if file.residual.isEmpty {
                notify("Exported \"\(workflow.name)\"", tone: .success)
            } else {
                notify("Exported, but these still name this machine: \(file.residual.joined(separator: ", "))")
            }
        }
    }

    /// The sidebar's import: asks for a file and adds it to the workspace.
    public func importWorkflow() {
        perform { [self] in
            guard let picked = try await workflowFiles.pick() else { return }
            try await importWorkflowFile(picked)
        }
    }

    /// A drop on the sidebar's list: the first JSON file in it, or a reason it is not one.
    public func importDroppedWorkflow(_ urls: [URL]) {
        perform { [self] in
            guard let picked = try WorkflowFiles.readDroppedWorkflowFile(urls) else {
                toast = "That is not a workflow file"
                return
            }
            try await importWorkflowFile(picked)
        }
    }

    func importWorkflowFile(_ picked: PickedWorkflowFile) async throws {
        guard let project = projects.first(where: { $0.name == activeProjectName }) ?? projects.first else {
            toast = "Add a project first, so the imported paths have somewhere to resolve"
            return
        }
        let result = WorkflowFiles.definitionFromFile(
            picked.contents, project: (project.name, project.path), bundle: WorkflowPortability.slugify(project.name),
            connections: await portableConnections(), existingWorkflows: workflows.map { ($0.id, $0.name) })
        let imported: WorkflowFiles.Imported
        switch result {
        case .failure(let error):
            toast = error.message
            return
        case .success(let value):
            imported = value
        }

        let existing = workflow(imported.definition.id)
        let placed = WorkflowFiles.place(imported.document, existing: existing, activeWorkspace: workspaceId)
        if let existing {
            _ = try await backend.rpc("workflow:update", .object(["id": .string(existing.id), "updates": OJSON.object(placed).unordered]))
        } else {
            _ = try await backend.rpc("workflow:create", .object(["workflow": OJSON.object(placed).unordered]))
        }
        await reload()

        let pending = imported.unresolved.map(WorkflowFiles.describeRequirement).joined(separator: ", ")
        let note = !pending.isEmpty ? " — still needs \(pending)" : existing != nil ? "" : " — enable it when ready"
        let headline = "\(existing != nil ? "Updated" : "Imported") \"\(imported.definition.name)\"\(note)"
        guard !imported.unresolved.isEmpty else { return notify(headline, tone: .success) }
        // What it still needs is best answered where the steps are.
        let id = imported.definition.id, requirements = imported.unresolved
        notify(headline, tone: .success, action: ToastAction(label: "Review") { [weak self] in
            self?.openImported(ImportedRequirements(workflowId: id, requirements: requirements))
        })
    }

    /// The toast's Review: the imported workflow's editor, its unbound steps listed.
    func openImported(_ imported: ImportedRequirements) {
        openWorkflow(imported.workflowId)
        importedRequirements = imported
    }

    /// This machine's connections, enough to bind or name them; none when vornd cannot list them.
    func portableConnections() async -> [PortableConnection] {
        (try? await backend.rpc("connection:list", .object([:]), as: [PortableConnection].self)) ?? []
    }
}
