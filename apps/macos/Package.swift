// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The grid client is Rust: grid-ffi/build.sh builds it into this directory.
let gridLibDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("grid-ffi/target/aarch64-apple-darwin/release")
    .path

let package = Package(
    name: "Vorn",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .executable(name: "Vorn", targets: ["VornMac"]),
        .executable(name: "VornTasksPreview", targets: ["VornTasksPreview"]),
        .executable(name: "WorkflowsPreview", targets: ["WorkflowsPreview"]),
        .library(name: "VornCore", targets: ["VornCore"]),
        .library(name: "VornUI", targets: ["VornUI"]),
        .library(name: "VornTerminals", targets: ["VornTerminals"]),
        .library(name: "VornTasks", targets: ["VornTasks"]),
        .library(name: "VornWorkflows", targets: ["VornWorkflows"]),
        .library(name: "VornApp", targets: ["VornApp"]),
    ],
    targets: [
        .target(name: "VornCore"),
        .target(name: "VornUI", dependencies: ["VornCore"], resources: [.process("Resources")]),
        .target(
            name: "CVornGrid",
            linkerSettings: [
                .unsafeFlags(["-L\(gridLibDir)"]),
                .linkedLibrary("vorn_grid_ffi"),
            ]
        ),
        .target(name: "VornTerminals", dependencies: ["VornCore", "VornUI", "CVornGrid"]),
        .target(name: "VornTasks", dependencies: ["VornCore", "VornUI", "VornWorkflows"]),
        .target(name: "VornWorkflows", dependencies: ["VornCore", "VornUI"]),
        .target(
            name: "VornApp",
            dependencies: ["VornCore", "VornUI", "VornTerminals", "VornTasks", "VornWorkflows"]
        ),
        .executableTarget(name: "VornMac", dependencies: ["VornApp"]),
        // Mount one feature view alone, in a window or offscreen.
        .executableTarget(name: "VornTasksPreview", dependencies: ["VornCore", "VornUI", "VornTasks"]),
        .executableTarget(name: "WorkflowsPreview", dependencies: ["VornCore", "VornUI", "VornWorkflows"]),
        .testTarget(name: "VornCoreTests", dependencies: ["VornCore"]),
        .testTarget(name: "VornTerminalsTests", dependencies: ["VornTerminals", "VornCore"]),
        .testTarget(name: "VornTasksTests", dependencies: ["VornCore", "VornTasks"]),
        .testTarget(
            name: "VornWorkflowsTests",
            dependencies: ["VornWorkflows", "VornCore"],
            resources: [.copy("Fixtures")]
        ),
        .testTarget(name: "VornE2ETests", dependencies: ["VornCore", "VornTerminals", "VornTasks", "VornWorkflows", "VornApp"]),
    ],
    swiftLanguageModes: [.v5]
)
