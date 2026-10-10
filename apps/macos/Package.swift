// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Vorn",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "VornCore", targets: ["VornCore"]),
        .library(name: "VornUI", targets: ["VornUI"]),
        .library(name: "VornWorkflows", targets: ["VornWorkflows"]),
    ],
    targets: [
        .target(name: "VornCore"),
        .target(name: "VornUI"),
        .target(name: "VornWorkflows", dependencies: ["VornCore", "VornUI"]),
        // Mounts the Workflows view alone, in a window or offscreen.
        .executableTarget(
            name: "WorkflowsPreview",
            dependencies: ["VornCore", "VornUI", "VornWorkflows"]
        ),
        .testTarget(name: "VornCoreTests", dependencies: ["VornCore"]),
        .testTarget(
            name: "VornWorkflowsTests",
            dependencies: ["VornWorkflows", "VornCore"],
            resources: [.copy("Fixtures")]
        ),
    ]
)
