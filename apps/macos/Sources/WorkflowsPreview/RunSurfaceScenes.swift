import AppKit
import SwiftUI
import VornCore
import VornWorkflows

/// A contextual workflow whose manual trigger declares one input of every kind.
func sampleInputsWorkflow() -> WorkflowDefinition {
    let inputs: JSONValue = [
        ["key": "goal", "label": "Goal", "type": "text", "required": true, "placeholder": "What should it do?"],
        ["key": "notes", "label": "Notes", "type": "textarea", "description": "Anything the agent should know."],
        ["key": "retries", "label": "Retries", "type": "number", "defaultValue": "2"],
        ["key": "env", "label": "Environment", "type": "select", "options": [
            ["value": "staging", "label": "Staging"], ["value": "prod", "label": "Production"],
        ]],
        ["key": "repo", "label": "Repository", "type": "project"],
        ["key": "base", "label": "Base branch", "type": "branch"],
        ["key": "dry", "label": "Dry run", "type": "boolean", "defaultValue": "true"],
    ]
    return WorkflowDefinition(
        id: "wf-inputs", name: "Ship a fix",
        nodes: [
            WorkflowNode(id: "t", type: .trigger, label: "Trigger", config: [
                "triggerType": "manual", "contextual": true, "inputs": inputs,
            ]),
            WorkflowNode(id: "a", type: .launchAgent, label: "Fix", config: [
                "projectName": "{{context.projectName}}", "branch": "{{context.branch}}", "useWorktree": "fromContext",
            ]),
        ],
        edges: [WorkflowEdge(id: "e", source: "t", target: "a")])
}

let sampleProjects = [
    Project(name: "vorn", path: "/Users/me/dev/vorn", icon: "Rocket", iconColor: "#c9972a"),
    Project(name: "website", path: "/Users/me/dev/website", icon: "Globe", iconColor: "#51a2ff"),
]

/// Stages the run-surface scenes; false when `scene` is not one of them.
@MainActor
func stageRunSurface(_ store: WorkflowsStore, scene: String) async -> Bool {
    switch scene {
    case "run-prompt":
        store.projects = sampleProjects
        let prompt = RunPrompt(
            workflow: sampleInputsWorkflow(), context: nil, targetNodeId: nil, projects: sampleProjects)
        prompt.branch = "fix/login"
        prompt.values["goal"] = "Stop the login loop"
        store.surfaces.prompt = prompt
    default:
        return false
    }
    return true
}

/// The scenes drawn through a hosting view, since ImageRenderer leaves text fields and web views blank.
let runSurfaceScenes: Set<String> = ["run-prompt", "missed", "full-output", "gate-review", "sign-in-wait"]

/// Draws `content` in an offscreen window that is never shown, at 2x.
@MainActor
func hostedImage(of content: some View, size: CGSize) async -> CGImage? {
    let host = NSHostingView(rootView: content)
    host.frame = CGRect(origin: .zero, size: size)
    let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: .darkAqua)
    window.backgroundColor = .clear
    window.isOpaque = false
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    try? await Task.sleep(for: .milliseconds(600))
    host.layoutSubtreeIfNeeded()
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width * 2), pixelsHigh: Int(size.height * 2),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { return nil }
    rep.size = size
    host.cacheDisplay(in: host.bounds, to: rep)
    return rep.cgImage
}
