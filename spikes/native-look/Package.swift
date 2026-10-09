// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "NativeLook",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "NativeLook",
            resources: [.copy("Resources/vorn-logo.png")]
        )
    ]
)
