import AppKit
import SwiftUI

/// The sessions view with nothing open: top bar, logo, composer and hint.
/// Spacing follows the Tailwind classes in App.tsx, AppNavCluster.tsx,
/// MainViewPills.tsx, SessionDock.tsx and PromptLauncher.tsx (1 unit = 4pt).
struct MainScreen: View {
    /// Draws stand-in window buttons for offscreen renders, where there is no
    /// real window to supply them.
    var fauxChrome = false
    var sessionCount = 2

    var body: some View {
        VStack(spacing: 0) {
            TopBar(sessionCount: sessionCount)
            PromptLauncher()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Theme.surfaceBase)
        .overlay(alignment: .topLeading) {
            if fauxChrome { FauxTrafficLights() }
        }
        .environment(\.colorScheme, .dark)
    }
}

// MARK: - Top bar

private struct TopBar: View {
    let sessionCount: Int

    var body: some View {
        HStack(spacing: 0) {
            HStack(spacing: 4) {
                ToolbarIconButton(symbol: "sidebar.left", size: 14, weight: .regular)
                VDivider()
                ViewPills()
                SessionsChip(count: sessionCount)
            }
            Spacer(minLength: 0)
            HStack(spacing: 4) {
                ToolbarIconButton(symbol: "slider.horizontal.3", size: 14, weight: .light)
                VDivider()
                ToolbarIconButton(symbol: "arrow.counterclockwise", size: 13, weight: .light)
                ToolbarIconButton(symbol: "plus", size: 14, weight: .regular)
            }
        }
        .padding(.leading, Theme.trafficLightPad)
        .padding(.trailing, 12)
        .frame(height: Theme.toolbarHeight)
        .background(Theme.surfaceBase)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.white(0.06)).frame(height: 1)
        }
    }
}

/// `p-1` around a 16px lucide glyph, gray-400.
private struct ToolbarIconButton: View {
    let symbol: String
    let size: CGFloat
    let weight: Font.Weight

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: size, weight: weight))
            .foregroundStyle(Theme.gray400)
            .frame(width: 24, height: 24)
            .contentShape(Rectangle())
    }
}

/// `w-px h-4 bg-white/[0.06] mx-0.5`
private struct VDivider: View {
    var body: some View {
        Rectangle()
            .fill(Theme.white(0.06))
            .frame(width: 1, height: 16)
            .padding(.horizontal, 2)
    }
}

private struct ViewPills: View {
    enum Mode: CaseIterable { case sessions, tasks, workflows }
    var active: Mode = .sessions

    var body: some View {
        HStack(spacing: 2) {
            pill(.sessions, symbol: "display")
            pill(.tasks, symbol: "checklist")
            pill(.workflows, symbol: "flowchart")
        }
        .padding(2)
        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
    }

    private func pill(_ mode: Mode, symbol: String) -> some View {
        let isActive = mode == active
        return Image(systemName: symbol)
            .font(.system(size: 12, weight: .medium))
            .frame(width: 14, height: 14)
            .foregroundStyle(isActive ? Color.white : Theme.gray500)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(
                isActive ? Theme.white(0.1) : .clear,
                in: RoundedRectangle(cornerRadius: Theme.radiusMd)
            )
    }
}

/// SessionDock's collapsed chip: Layers glyph plus a mono count.
private struct SessionsChip: View {
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "square.3.layers.3d")
                .font(.system(size: 10, weight: .regular))
            Text("\(count)")
                .font(.system(size: 11, weight: .medium, design: .monospaced))
        }
        .foregroundStyle(Theme.gray300)
        .padding(.horizontal, 8)
        .frame(height: 26)
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.radiusMd)
                .strokeBorder(Theme.white(0.06), lineWidth: 1)
        )
    }
}

// MARK: - Composer

private struct PromptLauncher: View {
    var body: some View {
        VStack(spacing: 0) {
            VornLogo()
                .frame(height: 32)
                .opacity(0.5)
                .padding(.bottom, 24)
            Composer()
            HintLine()
                .padding(.top, 8)
        }
        .frame(maxWidth: 800)
        .padding(.horizontal, 16)
    }
}

