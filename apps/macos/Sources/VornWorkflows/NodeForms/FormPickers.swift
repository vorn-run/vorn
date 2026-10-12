import SwiftUI
import VornCore
import VornUI

/// ProjectPicker.tsx, the form variant: a project, None when `allowNone`, From Context when offered.
struct FormProjectPicker: View {
    let current: String
    let projects: [Project]
    var allowNone = false
    var isFromContext = false
    var onSelectFromContext: (() -> Void)?
    let onChange: (String) -> Void
    @State private var open = false

    var body: some View {
        let project = projects.first { $0.name == current }
        PickerTrigger(
            text: isFromContext ? "From Context" : (current.isEmpty ? nil : current),
            placeholder: "Select project...", open: open
        ) {
            if isFromContext {
                LucideIcon(.zap, size: 13, strokeWidth: 1.5).foregroundStyle(Theme.blue400)
            } else {
                ProjectPickerIcon(icon: project?.icon, color: project?.iconColor)
            }
        } action: {
            open.toggle()
        }
        .formPopup(isPresented: $open) {
            PopupList {
                if let onSelectFromContext {
                    CheckTrailingRow(label: "From Context", selected: isFromContext) {
                        LucideIcon(.zap, size: 13, strokeWidth: 1.5).foregroundStyle(Theme.blue400)
                    } action: {
                        open = false
                        onSelectFromContext()
                    }
                    Hairline(opacity: 0.06)
                }
                if allowNone {
                    CheckTrailingRow(label: "None", selected: current.isEmpty && !isFromContext, italic: true, muted: true) {
                        LucideIcon(.folder, size: 13).foregroundStyle(Theme.gray600)
                    } action: {
                        select("")
                    }
                }
                ForEach(projects, id: \.name) { p in
                    CheckTrailingRow(label: p.name, selected: !isFromContext && p.name == current) {
                        ProjectPickerIcon(icon: p.icon, color: p.iconColor)
                    } action: {
                        select(p.name)
                    }
                }
            }
        }
    }

    private func select(_ name: String) {
        open = false
        if name != current { onChange(name) }
    }
}
