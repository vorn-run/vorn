import Foundation
import VornCore

/// WorkflowCanvas.tsx AddableNodeType: what the library and the canvas can add.
enum AddableNodeType: String, CaseIterable, Sendable {
    case agent, script, httpRequest, condition, approval, loop, connectorAction
}

/// library-pick.ts LibraryPick: one row picked in the step library.
enum LibraryPick: Equatable, Sendable {
    case type(AddableNodeType)
    case parallel
    case connectorAction(connectionId: String, action: String, actionLabel: String)
    case triggerType(String)
    case connectorTrigger(connectionId: String, event: String)
    case catalogAction(connectorId: String, action: String, actionLabel: String)
    case httpProfile(profileConnectionId: String, profileName: String)

    /// library-pick.ts stepForPick: the step a pick adds and its preset config; nil for picks that are not steps.
    var step: (type: AddableNodeType, config: [String: JSONValue]?)? {
        switch self {
        case .connectorAction(let connectionId, let action, let actionLabel):
            return (.connectorAction, [
                "connectionId": .string(connectionId), "action": .string(action), "actionLabel": .string(actionLabel),
            ])
        case .catalogAction(let connectorId, let action, let actionLabel):
            return (.connectorAction, [
                "connectionId": "", "connectorId": .string(connectorId), "action": .string(action),
                "actionLabel": .string(actionLabel),
            ])
        case .httpProfile(let id, _): return (.httpRequest, ["profileConnectionId": .string(id)])
        case .type(let type): return (type, nil)
        default: return nil
        }
    }
}

/// WorkflowCanvas.tsx InsertAnchor: where the library's pick will land.
struct InsertAnchor: Equatable, Sendable {
    var afterNodeId: String
    var beforeNodeId: String?
    var insideBranch: Bool = false
    var bodyOnly: Bool = false
    var replaceNodeId: String?

    static let trigger = InsertAnchor(afterNodeId: EditorAnchor.trigger)

    var isTrigger: Bool { afterNodeId == EditorAnchor.trigger }
}
