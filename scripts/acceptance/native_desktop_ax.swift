// External macOS Accessibility client. NOT linked into SwarmCraft.
// JSON requests arrive on stdin; never put an invitation in argv or the clipboard.
import AppKit
import ApplicationServices
import Foundation

struct Request: Decodable {
    let pid: Int32
    let action: String
    let identifier: String?
    let label: String?
    let roles: [String]?
    let prefix: Bool?
    let exactIdentifier: Bool?
    let value: String?
}

enum Failure: Error {
    case denied, invalid, missing, ambiguous, disabled, unavailable
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var result: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &result) == .success else { return nil }
    return result
}

func string(_ element: AXUIElement, _ name: String) -> String? {
    return attribute(element, name) as? String
}

func valueString(_ element: AXUIElement) -> String {
    let value = attribute(element, "AXValue")
    return value as? String ?? (value as? NSNumber)?.stringValue ?? ""
}

func children(_ element: AXUIElement, _ name: String = "AXChildren") -> [AXUIElement] {
    return attribute(element, name) as? [AXUIElement] ?? []
}

func enabled(_ element: AXUIElement) -> Bool {
    return (attribute(element, "AXEnabled") as? NSNumber)?.boolValue == true
}

func role(_ element: AXUIElement) -> String { string(element, "AXRole") ?? "" }

func walk(_ roots: [AXUIElement]) throws -> [AXUIElement] {
    var stack = roots.map { ($0, 0) }
    var output: [AXUIElement] = []
    while let (element, depth) = stack.popLast() {
        guard output.count < 6000, depth < 64 else { throw Failure.unavailable }
        output.append(element)
        stack.append(contentsOf: children(element).map { ($0, depth + 1) })
    }
    return output
}

func labels(_ element: AXUIElement) -> [String] {
    var result = ["AXTitle", "AXDescription", "AXValue", "AXHelp"].compactMap { string(element, $0) }
    // WebKit often exposes a button's name as a child static-text node.
    for child in children(element).prefix(12) {
        result.append(contentsOf: ["AXTitle", "AXDescription", "AXValue"].compactMap { string(child, $0) })
    }
    return result.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
}

func find(_ request: Request, _ app: AXUIElement) throws -> AXUIElement {
    // Never search the whole desktop or a browser. Only WKWebView's accessibility
    // web area in the exact launched application process is in scope.
    let windows = children(app, "AXWindows")
    let areas = try walk(windows).filter { role($0) == "AXWebArea" }
    guard areas.count == 1 else {
        throw areas.isEmpty ? Failure.missing : Failure.ambiguous
    }
    let nodes = try walk(areas).filter { request.roles?.contains(role($0)) ?? true }
    if let id = request.identifier, !id.isEmpty {
        let exact = nodes.filter { string($0, "AXDOMIdentifier") == id || string($0, "AXIdentifier") == id }
        if exact.count == 1 { return exact[0] }
        if exact.count > 1 { throw Failure.ambiguous }
    }
    // The node and header readiness indicators have identical labels but
    // independent meanings. Do not let one Ready node satisfy both probes.
    if request.exactIdentifier == true { throw Failure.missing }
    if let label = request.label, !label.isEmpty {
        var exact = nodes.filter { node in
            labels(node).contains { request.prefix == true ? $0.hasPrefix(label) : $0 == label }
        }
        if request.action == "read",
           let roles = request.roles,
           roles.allSatisfy({ ["AXStaticText", "AXHeading", "AXGroup"].contains($0) }) {
            // WebKit may expose the same visible text in a grouping element
            // and in a leaf. Prefer the leaf, never a container's child label.
            let leaves = exact.filter { role($0) == "AXStaticText" || role($0) == "AXHeading" }
            if !leaves.isEmpty { exact = leaves }
        }
        if exact.count == 1 { return exact[0] }
        if exact.count > 1 { throw Failure.ambiguous }
    }
    throw Failure.missing
}

