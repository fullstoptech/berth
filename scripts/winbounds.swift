// Print "<windowNumber>,<x>,<y>,<width>,<height>" for the first layer-0
// on-screen window owned by the given pid. Used by scripts/shot.sh: unlike
// osascript/System Events it needs no Accessibility permission.
import CoreGraphics
import Foundation

let pid = Int(CommandLine.arguments[1])!
let opts: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else {
    exit(1)
}
for w in list {
    guard let owner = w[kCGWindowOwnerPID as String] as? Int, owner == pid,
          let layer = w[kCGWindowLayer as String] as? Int, layer == 0,
          let b = w[kCGWindowBounds as String] as? [String: Any],
          let x = b["X"] as? Double, let y = b["Y"] as? Double,
          let width = b["Width"] as? Double, let height = b["Height"] as? Double,
          let num = w[kCGWindowNumber as String] as? Int else { continue }
    print("\(num),\(Int(x)),\(Int(y)),\(Int(width)),\(Int(height))")
    exit(0)
}
exit(1)
