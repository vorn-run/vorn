import SwiftUI
import VornCore
import VornUI

/// DeviceCard.tsx: a session's simulator, framed, with the controls a person needs when the agent is not driving.
public struct DeviceCard: View {
    @Bindable var panes: PanesModel
    @Bindable var model: BrowserModel
    let sessionID: String

    public init(panes: PanesModel, sessionID: String) {
        self.panes = panes
        self.model = panes.browser
        self.sessionID = sessionID
    }

    public var body: some View {
        let s = sessionID
        if let state = model.devices[s] {
            let name = model.claims[s]?.name ?? "Simulator"
            let switching = model.claiming.contains(s)
            VStack(spacing: 0) {
                HStack(spacing: 6) {
                    LucideIcon(.smartphone, size: 12, strokeWidth: 2).foregroundStyle(Theme.gray500)
                    Hovering { hover in
                        Button { if !switching { model.switchDevice(s) } } label: {
                            HStack(spacing: 4) {
                                Text(name).font(.ui(11, weight: .medium)).foregroundStyle(Theme.gray300).lineLimit(1)
                                if switching {
                                    SpinningGlyph(glyph: .loaderCircle, size: 10, strokeWidth: 2.5, active: true)
                                } else {
                                    LucideIcon(.chevronDown, size: 10, strokeWidth: 2.5).foregroundStyle(Theme.gray500)
                                }
                            }
                            .padding(.horizontal, 4).padding(.vertical, 2)
                            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.06) : .clear))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help("Switch simulator")
                        .accessibilityLabel("Switch simulator, currently \(name)")
                    }
                    .popover(isPresented: model.pickerBinding(s, fromPane: true), arrowEdge: .bottom) { DevicePickerList(model: model) }
                    Spacer(minLength: 0)
                    PaneIconButton(icon: .mousePointerClick, help: "Point at an element to describe it to this session's agent") {
                        model.deviceControl(s, "pick", area: nil)
                    }
                    .accessibilityLabel("Point at an element for the agent")
                    PaneIconButton(icon: .pencil, help: "Draw over the screen, then click again to send it to this session's agent") {
                        model.deviceControl(s, "ink", area: nil)
                    }
                    .accessibilityLabel("Draw on the screen for the agent")
                    PaneControls(panes: panes, paneID: PaneID.device(s), title: name, onClose: { model.closeDevicePane(s) })
                }
                .padding(.horizontal, 8).padding(.vertical, 4)
                if let error = state.error {
                    HStack(alignment: .top, spacing: 4) {
                        Text(error).font(.ui(10)).foregroundStyle(Theme.amber400.opacity(0.9)).frame(maxWidth: .infinity, alignment: .leading)
                            .fixedSize(horizontal: false, vertical: true)
                        IconButton(.x, size: 10, strokeWidth: 2.5, padding: 2, color: Theme.gray500, hoverColor: Theme.gray200,
                                   help: "Dismiss error") { model.dismissDeviceError(s) }
                    }
                    .padding(.horizontal, 8).padding(.vertical, 4)
                }
                GeometryReader { geo in
                    let area = geo.size
                    ScrollView([.horizontal, .vertical]) {
                        stage(s, name: name, area: area, stale: state.error != nil)
                            .frame(minWidth: area.width, minHeight: area.height)
                    }
                    .onAppear { model.deviceAreas[s] = area }
                    .onChange(of: area) { _, a in model.deviceAreas[s] = a }
                }
                .background(Theme.surfacePanel)
                DeviceControlBar(model: model, s: s, state: state)
            }
            .background(Theme.surfacePanel)
        }
    }

    @ViewBuilder
    private func stage(_ s: String, name: String, area: CGSize, stale: Bool) -> some View {
        let (scale, points) = model.deviceScale(s, area: area)
        if let frame = model.frames[s], points.width > 0 {
            let drawn = CGSize(width: points.width * scale, height: points.height * scale)
            let inset = Devices.bezelThickness(area: area, points: points)
            let outer = Devices.outerRadius(drawn: drawn, points: points)
            Image(nsImage: frame.image)
                .resizable()
                .interpolation(.high)
                .frame(width: drawn.width, height: drawn.height)
                .clipShape(RoundedRectangle(cornerRadius: max(outer - inset, 0), style: .continuous))
                .opacity(stale ? 0.4 : 1)
                .accessibilityLabel("Screen of \(name)")
                .padding(inset)
                .background(RoundedRectangle(cornerRadius: outer, style: .continuous).fill(Theme.surfaceOverlay))
                .overlay(RoundedRectangle(cornerRadius: outer, style: .continuous).strokeBorder(Theme.white(0.07)))
                .shadow(color: .black.opacity(0.45), radius: 16, y: 12)
                .padding(8)
        } else {
            Text(stale ? "No frame" : "Waiting for the device\u{2026}").font(.ui(11)).foregroundStyle(Theme.gray500)
        }
    }
}

/// DeviceControlBar.tsx: home, lock, screenshot, keyboard, rotate and zoom.
struct DeviceControlBar: View {
    let model: BrowserModel
    let s: String
    let state: DevicePaneState

