// winwait <pid> <timeout seconds>: prints the epoch time at which <pid> first
// has an on-screen window larger than 200x200 points, or "none" and exits 1.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count == 3, let pid = Int32(args[1]), let limit = Double(args[2]) else {
  FileHandle.standardError.write("usage: winwait <pid> <timeout>\n".data(using: .utf8)!)
  exit(2)
}
let end = Date().addingTimeInterval(limit)
while Date() < end {
  let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
  for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
    let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
    if (b["Width"] ?? 0) > 200 && (b["Height"] ?? 0) > 200 {
      print(String(format: "%.3f", Date().timeIntervalSince1970))
      exit(0)
    }
  }
  usleep(5000)
}
print("none")
exit(1)