func run(_ request: Request) throws -> [String: Any] {
    guard AXIsProcessTrusted() else { throw Failure.denied }
    guard request.pid > 1, let running = NSRunningApplication(processIdentifier: request.pid),
          !running.isTerminated else { throw Failure.missing }
    let app = AXUIElementCreateApplication(request.pid)
    AXUIElementSetMessagingTimeout(app, 2)
    if request.action == "window" {
        // Window-specific capture, never a screenshot of another application.
        let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
            as? [[String: Any]] ?? []
        let windows = info.filter {
            ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == request.pid &&
            ($0[kCGWindowLayer as String] as? NSNumber)?.intValue == 0
        }
        guard windows.count == 1, let id = windows[0][kCGWindowNumber as String] as? NSNumber else {
            throw Failure.unavailable
        }
        return ["window": id]
    }
    if request.action == "close" {
        guard running.terminate() else { throw Failure.unavailable }
        return ["ok": true]
    }
    let element = try find(request, app)
    switch request.action {
    case "read":
        let value: String
        if ["AXTextField", "AXTextArea", "AXIncrementor", "AXSpinButton"].contains(role(element)) {
            value = valueString(element)
        } else if let label = request.label, let matching = labels(element).first(where: {
            request.prefix == true ? $0.hasPrefix(label) : $0 == label
        }) {
            value = matching
        } else {
            value = labels(element).first ?? ""
        }
        return ["value": value, "enabled": enabled(element),
                "checked": (attribute(element, "AXValue") as? NSNumber)?.boolValue ?? false]
    case "press":
        guard enabled(element) else { throw Failure.disabled }
        _ = AXUIElementPerformAction(element, "AXScrollToVisible" as CFString)
        guard AXUIElementPerformAction(element, kAXPressAction as CFString) == .success else {
            throw Failure.unavailable
        }
        return ["ok": true]
    case "type":
        guard enabled(element), let value = request.value, value.utf16.count <= 32768,
              ["AXTextField", "AXTextArea", "AXIncrementor", "AXSpinButton"].contains(role(element)) else { throw Failure.invalid }
        _ = AXUIElementPerformAction(element, "AXScrollToVisible" as CFString)
        guard running.activate(options: [.activateIgnoringOtherApps]),
              AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success else {
            throw Failure.unavailable
        }
        // Use actual keyboard events addressed to this process, not JS or AXValue
        // mutation (the latter can bypass input events in some WebKit versions).
        guard let source = CGEventSource(stateID: .combinedSessionState),
              let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else {
            throw Failure.unavailable
        }
        down.flags = .maskCommand
        up.flags = .maskCommand
        down.postToPid(request.pid)
        up.postToPid(request.pid)
        Thread.sleep(forTimeInterval: 0.1)
        // Each chunk is a small Unicode keyboard event; no global clipboard.
        for character in value {
            let units = Array(String(character).utf16)
            guard let key = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
                  let release = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else {
                throw Failure.unavailable
            }
            units.withUnsafeBufferPointer {
                key.keyboardSetUnicodeString(stringLength: $0.count, unicodeString: $0.baseAddress)
                release.keyboardSetUnicodeString(stringLength: $0.count, unicodeString: $0.baseAddress)
            }
            key.postToPid(request.pid)
            release.postToPid(request.pid)
        }
        let deadline = Date().addingTimeInterval(5)
        repeat {
            if valueString(element) == value { return ["ok": true] }
            Thread.sleep(forTimeInterval: 0.1)
        } while Date() < deadline
        throw Failure.unavailable
    default: throw Failure.invalid
    }
}

do {
    let data = FileHandle.standardInput.readDataToEndOfFile()
    guard data.count <= 131072 else { throw Failure.invalid }
    let request = try JSONDecoder().decode(Request.self, from: data)
    let result = try run(request)
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: result))
} catch {
    // No values, native exception strings, or accessibility tree dumps in logs.
    let kind: String
    switch error {
    case Failure.denied: kind = "accessibility-permission-required"
    case Failure.missing: kind = "element-or-window-not-found"
    case Failure.ambiguous: kind = "ambiguous-accessibility-element"
    case Failure.disabled: kind = "control-disabled"
    default: kind = "native-accessibility-operation-failed"
    }
    FileHandle.standardOutput.write(try! JSONSerialization.data(withJSONObject: ["error": kind]))
    exit(1)
}
