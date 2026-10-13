import Foundation

/// Terminal preferences kept on this Mac.
public enum TerminalPreferences {
    /// Option types Meta: Option+key sends Esc then the key, instead of the character Option makes.
    public static let optionAsMetaKey = "vorn.terminal.optionAsMeta"

    public static var optionAsMeta: Bool { UserDefaults.standard.bool(forKey: optionAsMetaKey) }

    /// The terminal font size steps ⌘+ and ⌘- move through, as Settings allows.
    public static let fontSizes: ClosedRange<Double> = 8...24
    public static let defaultFontSize: Double = 13

    /// The font size one step bigger (`by` 1) or smaller (-1), kept in range.
    public static func fontSize(_ current: Double?, by step: Int) -> Double {
        let next = (current ?? defaultFontSize).rounded() + Double(step)
        return min(max(next, fontSizes.lowerBound), fontSizes.upperBound)
    }
}
