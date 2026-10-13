import Foundation
import Testing
import VornCore
@testable import VornSettings

/// The settings pages' writes and reads against a throwaway vornd.
@MainActor
struct SettingsLiveTests {
    private func fresh(_ server: TestVornd) async throws -> [String: JSONValue] {
        let client = VornClient(dataDir: server.dataDir)
        defer { Task { await client.close() } }
        try await client.connect()
        return try #require(try await client.call("config:load").objectValue)
    }

    @Test func pagesSaveToTheConfig() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let client = VornClient(dataDir: server.dataDir)
        #expect(await liveUntil { (try? await client.connect()) != nil })
        let model = SettingsModel(client: client)
        await model.load()
        #expect(model.config != nil)

        // Appearance and General write `defaults` keys.
        model.updateDefaults(["fontSize": 15, "layoutMode": "tabs", "taskViewMode": "kanban", "rowHeight": 320])
        model.updateDefaults(["startAtLogin": true, "headlessStepTimeoutMinutes": 120, "shell": "/bin/bash"])
        // Notifications keep their object whole.
        model.updateDefaults(["notifications": ["enabled": true, "onWaiting": false, "onError": true, "onBell": true, "soundEnabled": true]])
        await model.flush()
        var saved = try await fresh(server)
        let d = try #require(saved["defaults"]?.objectValue)
        #expect(d["fontSize"]?.numberValue == 15)
        #expect(d["layoutMode"]?.stringValue == "tabs")
        #expect(d["taskViewMode"]?.stringValue == "kanban")
        #expect(d["rowHeight"]?.numberValue == 320)
        #expect(d["startAtLogin"]?.boolValue == true)
        #expect(d["headlessStepTimeoutMinutes"]?.numberValue == 120)
        #expect(d["shell"]?.stringValue == "/bin/bash")
        #expect(d["notifications"]?["onWaiting"]?.boolValue == false)
        #expect(d["notifications"]?["soundEnabled"]?.boolValue == true)

        // Coding Agents: a changed command is saved beside the others' defaults.
        let claude = try #require(AgentDefinition.of(.claude))
        var all: [String: JSONValue] = [:]
        for def in AgentDefinition.all { all[def.type.rawValue] = def.defaultCommand }
        all["claude"] = .object(["command": "claude-dev", "args": ["--verbose"], "headlessArgs": .array(claude.headlessArgs.map { .string($0) })])
        model.updateConfig(["agentCommands": .object(all)])
        // SSH & Hosts: a host row lives in `remoteHosts`.
        model.updateConfig(["remoteHosts": [["id": "h1", "label": "Box", "hostname": "box.example.com", "user": "ci", "port": 2222, "authMethod": "agent"]]])
        await model.flush()
        saved = try await fresh(server)
        #expect(saved["agentCommands"]?["claude"]?["command"]?.stringValue == "claude-dev")
        #expect(saved["agentCommands"]?["codex"]?["headlessArgs"] == ["-a", "never"])
        #expect(saved["remoteHosts"]?.arrayValue?.first?["hostname"]?.stringValue == "box.example.com")

        // Reads each page makes.
        await model.detectAgents()
        #expect(model.installedAgents?.keys.contains("claude") == true)
        await model.loadShells()
        #expect(model.shells?.contains { $0.path == "/bin/zsh" } == true)
        await model.loadSSHKeys()
        #expect(model.sshKeys.isEmpty)
        await model.loadConnectorKeys()
        #expect(model.connectorKeysLoaded && model.connectorKeys.isEmpty)
        await model.loadCore()
        #expect(model.core?.version?.isEmpty == false)
        #expect(model.holders?.current != nil)
        let test = await model.testSSH(["id": "h1", "label": "Box", "hostname": "127.0.0.1", "user": "nobody", "port": 1, "authMethod": "agent"])
        #expect(!test.ok)
        await client.close()
    }

    @Test func worktreesPageRemovesWhatItSelects() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let client = VornClient(dataDir: server.dataDir)
        #expect(await liveUntil { (try? await client.connect()) != nil })
        let repo = server.root.appendingPathComponent("repo")
        try makeGitRepo(at: repo)
        _ = try await client.call("config:save", .object({
            var c = try! #require(try await client.call("config:load").objectValue)
            c["projects"] = [["name": "repo", "path": .string(repo.path), "preferredAgents": []]]
            return c
        }()))
        _ = try await client.call("git:createWorktree", ["projectPath": .string(repo.path), "branch": "merged-work", "worktreeName": "merged-work"])

        let model = SettingsModel(client: client)
        await model.wtLoad(refresh: true)
        let entry = try #require(model.wtEntries.first { $0.name == "merged-work" })
        #expect(entry.isSelectable)
        model.wtSelected = [entry.path]
        model.wtRemoveSelected()
        #expect(await liveUntil { !model.wtBusy && model.wtEntries.allSatisfy { $0.name != "merged-work" } && !model.inventoryLoading })
        #expect(!FileManager.default.fileExists(atPath: entry.path))
        #expect(model.wtSelected.isEmpty)
        await client.close()
    }
}
