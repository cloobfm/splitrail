// Draws the menu bar title the way iStat-style items look: a small label over a number, at a
// fixed width so the number does not shift the menu bar as it changes. The PNG is returned as
// base64 for SwiftBar's `templateImage=` (black on clear, tinted by the system for light and
// dark menu bars).
//
// Long-running, so AppKit starts once: each stdin line `label<TAB>value<TAB>alpha` gets one line
// back, the base64 PNG (empty if it could not be drawn).
import AppKit

let labelFont = NSFont.systemFont(ofSize: 8.5, weight: .semibold)
let valueFont = NSFont.monospacedDigitSystemFont(ofSize: 13, weight: .medium)
let height: CGFloat = 22
let scale: CGFloat = 2

// As wide as the widest value it will show, so the width never changes.
let widest = ["888", "88.8", "8.8k"]
    .map { ($0 as NSString).size(withAttributes: [.font: valueFont]).width }
    .max() ?? 30

func render(label: NSString, value: NSString, alpha: CGFloat) -> String? {
    let color = NSColor.black.withAlphaComponent(alpha)
    let labelAttrs: [NSAttributedString.Key: Any] = [.font: labelFont, .foregroundColor: color]
    let valueAttrs: [NSAttributedString.Key: Any] = [.font: valueFont, .foregroundColor: color]
    let width = ceil(max(widest, label.size(withAttributes: labelAttrs).width)) + 1
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(width * scale), pixelsHigh: Int(height * scale),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
    else { return nil }
    rep.size = NSSize(width: width, height: height) // Points: drawn at Retina resolution.
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    label.draw(at: NSPoint(x: 0, y: 12), withAttributes: labelAttrs)
    value.draw(at: NSPoint(x: 0, y: -2), withAttributes: valueAttrs)
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])?.base64EncodedString()
}

setvbuf(stdout, nil, _IOLBF, 0)
while let line = readLine() {
    let parts = line.components(separatedBy: "\t")
    guard parts.count >= 2 else {
        print("")
        continue
    }
    let alpha = parts.count > 2 ? CGFloat(Double(parts[2]) ?? 1) : 1
    print(render(label: parts[0] as NSString, value: parts[1] as NSString, alpha: alpha) ?? "")
}
