// Draws the menu bar title the way iStat-style items look: a small label over a number, at a
// fixed width so the number does not shift the menu bar as it changes. The PNG is returned as
// base64 for SwiftBar's `image=`, already in the menu bar's text colour: a template image would
// be faded by macOS on every display but the active one, which iStat-style meters are not.
//
// The colour follows the menu bar, not Dark Mode (a light wallpaper gives dark menu bar text even
// in Dark Mode): a zero-width status item of our own reports the menu bar's appearance.
//
// Beside the label, an optional pace dot: `dot` is a hex colour, filled or a ring.
//
// Long-running, so AppKit starts once: each stdin line `label<TAB>value<TAB>alpha<TAB>dot<TAB>filled`
// (the last two optional, `filled` 1 or 0) gets one line back, the base64 PNG (empty if it could not be drawn). Images are cached, so asking every
// second costs only the pipe.
import AppKit

let labelFont = NSFont.systemFont(ofSize: 8.5, weight: .semibold)
let valueFont = NSFont.monospacedDigitSystemFont(ofSize: 13, weight: .medium)
let height: CGFloat = 22
let scale: CGFloat = 2

// As wide as the widest value it will show, so the width never changes.
let widest = ["888", "88.8", "8.8k"]
    .map { ($0 as NSString).size(withAttributes: [.font: valueFont]).width }
    .max() ?? 30

/// `#rrggbb` as a colour.
func hexColor(_ hex: String) -> NSColor? {
    guard hex.hasPrefix("#"), hex.count == 7, let v = UInt32(hex.dropFirst(), radix: 16) else { return nil }
    return NSColor(srgbRed: CGFloat(v >> 16 & 0xff) / 255, green: CGFloat(v >> 8 & 0xff) / 255,
                   blue: CGFloat(v & 0xff) / 255, alpha: 1)
}

func render(label: NSString, value: NSString, alpha: CGFloat, dark: Bool, dot: NSColor?, filled: Bool) -> String? {
    // The menu bar's own text: near-black on a light bar, white on a dark one.
    let color = (dark ? NSColor.white : NSColor.black).withAlphaComponent(alpha * (dark ? 0.95 : 0.85))
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
    if let dot = dot {
        // Centred on the label's lowercase letters, a little past its end.
        let size: CGFloat = 5.5
        let x = ceil(label.size(withAttributes: labelAttrs).width) + 2.5
        let circle = NSBezierPath(ovalIn: NSRect(x: x, y: 13.5, width: size, height: size)
            .insetBy(dx: filled ? 0 : 0.6, dy: filled ? 0 : 0.6))
        let ink = dot.withAlphaComponent(alpha)
        if filled {
            ink.setFill()
            circle.fill()
        } else {
            ink.setStroke()
            circle.lineWidth = 1.2
            circle.stroke()
        }
    }
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])?.base64EncodedString()
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let probe = NSStatusBar.system.statusItem(withLength: 0)
var cache: [String: String] = [:]

func menuBarIsDark() -> Bool {
    guard let appearance = probe.button?.effectiveAppearance else { return false }
    return appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
}

setvbuf(stdout, nil, _IOLBF, 0)
Thread.detachNewThread {
    while let line = readLine() {
        // AppKit on the main thread, where the status item lives.
        let image: String = DispatchQueue.main.sync {
            let parts = line.components(separatedBy: "\t")
            guard parts.count >= 2 else { return "" }
            let alpha = parts.count > 2 ? CGFloat(Double(parts[2]) ?? 1) : 1
            let dark = menuBarIsDark()
            let key = "\(line)\t\(dark)"
            if let hit = cache[key] { return hit }
            let dot = parts.count > 3 ? hexColor(parts[3]) : nil
            let filled = parts.count > 4 && parts[4] == "1"
            let drawn = render(label: parts[0] as NSString, value: parts[1] as NSString, alpha: alpha, dark: dark,
                               dot: dot, filled: filled) ?? ""
            if cache.count > 500 { cache.removeAll() }
            cache[key] = drawn
            return drawn
        }
        print(image)
    }
    exit(0)
}
app.run()
