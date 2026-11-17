import AppKit

guard CommandLine.arguments.count == 2 else { exit(2) }
let size = 1024
let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0
)!
let graphics = NSGraphicsContext(bitmapImageRep: bitmap)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = graphics

func color(_ rgb: UInt32) -> NSColor {
    NSColor(srgbRed: CGFloat((rgb >> 16) & 255) / 255,
            green: CGFloat((rgb >> 8) & 255) / 255,
            blue: CGFloat(rgb & 255) / 255, alpha: 1)
}
func rounded(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ radius: CGFloat) -> NSBezierPath {
    NSBezierPath(roundedRect: NSRect(x: x, y: y, width: w, height: h), xRadius: radius, yRadius: radius)
}
let tile = rounded(64, 64, 896, 896, 205)
NSGradient(starting: color(0x152E35), ending: color(0x226A64))!.draw(in: tile, angle: 75)

color(0x449C88).setFill()
rounded(230, 294, 528, 407, 55).fill()
color(0xC5F1DB).setFill()
rounded(206, 318, 528, 407, 54).fill()
color(0x174D4C).setFill()
rounded(237, 349, 466, 345, 30).fill()

let check = NSBezierPath()
check.move(to: NSPoint(x: 347, y: 509))
check.line(to: NSPoint(x: 430, y: 430))
check.line(to: NSPoint(x: 580, y: 585))
check.lineWidth = 37
check.lineCapStyle = .round
check.lineJoinStyle = .round
color(0xD6FAE7).setStroke()
check.stroke()

color(0xF7C86C).setFill()
rounded(603, 321, 83, 16, 8).fill()
color(0xA2DFCA).setFill()
rounded(384, 244, 187, 25, 13).fill()

NSGraphicsContext.restoreGraphicsState()
guard let png = bitmap.representation(using: .png, properties: [:]) else { exit(1) }
try png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
