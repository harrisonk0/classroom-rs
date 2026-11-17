import AppKit

guard CommandLine.arguments.count == 2 else {
    fatalError("Usage: swift tests/desktop_capture.swift <screenshot directory>")
}
let directory = URL(fileURLWithPath: CommandLine.arguments[1])
var failed = false
for (name, top) in [("short-answer.png", 355), ("short-answer-edit.png", 436)] {
    let png = directory.appendingPathComponent(name)
    guard let image = NSBitmapImageRep(data: try Data(contentsOf: png)) else {
        fatalError("Invalid PNG for \(name)")
    }
    let scale = Double(image.pixelsWide) / 1200
    guard abs(Double(image.pixelsHigh) - 810 * scale) <= 1 else {
        fatalError("Expected the standard 1200 × 810 capture bounds for \(name)")
    }
    var ink = 0
    for y in Int(Double(top) * scale)..<Int(Double(top + 24) * scale) {
        for x in Int(300 * scale)..<Int(1025 * scale) {
            if let color = image.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
               color.redComponent < 0.45 && color.greenComponent < 0.45 && color.blueComponent < 0.45 {
                ink += 1
            }
        }
    }
    print("\(name) draft pixels=\(ink)")
    failed = failed || ink < 100
}
exit(failed ? 1 : 0)
