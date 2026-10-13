import CoreImage
import CoreImage.CIFilterBuiltins
import Foundation

/// A QR code's modules, row-major, without a quiet zone (CIQRCodeGenerator).
public struct QRMatrix: Hashable, Sendable {
    public let size: Int
    public let modules: [Bool]

    public subscript(x: Int, y: Int) -> Bool { modules[y * size + x] }

    /// Encodes `text` at error correction `level` (L, M, Q, H); nil when it does not fit.
    public static func make(_ text: String, level: String = "M") -> QRMatrix? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = level
        guard let raw = filter.outputImage else { return nil }
        // Rendered large, unfiltered and on the CPU, then read at module centres, so no GPU or blending decides a module.
        let scale = 8
        let output = raw.samplingNearest().transformed(by: CGAffineTransform(scaleX: CGFloat(scale), y: CGFloat(scale)))
        let extent = output.extent.integral
        let w = Int(extent.width), h = Int(extent.height)
        guard w > 0, w == h else { return nil }
        var pixels = [UInt8](repeating: 0, count: w * h)
        let context = CIContext(options: [.workingColorSpace: NSNull(), .outputColorSpace: NSNull(), .useSoftwareRenderer: true])
        context.render(output, toBitmap: &pixels, rowBytes: w, bounds: extent, format: .L8, colorSpace: nil)
        let dark = pixels.map { $0 < 128 }
        // The generator pads with a quiet zone; the dark pixels' bounds are the code.
        var minX = w, minY = h, maxX = -1, maxY = -1
        for y in 0..<h {
            for x in 0..<w where dark[y * w + x] {
                minX = min(minX, x); maxX = max(maxX, x); minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        guard maxX >= minX, maxY >= minY else { return nil }
        let size = Int((Double(maxX - minX + 1) / Double(scale)).rounded())
        guard size > 0 else { return nil }
        var modules = [Bool](repeating: false, count: size * size)
        for y in 0..<size {
            for x in 0..<size {
                let px = min(w - 1, minX + x * scale + scale / 2), py = min(h - 1, minY + y * scale + scale / 2)
                modules[y * size + x] = dark[py * w + px]
            }
        }
        return QRMatrix(size: size, modules: modules)
    }
}