private struct VornLogo: View {
    var body: some View {
        if let url = Bundle.module.url(forResource: "vorn-logo", withExtension: "png"),
           let image = NSImage(contentsOf: url) {
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .scaledToFit()
        }
    }
}

private struct Composer: View {
    var body: some View {
        ZStack(alignment: .bottom) {
            // textarea: rows=3 at text-sm (20pt line), pt-4 pb-12 → 124pt tall.
            Text("Describe your task...")
                .font(.system(size: 14))
                .foregroundStyle(Theme.gray600)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .frame(height: 124, alignment: .top)

            SettingsBar()
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .overlay(alignment: .top) {
                    Rectangle().fill(Theme.white(0.04)).frame(height: 1)
                }
        }
        .padding(1)
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.radiusXl)
                .strokeBorder(Theme.white(0.06), lineWidth: 1)
        )
    }
}

private struct SettingsBar: View {
    var body: some View {
        HStack(spacing: 4) {
            Chip(symbol: "folder", symbolSize: 11, label: "vorn", tint: Theme.gray300)
            Chip(symbol: "sparkle", symbolSize: 12, label: "Agent", tint: Theme.gray400)
            Chip(
                symbol: "cpu", symbolSize: 11, label: "Default",
                tint: Theme.gray500, symbolTint: Theme.gray500
            )
            Chip(symbol: "folder.badge.gearshape", symbolSize: 11, label: nil, tint: Theme.gray600, symbolWeight: .light)
            Chip(symbol: "arrow.triangle.branch", symbolSize: 10, label: "main", tint: Theme.gray400)
            Spacer(minLength: 0)
            SendButton()
        }
    }
}

/// `flex items-center gap-1.5 px-2 py-1 rounded-md text-xs` + ChevronDown 10.
private struct Chip: View {
    let symbol: String
    let symbolSize: CGFloat
    let label: String?
    let tint: Color
    var symbolTint: Color?
    var symbolWeight: Font.Weight = .regular

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol)
                .font(.system(size: symbolSize, weight: symbolWeight))
                .foregroundStyle(symbolTint ?? tint)
                .frame(width: 14, height: 14)
            if let label {
                Text(label)
                    .font(.system(size: 12))
                    .lineLimit(1)
            }
            Image(systemName: "chevron.down")
                .font(.system(size: 7, weight: .semibold))
        }
        .foregroundStyle(tint)
        .padding(.horizontal, 8)
        .frame(height: 24)
        .contentShape(RoundedRectangle(cornerRadius: Theme.radiusMd))
    }
}

/// `p-1.5 rounded-full bg-ink text-surface-base` around ArrowUp 14 / stroke 2.5.
private struct SendButton: View {
    var body: some View {
        Image(systemName: "arrow.up")
            .font(.system(size: 11, weight: .bold))
            .foregroundStyle(Theme.surfaceBase)
            .frame(width: 26, height: 26)
            .background(Theme.ink, in: Circle())
    }
}

private struct HintLine: View {
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "lightbulb")
                .font(.system(size: 10))
                .foregroundStyle(Theme.inkFaint)
            Text("⌘B")
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(Theme.gray500)
                .padding(.horizontal, 4)
                .padding(.vertical, 2)
                .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
            Text("Toggle the sidebar to focus on your grid")
                .font(.system(size: 11))
                .foregroundStyle(Theme.gray600)
        }
    }
}

// MARK: - Offscreen chrome

/// Close/minimise/zoom at the position the Electron window asks for
/// (`trafficLightPosition: { x: 16, y: 13 }`), for renders without a window.
private struct FauxTrafficLights: View {
    var body: some View {
        HStack(spacing: 8) {
            light(0xFF5F57, 0xE14640)
            light(0xFEBC2E, 0xDFA023)
            light(0x28C840, 0x1AAB29)
        }
        .padding(.leading, 16)
        .padding(.top, 14)
    }

    private func light(_ fill: UInt32, _ edge: UInt32) -> some View {
        Circle()
            .fill(Color(hex: fill))
            .overlay(Circle().strokeBorder(Color(hex: edge), lineWidth: 0.5))
            .frame(width: 12, height: 12)
    }
}
