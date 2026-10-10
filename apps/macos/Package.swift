// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "Vorn",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "VornCore", targets: ["VornCore"]),
        .library(name: "VornUI", targets: ["VornUI"]),
        .library(name: "VornTasks", targets: ["VornTasks"]),
        .executable(name: "VornTasksPreview", targets: ["VornTasksPreview"]),
    ],
    targets: [
        .target(name: "VornCore"),
        .target(name: "VornUI", dependencies: ["VornCore"]),
        .target(name: "VornTasks", dependencies: ["VornCore", "VornUI"]),
        .executableTarget(
            name: "VornTasksPreview",
            dependencies: ["VornCore", "VornUI", "VornTasks"]
        ),
        .testTarget(name: "VornCoreTests", dependencies: ["VornCore"]),
        .testTarget(name: "VornTasksTests", dependencies: ["VornCore", "VornTasks"]),
    ]
)
