import SwiftUI
import VornCore
import VornUI

/// The renderer's lucide glyphs, drawn with the closest SF Symbol.
public enum Glyph: String, Sendable {
    case activity, check, checkSquare, chevronDown, chevronRight, clock, download, edit, externalLink, eye, filterList,
        gitFork, hand, history, inbox, listPlus, loader, messageSquare, more, pencil, play, power, refresh,
        `repeat`, rotateCcw, save, settings, sliders, square, terminal, trash, upload, workflow, x, zap, globe,
        sidebar

    public var symbol: String {
        switch self {
        case .activity: return "waveform.path.ecg"
        case .check: return "checkmark"
        case .checkSquare: return "checkmark.square"
        case .chevronDown: return "chevron.down"
        case .chevronRight: return "chevron.right"
        case .clock: return "clock"
        case .download: return "square.and.arrow.down"
        case .edit, .pencil: return "pencil"
        case .externalLink: return "arrow.up.right.square"
        case .eye: return "eye"
        case .filterList: return "line.3.horizontal.decrease"
        case .gitFork: return "arrow.triangle.branch"
        case .hand: return "hand.raised"
        case .history: return "clock.arrow.circlepath"
        case .inbox: return "tray"
        case .listPlus: return "text.badge.plus"
        case .loader: return "rays"
        case .messageSquare: return "bubble.left"
        case .more: return "ellipsis"
        case .play: return "play"
        case .power: return "power"
        case .refresh: return "arrow.clockwise"
        case .repeat: return "repeat"
        case .rotateCcw: return "arrow.counterclockwise"
        case .save: return "square.and.arrow.down.on.square"
        case .settings: return "gearshape"
        case .sliders: return "slider.horizontal.3"
        case .square: return "square"
        case .terminal: return "terminal"
        case .trash: return "trash"
        case .upload: return "square.and.arrow.up"
        case .workflow: return "flowchart"
        case .x: return "xmark"
        case .zap: return "bolt"
        case .globe: return "globe"
        case .sidebar: return "sidebar.left"
        }
    }

    /// A node's glyph in a step list (RunStepsList NODE_ICON).
    public static func node(_ type: WorkflowNodeType) -> Glyph {
        switch type {
        case .trigger: return .zap
        case .launchAgent: return .play
        case .script: return .terminal
        case .condition: return .gitFork
        case .approval: return .hand
        case .createTaskFromItem: return .listPlus
        case .callConnectorAction: return .zap
        case .httpRequest: return .globe
        case .loop: return .repeat
        default: return .zap
        }
    }

    /// The fallback icon of a run whose workflow has none (RunIcon SOURCE_ICON).
    public static func source(_ source: RunSource) -> Glyph {
        switch source {
        case .manual: return .zap
        case .schedule: return .clock
        case .task: return .checkSquare
        case .connector: return .play
        case .restore: return .rotateCcw
        }
    }
}

/// The project icon set a workflow picks from (ICON_MAP), as SF Symbols.
public enum WorkflowIcons {
    public static let symbols: [String: String] = [
        "Folder": "folder",
        "FolderGit2": "folder.badge.gearshape",
        "Code": "chevron.left.forwardslash.chevron.right",
        "Globe": "globe",
        "Database": "cylinder.split.1x2",
        "Server": "server.rack",
        "Smartphone": "iphone",
        "Package": "shippingbox",
        "FileCode": "doc.text",
        "Terminal": "terminal",
        "Cpu": "cpu",
        "Cloud": "cloud",
        "Shield": "shield",
        "Zap": "bolt",
        "Workflow": "flowchart",
        "Gamepad2": "gamecontroller",
        "Music": "music.note",
        "Image": "photo",
        "BookOpen": "book",
        "FlaskConical": "testtube.2",
        "Rocket": "paperplane",
        "Play": "play",
        "github": "arrow.triangle.branch",
        "linear": "line.diagonal",
        "mcp": "puzzlepiece.extension",
    ]

    public static func symbol(_ name: String?) -> String? { name.flatMap { symbols[$0] } }

    /// A workflow's icon, falling back to the Workflow glyph like the sidebar does.
    public static func symbolOrWorkflow(_ name: String?) -> String { symbol(name) ?? Glyph.workflow.symbol }

    public static let defaultColor = Color(hex: 0x6B7280)

    public static func color(_ css: String?) -> Color {
        css.flatMap { Color(cssHex: $0) } ?? defaultColor
    }
}

