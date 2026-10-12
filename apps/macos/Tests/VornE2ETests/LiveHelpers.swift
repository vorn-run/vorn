import Foundation
import Testing

/// Polls `check` until it holds or `timeout` passes.
@MainActor
func liveUntil(timeout: TimeInterval = 20, _ check: @MainActor () async -> Bool) async -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if await check() { return true }
        try? await Task.sleep(for: .milliseconds(100))
    }
    return false
}

/// Runs git in `dir` with a fixed identity and no user config.
@discardableResult
func git(_ dir: URL, _ args: String...) throws -> String {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/usr/bin/git")
    p.arguments = ["-C", dir.path] + args
    p.environment = [
        "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com", "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@example.com", "HOME": dir.deletingLastPathComponent().path, "PATH": "/usr/bin:/bin",
    ]
    let out = Pipe()
    p.standardOutput = out
    p.standardError = FileHandle.nullDevice
    try p.run()
    let data = out.fileHandleForReading.readDataToEndOfFile()
    p.waitUntilExit()
    #expect(p.terminationStatus == 0, "git \(args.joined(separator: " "))")
    return String(decoding: data, as: UTF8.self)
}

/// A repo with one commit holding `files`.
func makeGitRepo(at url: URL, files: [String: String] = ["README.md": "hello\n"]) throws {
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    for (name, body) in files {
        let f = url.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: f.deletingLastPathComponent(), withIntermediateDirectories: true)
        try body.write(to: f, atomically: true, encoding: .utf8)
    }
    try git(url, "init", "-q", "-b", "main")
    try git(url, "add", ".")
    try git(url, "commit", "-q", "-m", "init")
}
