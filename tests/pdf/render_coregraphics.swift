import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

// Match PDF page boxes at CSS pixel density, using Apple's PDF renderer.
let arguments = CommandLine.arguments
if arguments.count != 4 {
    fputs("usage: render-coregraphics PDF DPI OUTPUT_PREFIX\n", stderr)
    exit(2)
}
guard let dpi = Double(arguments[2]), dpi.isFinite, dpi > 0,
      let document = CGPDFDocument(URL(fileURLWithPath: arguments[1]) as CFURL) else {
    fputs("invalid PDF or density\n", stderr)
    exit(2)
}
let scale = dpi / 72.0
let digits = String(document.numberOfPages).count
for index in 1...document.numberOfPages {
    guard let page = document.page(at: index) else { exit(1) }
    let bounds = page.getBoxRect(.mediaBox)
    let width = Int(ceil(bounds.width * scale))
    let height = Int(ceil(bounds.height * scale))
    guard let context = CGContext(data: nil, width: width, height: height,
        bitsPerComponent: 8, bytesPerRow: width * 4,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { exit(1) }
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    context.translateBy(x: 0, y: Double(height) - bounds.height * scale)
    context.scaleBy(x: scale, y: scale)
    context.translateBy(x: -bounds.minX, y: -bounds.minY)
    context.drawPDFPage(page)
    let filename = arguments[3] + "-" + String(format: "%0*d", digits, index) + ".png"
    guard let image = context.makeImage(),
          let destination = CGImageDestinationCreateWithURL(
            URL(fileURLWithPath: filename) as CFURL, UTType.png.identifier as CFString, 1, nil) else { exit(1) }
    CGImageDestinationAddImage(destination, image, nil)
    if !CGImageDestinationFinalize(destination) { exit(1) }
}