    var body: some View {
        let area = model.deviceAreas[s]
        let scale = model.deviceScale(s, area: area).scale
        HStack(spacing: 2) {
            PaneIconButton(icon: .house, help: "Home") { model.deviceControl(s, "home", area: area) }.accessibilityLabel("Press Home")
            PaneIconButton(icon: .lock, help: "Lock") { model.deviceControl(s, "lock", area: area) }.accessibilityLabel("Press Lock")
            divider
            if state.saving {
                SpinningGlyph(glyph: .loaderCircle, size: 14, strokeWidth: 2, active: true).padding(4).opacity(0.5)
            } else {
                PaneIconButton(icon: .imageDown, help: "Save a screenshot") { model.deviceControl(s, "save", area: area) }
            }
            PaneIconButton(icon: .keyboard, help: "Type on the device") { model.deviceControl(s, "keyboard", area: area) }
            PaneIconButton(icon: .rotateCw, help: "Rotate") { model.deviceControl(s, "rotate", area: area) }
                .accessibilityLabel("Rotate the device")
            divider
            PaneIconButton(icon: .minus, help: "Zoom out", disabled: scale <= Devices.zoomMin) { model.deviceControl(s, "zoom-out", area: area) }
            Text("\(Int(DesignCanvas.jsRound(scale * 100)))%").font(.ui(10).monospacedDigit()).foregroundStyle(Theme.gray500)
                .frame(width: 36)
            PaneIconButton(icon: .plus, help: "Zoom in", disabled: scale >= Devices.zoomMax) { model.deviceControl(s, "zoom-in", area: area) }
            PaneIconButton(icon: .scan, help: "Fit the device to the pane", active: state.zoom == nil) {
                model.deviceControl(s, "fit", area: area)
            }
            Hovering { hover in
                Button { model.deviceControl(s, "actual", area: area) } label: {
                    Text("1:1").font(.ui(10, weight: .medium)).foregroundStyle(Theme.ink).frame(width: 22, height: 22)
                        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.10) : .clear))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .help("One pixel per point")
            .accessibilityLabel("Show the device at actual size")
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 8).padding(.vertical, 4)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private var divider: some View { Rectangle().fill(Theme.white(0.08)).frame(width: 1, height: 14).padding(.horizontal, 4) }
}

/// DevicePicker.tsx: the simulators a session can take.
public struct DevicePickerList: View {
    let model: BrowserModel

    public init(model: BrowserModel) { self.model = model }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("SIMULATORS").font(.ui(10)).tracking(0.25).foregroundStyle(Theme.gray500)
                .padding(.horizontal, 12).padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            if let p = model.picker {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        if p.loading {
                            SpinningGlyph(glyph: .loaderCircle, size: 14, strokeWidth: 2, active: true).foregroundStyle(Theme.gray500)
                                .frame(maxWidth: .infinity).padding(.vertical, 12)
                        } else if let error = p.error {
                            Text(error).font(.ui(11)).foregroundStyle(Theme.red400.opacity(0.8)).padding(.horizontal, 12).padding(.vertical, 8)
                                .fixedSize(horizontal: false, vertical: true)
                        } else if p.devices.isEmpty {
                            Text("No simulators found").font(.ui(12)).foregroundStyle(Theme.gray500).padding(.horizontal, 12).padding(.vertical, 8)
                        } else {
                            ForEach(p.devices, id: \.device.udid) { row($0, session: p.session) }
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
        }
        .frame(minWidth: 220, maxWidth: 320)
        .frame(maxHeight: 280)
        .fixedSize(horizontal: false, vertical: true)
        .background(Theme.surfaceOverlay)
    }

    private func row(_ d: Devices.Listed, session: String) -> some View {
        Hovering { hover in
            Button { if d.selectable { model.pickDevice(session, d.device.udid) } } label: {
                HStack(spacing: 8) {
                    LucideIcon(.smartphone, size: 11, strokeWidth: 2)
                        .foregroundStyle(d.device.booted ? Theme.green400.opacity(0.7) : Theme.gray600)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(d.device.name).font(.ui(12)).lineLimit(1)
                        if let sub = d.subtitle { Text(sub).font(.ui(9)).foregroundStyle(Theme.gray500).lineLimit(1) }
                    }
                    Spacer(minLength: 0)
                    if !d.selectable {
                        Text("in use").font(.ui(9)).foregroundStyle(Theme.amber400.opacity(0.6))
                    } else if d.device.booted {
                        Text("booted").font(.ui(9)).foregroundStyle(Theme.green400.opacity(0.6))
                    }
                }
                .foregroundStyle(d.selectable ? Theme.gray300 : Theme.gray600)
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(d.selectable && hover ? Theme.white(0.06) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!d.selectable)
            .help(d.selectable ? d.device.runtime : "In use by \(d.device.claimedBy ?? "another session")")
        }
    }
}

/// The card's phone button (CardActionCluster): hides the pane, else opens the picker; spins while a claim boots.
public struct DeviceCardButton: View {
    let model: BrowserModel
    let session: TerminalSession

    public init(model: BrowserModel, session: TerminalSession) {
        self.model = model
        self.session = session
    }

    public var body: some View {
        if model.showsDeviceButton(session) {
            let claiming = model.claiming.contains(session.id)
            let shown = model.hasDevicePane(session.id)
            Group {
                if claiming {
                    SpinningGlyph(glyph: .loaderCircle, size: 14, strokeWidth: 2, active: true)
                        .foregroundStyle(Theme.ink)
                        .padding(4)
                        .opacity(0.5)
                        .help("Booting the simulator…")
                } else {
                    IconButton(.smartphone, size: 14, padding: 4, radius: Theme.radius, color: Theme.ink, hoverColor: Theme.ink,
                               hoverFill: Theme.white(0.10), help: shown ? "Hide device" : "Open device") {
                        model.deviceButton(session.id)
                    }
                }
            }
            .popover(isPresented: model.pickerBinding(session.id, fromPane: false), arrowEdge: .bottom) {
                DevicePickerList(model: model)
            }
        }
    }
}
