// The IME test's driver, beside the app's VORN_SPIKE_DEMO=ime (which makes
// pane 1 the focused view of the active app):
//   imectl on                      select the Japanese (Romaji) input source
//   imectl type <pid> <out> <x,y,w,h>  type "konnnichiha" (こんにちは) as real key
//                                  events, screenshot <out>-preedit.png, press
//                                  Return to commit, screenshot <out>-commit.png
//   imectl off                     back to U.S.; disables Japanese again
//                                  unless it was enabled before `on`
import AppKit
import Carbon

func source(_ id: String) -> TISInputSource? {
    (TISCreateInputSourceList([kTISPropertyInputSourceID as String: id] as CFDictionary, true)
        .takeRetainedValue() as! [TISInputSource]).first
}
func enabled(_ s: TISInputSource) -> Bool {
    Unmanaged<CFBoolean>.fromOpaque(TISGetInputSourceProperty(s, kTISPropertyInputSourceIsEnabled))
        .takeUnretainedValue() == kCFBooleanTrue
}
let parent = source("com.apple.inputmethod.Kotoeri.RomajiTyping")!
let mode = source("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese")!
let marker = "/tmp/vorn-spike-imectl-enabled"
switch CommandLine.arguments.dropFirst().first {
case "on":
    if !enabled(parent) {
        TISEnableInputSource(parent)
        FileManager.default.createFile(atPath: marker, contents: nil)
        sleep(2)
    }
    print("select:", TISSelectInputSource(mode))
case "off":
    print("select US:", TISSelectInputSource(source("com.apple.keylayout.US")!))
    if FileManager.default.fileExists(atPath: marker) {
        print("disable:", TISDisableInputSource(parent))
        try? FileManager.default.removeItem(atPath: marker)
    }
case "type":
    let a = CommandLine.arguments
    let pid = pid_t(a[2])!
    let keys: [Character: CGKeyCode] = ["k": 40, "o": 31, "n": 45, "i": 34, "c": 8, "h": 4, "a": 0]
    let src = CGEventSource(stateID: .hidSystemState)
    func tap(_ k: CGKeyCode) {
        CGEvent(keyboardEventSource: src, virtualKey: k, keyDown: true)?.postToPid(pid)
        usleep(30_000)
        CGEvent(keyboardEventSource: src, virtualKey: k, keyDown: false)?.postToPid(pid)
        usleep(90_000)
    }
    func shot(_ tag: String) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        p.arguments = ["-x", "-R", a[4], "\(a[3])-\(tag).png"]
        try? p.run()
        p.waitUntilExit()
    }
    for c in "konnnichiha" { tap(keys[c]!) }
    usleep(600_000)
    shot("preedit")
    tap(36)
    usleep(600_000)
    shot("commit")
default:
    print("usage: imectl on|off|type <pid> <out> <x,y,w,h>")
}
