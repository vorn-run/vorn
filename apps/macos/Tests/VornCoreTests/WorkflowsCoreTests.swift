import Foundation
import Testing
@testable import VornCore

@Suite struct EndpointTests {
    @Test func parsesTheJSONPortFile() {
        let parsed = VornEndpoint.parsePort(Data(#"{"port":50991,"pid":1}"#.utf8))
        #expect(parsed?.port == 50991)
        #expect(parsed?.pid == 1)
    }

    @Test func parsesABarePort() {
        #expect(VornEndpoint.parsePort(Data("4242\n".utf8))?.port == 4242)
    }

    @Test func rejectsOutOfRangeOrGarbage() {
        #expect(VornEndpoint.parsePort(Data(#"{"port":0}"#.utf8)) == nil)
        #expect(VornEndpoint.parsePort(Data("70000".utf8)) == nil)
        #expect(VornEndpoint.parsePort(Data("nope".utf8)) == nil)
    }

    @Test func buildsTheSocketURL() {
        #expect(VornEndpoint(port: 9, token: "t").webSocketURL.absoluteString == "ws://127.0.0.1:9/ws")
    }

    @Test func discoveryNamesTheMissingFile() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        #expect(throws: VornEndpoint.DiscoveryError.missing(dir.appendingPathComponent("ws-port").path)) {
            try VornEndpoint.discover(dataDir: dir)
        }
        try Data(#"{"port":1234}"#.utf8).write(to: dir.appendingPathComponent("ws-port"))
        try Data("secret\n".utf8).write(to: dir.appendingPathComponent("local-token"))
        let endpoint = try VornEndpoint.discover(dataDir: dir)
        #expect(endpoint.token == "secret")
        #expect(endpoint.webSocketURL.port == 1234)
    }
}

@Suite struct ReadOnlyTests {
    @Test func allowsQueriesAndSubscriptions() {
        for method in ["auth:authenticate", "subscribe:set", "workflow:list", "workflow:listAllRuns", "workflow:getRun"] {
            #expect(VornClient.isReadOnly(method), "\(method)")
        }
    }

    @Test func refusesWrites() {
        for method in ["workflow:run", "workflow:setEnabled", "workflow:delete", "workflow:resolveGate", "workflow:stop", "listless"] {
            #expect(!VornClient.isReadOnly(method), "\(method)")
        }
    }
}

@Suite struct ModelDecodingTests {
    @Test func decodesAWorkflow() throws {
        let json = #"""
        {"id":"w","name":"N","icon":"Zap","iconColor":"#fff","enabled":true,"createdAt":"x",
         "nodes":[{"id":"t","type":"trigger","label":"Trigger","config":{"triggerType":"recurring","cron":"* * * * *"},
                   "position":{"x":0,"y":0}}],
         "edges":[]}
        """#
        let wf = try JSONDecoder().decode(WorkflowDefinition.self, from: Data(json.utf8))
        #expect(wf.workspace == "personal")
        #expect(wf.isScheduled)
        #expect(wf.triggerNode?.triggerType == "recurring")
    }

    @Test func decodesARunWaitingOnSignIn() throws {
        let json = #"""
        {"runId":"r","workflowId":"w","startedAt":"2026-01-01T00:00:00Z","status":"running",
         "nodeStates":[{"nodeId":"a","status":"waiting","waitingFor":"signIn"},{"nodeId":"b","status":"pending"}]}
        """#
        let run = try JSONDecoder().decode(WorkflowExecution.self, from: Data(json.utf8))
        #expect(run.waitingStep?.isSignInWait == true)
        #expect(run.state(of: "b")?.status == .pending)
    }
}
