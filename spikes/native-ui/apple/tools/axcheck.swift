import ApplicationServices
import AppKit
// usage: axcheck <pid>  — prints what VoiceOver would read from the focused element.
let pid = pid_t(CommandLine.arguments[1])!
print("trusted:", AXIsProcessTrusted())
let app = AXUIElementCreateApplication(pid)
func attr(_ e: AXUIElement, _ a: String) -> AnyObject? {
    var v: AnyObject?; let r = AXUIElementCopyAttributeValue(e, a as CFString, &v)
    return r == .success ? v : nil
}
func pattr(_ e: AXUIElement, _ a: String, _ p: AnyObject) -> AnyObject? {
    var v: AnyObject?; let r = AXUIElementCopyParameterizedAttributeValue(e, a as CFString, p, &v)
    return r == .success ? v : nil
}
guard let f = attr(app, kAXFocusedUIElementAttribute) else { print("no focused element"); exit(1) }
let el = f as! AXUIElement
print("role:", attr(el, kAXRoleAttribute) as? String ?? "-", "| description:", attr(el, kAXDescriptionAttribute) as? String ?? "-")
let value = attr(el, kAXValueAttribute) as? String ?? ""
print("value chars:", value.count)
print("value tail:", value.split(separator: "\n").suffix(3).joined(separator: " ⏎ "))
if let n = attr(el, kAXInsertionPointLineNumberAttribute) as? Int {
    print("insertion line:", n)
    if let rv = pattr(el, kAXRangeForLineParameterizedAttribute, n as NSNumber) {
        if let s = pattr(el, kAXStringForRangeParameterizedAttribute, rv) as? String { print("cursor line text:", "[\(s.trimmingCharacters(in: .newlines))]") }
    }
}
