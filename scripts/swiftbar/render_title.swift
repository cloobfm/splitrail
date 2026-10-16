// Draws the menu bar title the way iStat-style items look: a small label over a number, at a
// fixed width so the number does not shift the menu bar as it changes. The PNG is returned as
// base64 for SwiftBar's `image=`, already in the menu bar's text colour: a template image would
// be faded by macOS on every display but the active one, which iStat-style meters are not.
//
// The colour follows the menu bar, not Dark Mode (a light wallpaper gives dark menu bar text even
// in Dark Mode): a zero-width status item of our own reports the menu bar's appearance.
//
// Long-running, so AppKit starts once: each stdin line `label<TAB>value<TAB>alpha` gets one line
// back, the base64 PNG (empty if it could not be drawn). Images are cached, so asking every
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

func render(label: NSString, value: NSString, alpha: CGFloat, dark: Bool) -> String? {
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
            let drawn = render(label: parts[0] as NSString, value: parts[1] as NSString, alpha: alpha, dark: dark) ?? ""
            if cache.count > 500 { cache.removeAll() }
            cache[key] = drawn
            return drawn
        }
        print(image)
    }
    exit(0)
}
app.run()
