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
        guard let output = filter.outputImage else { return nil }
        let extent = output.extent.integral
        let w = Int(extent.width), h = Int(extent.height)
        guard w > 0, w == h else { return nil }
        var pixels = [UInt8](repeating: 0, count: w * h)
        let context = CIContext(options: [.workingColorSpace: NSNull(), .outputColorSpace: NSNull()])
        context.render(output, toBitmap: &pixels, rowBytes: w, bounds: extent, format: .L8, colorSpace: nil)
        let dark = pixels.map { $0 < 128 }
        // The generator pads with a quiet zone; the finder pattern's corner marks where the code starts.
        guard let q = (0..<w).first(where: { dark[$0 * w + $0] }) else { return nil }
        let size = w - 2 * q
        guard size > 0 else { return nil }
        var modules = [Bool](repeating: false, count: size * size)
        for y in 0..<size {
            for x in 0..<size { modules[y * size + x] = dark[(y + q) * w + (x + q)] }
        }
        return QRMatrix(size: size, modules: modules)
    }
}
