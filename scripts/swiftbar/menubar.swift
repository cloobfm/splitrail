// The Splitrail menu bar item: a small `tk/s` label over the number at a fixed width, like
// iStat-style meters, with the pace dot beside the label, and the dropdown under it.
//
// It is its own status item rather than SwiftBar's because only a status item that draws at
// display time gets each display's menu bar colour: SwiftBar shows one fixed image on every
// display (white text on a light menu bar beside a dark one), and its template images are faded
// by macOS on every display but the active one. The text here is `labelColor`, resolved per
// display as it is drawn.
//
// `splitrail.py` (the SwiftBar plugin, which prints nothing to SwiftBar while this runs) starts
// it and writes one block per change to stdin:
//
//     TITLE<TAB>label<TAB>value<TAB>alpha<TAB>dot colour (#rrggbb or empty)<TAB>filled (1/0)
//     …menu lines in SwiftBar's format: `text | color=… font=… size=… bash=…`, `---` for a
//     separator, a leading `--` for a submenu item…
//     ~~~
//
// When stdin closes (the plugin stopped) it exits and the item goes away.
import AppKit

let labelFont = NSFont.systemFont(ofSize: 8.5, weight: .semibold)
let valueFont = NSFont.monospacedDigitSystemFont(ofSize: 13, weight: .medium)
let height: CGFloat = 22

// As wide as the widest value it will show, so the width never changes.
let widest = ["888", "88.8", "8.8k"]
    .map { ($0 as NSString).size(withAttributes: [.font: valueFont]).width }
    .max() ?? 30

/// `#rrggbb` or a few names as a colour.
func color(_ text: String) -> NSColor? {
    switch text.lowercased() {
    case "gray", "grey": return .secondaryLabelColor
    case "red": return .systemRed
    case "green": return .systemGreen
    default: break
    }
    guard text.hasPrefix("#"), text.count == 7, let v = UInt32(text.dropFirst(), radix: 16) else { return nil }
    return NSColor(srgbRed: CGFloat(v >> 16 & 0xff) / 255, green: CGFloat(v >> 8 & 0xff) / 255,
                   blue: CGFloat(v & 0xff) / 255, alpha: 1)
}

struct Title: Equatable {
    var label = "tk/s"
    var value = "—"
    var alpha: CGFloat = 0.5
    var dot: String = ""
    var filled = false
}

/// The title as an image drawn when the menu bar draws it, in that display's text colour.
func titleImage(_ t: Title) -> NSImage {
    let label = t.label as NSString
    let value = t.value as NSString
    let width = ceil(max(widest, label.size(withAttributes: [.font: labelFont]).width)) + 1
    let image = NSImage(size: NSSize(width: width, height: height), flipped: false) { _ in
        let ink = NSColor.labelColor.withAlphaComponent(t.alpha)
        label.draw(at: NSPoint(x: 0, y: 12), withAttributes: [.font: labelFont, .foregroundColor: ink])
        value.draw(at: NSPoint(x: 0, y: -2), withAttributes: [.font: valueFont, .foregroundColor: ink])
        if let dot = color(t.dot) {
            // Centred on the label's lowercase letters, a little past its end.
            let size: CGFloat = 5.5
            let x = ceil(label.size(withAttributes: [.font: labelFont]).width) + 2.5
            let circle = NSBezierPath(ovalIn: NSRect(x: x, y: 13.5, width: size, height: size)
                .insetBy(dx: t.filled ? 0 : 0.6, dy: t.filled ? 0 : 0.6))
            let fill = dot.withAlphaComponent(t.alpha)
            if t.filled {
                fill.setFill()
                circle.fill()
            } else {
                fill.setStroke()
                circle.lineWidth = 1.2
                circle.stroke()
            }
        }
        return true
    }
    image.isTemplate = false
    return image
}

final class Runner: NSObject {
    let path: String
    init(path: String) { self.path = path }
    @objc func run() {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/bash")
        process.arguments = [path]
        try? process.run()
    }
}

/// One SwiftBar-format line as a menu item.
func menuItem(_ line: String, runners: inout [Runner]) -> NSMenuItem {
    var text = line
    var params: [String: String] = [:]
    if let bar = line.range(of: " | ", options: .backwards) {
        text = String(line[..<bar.lowerBound])
        for pair in line[bar.upperBound...].split(separator: " ") {
            let kv = pair.split(separator: "=", maxSplits: 1).map(String.init)
            if kv.count == 2 { params[kv[0]] = kv[1] }
        }
    }
    let size = CGFloat(Double(params["size"] ?? "") ?? 13)
    let font = params["font"].flatMap { NSFont(name: $0, size: size) } ?? NSFont.menuFont(ofSize: size)
    var attrs: [NSAttributedString.Key: Any] = [.font: font]
    if let c = params["color"].flatMap(color) { attrs[.foregroundColor] = c }
    let item = NSMenuItem(title: text, action: nil, keyEquivalent: "")
    item.attributedTitle = NSAttributedString(string: text, attributes: attrs)
    if let path = params["bash"] {
        let runner = Runner(path: path)
        runners.append(runner)
        item.target = runner
        item.action = #selector(Runner.run)
    }
    return item
}

final class Item: NSObject, NSMenuDelegate {
    let status = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    let menu = NSMenu()
    var title = Title()
    var lines: [String] = []
    var runners: [Runner] = []

    override init() {
        super.init()
        status.autosaveName = "splitrail" // Keeps the place it is dragged to.
        menu.autoenablesItems = false
        menu.delegate = self
        status.menu = menu
        status.button?.image = titleImage(title)
    }

    func update(title new: Title, lines newLines: [String]) {
        if new != title {
            title = new
            status.button?.image = titleImage(new)
        }
        lines = newLines
    }

    /// Built as it opens, from the latest lines, so updates never close an open menu.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        runners.removeAll()
        var parent: NSMenuItem?
        for line in lines {
            if line == "---" {
                menu.addItem(.separator())
                parent = nil
            } else if line.hasPrefix("--"), let parent {
                if parent.submenu == nil {
                    parent.submenu = NSMenu()
                    parent.submenu?.autoenablesItems = false
                }
                parent.submenu?.addItem(menuItem(String(line.dropFirst(2)), runners: &runners))
            } else {
                let item = menuItem(line, runners: &runners)
                menu.addItem(item)
                parent = item
            }
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let item = Item()

Thread.detachNewThread {
    var block: [String] = []
    while let line = readLine() {
        guard line == "~~~" else {
            block.append(line)
            continue
        }
        var title = Title()
        var lines = block
        if let first = block.first, first.hasPrefix("TITLE\t") {
            let parts = first.components(separatedBy: "\t")
            title.label = parts.count > 1 ? parts[1] : "tk/s"
            title.value = parts.count > 2 ? parts[2] : "—"
            title.alpha = parts.count > 3 ? CGFloat(Double(parts[3]) ?? 1) : 1
            title.dot = parts.count > 4 ? parts[4] : ""
            title.filled = parts.count > 5 && parts[5] == "1"
            lines.removeFirst()
        }
        block = []
        DispatchQueue.main.async { item.update(title: title, lines: lines) }
    }
    exit(0)
}
app.run()
