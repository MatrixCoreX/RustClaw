import Foundation
import CoreGraphics

guard CommandLine.arguments.count == 2, let pid = Int(CommandLine.arguments[1]) else { exit(2) }
for _ in 0..<30 {
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    for window in windows where window[kCGWindowOwnerPID as String] as? Int == pid {
        if let bounds = window[kCGWindowBounds as String] as? [String: Any],
           let width = bounds["Width"] as? Double, width >= 800 {
            let result: [String: Any] = ["pid": pid, "visible": true, "bounds": bounds]
            let data = try! JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
            print(String(data: data, encoding: .utf8)!)
            Thread.sleep(forTimeInterval: 3)
            exit(0)
        }
    }
    Thread.sleep(forTimeInterval: 1)
}
fputs("application_window_missing\n", stderr)
exit(1)