/// A lucide-sized glyph: `size` is the icon box, like `<Icon size={n} />`.
struct Icon: View {
    let symbol: String
    var size: CGFloat
    var weight: Font.Weight = .regular

    init(_ glyph: Glyph, size: CGFloat, weight: Font.Weight = .regular) {
        self.symbol = glyph.symbol
        self.size = size
        self.weight = weight
    }

    init(symbol: String, size: CGFloat, weight: Font.Weight = .regular) {
        self.symbol = symbol
        self.size = size
        self.weight = weight
    }

    var body: some View {
        if symbol == Glyph.loader.symbol {
            // Loader2: a three-quarter ring, stroked like the other line glyphs.
            Circle()
                .trim(from: 0, to: 0.75)
                .stroke(style: StrokeStyle(lineWidth: size / 12, lineCap: .round))
                .padding(size / 8)
                .frame(width: size, height: size)
        } else {
            Image(systemName: symbol)
                .font(.system(size: size * 0.82, weight: weight))
                .frame(width: size, height: size)
        }
    }
}

/// The renderer's StatusDot: a filled circle coloured by status; running pulses.
struct StatusDot: View {
    let status: WorkflowStatusKey
    var size: CGFloat = 6
    @State private var dim = false

    nonisolated static func color(_ status: WorkflowStatusKey) -> Color {
        switch status {
        case .waiting: return Theme.bronzo
        case .error: return Theme.danger
        case .running: return Theme.ink
        case .success: return Theme.statusSage
        case .pending, .skipped, .cancelled: return Theme.inkGhost
        }
    }

    var body: some View {
        Circle()
            .fill(Self.color(status))
            .frame(width: size, height: size)
            .opacity(status == .running && dim ? 0.5 : 1)
            .onAppear {
                guard status == .running else { return }
                withAnimation(.easeInOut(duration: 1).repeatForever(autoreverses: true)) { dim = true }
            }
    }
}

/// Tracks the pointer so a view can restyle on hover, like `hover:` classes.
struct Hovering<Content: View>: View {
    @ViewBuilder let content: (Bool) -> Content
    @State private var hovered = false

    var body: some View {
        content(hovered).onHover { hovered = $0 }
    }
}

/// `p-1 rounded` icon button: ink-faint, hover ink on white .06, disabled at .4.
struct IconButton: View {
    let glyph: Glyph
    var size: CGFloat = 14
    var help: String
    var disabled = false
    var hoverTint: Color = Theme.ink
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                Icon(glyph, size: size)
                    .foregroundStyle(hovered && !disabled ? hoverTint : Theme.inkFaint)
                    .padding(4)
                    .background(
                        hovered && !disabled ? Theme.white(0.06) : .clear,
                        in: RoundedRectangle(cornerRadius: Theme.radius))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? 0.4 : 1)
            .help(help)
        }
    }
}

/// A keyboard hint: 10px mono ink-faint in a hairline box.
struct Kbd: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 10, design: .monospaced))
            .foregroundStyle(Theme.inkFaint)
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}

/// A 1pt hairline, horizontal unless `vertical`.
struct Hairline: View {
    var opacity: Double
    var vertical = false

    var body: some View {
        Rectangle()
            .fill(Theme.white(opacity))
            .frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
    }
}

/// A spinning glyph, for Loader2 and a refreshing RefreshCw.
struct Spinning<Content: View>: View {
    var active: Bool
    @ViewBuilder let content: Content
    @State private var angle = 0.0

    var body: some View {
        content
            .rotationEffect(.degrees(active ? angle : 0))
            .onAppear { start() }
            .onChange(of: active) { start() }
    }

    private func start() {
        guard active else { return }
        angle = 0
        withAnimation(.linear(duration: 1).repeatForever(autoreverses: false)) { angle = 360 }
    }
}

extension EnvironmentValues {
    /// Lays scrolling regions out flat, for offscreen renders that cannot draw a scroll view.
    @Entry public var workflowsStaticLayout = false
}

/// A scroll view, or its content clipped in place when laid out statically.
struct Scroller<Content: View>: View {
    var axes: Axis.Set = .vertical
    @ViewBuilder let content: Content
    @Environment(\.workflowsStaticLayout) private var isStatic

    var body: some View {
        if isStatic {
            content
                .fixedSize(horizontal: axes.contains(.horizontal), vertical: true)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .clipped()
        } else {
            ScrollView(axes) { content }
        }
    }
}
