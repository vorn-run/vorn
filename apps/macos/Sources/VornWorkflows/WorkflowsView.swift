import SwiftUI
import VornCore
import VornUI

/// The Workflows main view: the runs landing, or one workflow's page when opened.
/// The shell owns the top bar (mount `WorkflowsHeader` in it), the sidebar (`WorkflowsSidebarSection`),
/// and applies `.workflowsMenuHost(store)` at its window root so dropdowns float over both.
/// The store keeps following vornd after this view goes away; call `store.stop()` when done with it.
public struct WorkflowsView<Leading: View>: View {
    @Bindable var store: WorkflowsStore
    let leading: Leading

    /// `leading` sits at the start of a workflow page's bar, for the sidebar toggle and view pills.
    public init(store: WorkflowsStore, @ViewBuilder leading: () -> Leading) {
        self.store = store
        self.leading = leading()
    }

    public var body: some View {
        Group {
            if let workflow = store.editingWorkflow {
                WorkflowPage(store: store, workflow: workflow) { leading }
            } else {
                WorkflowsLandingView(store: store)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.surfaceBase)
        .overlay(alignment: .bottomTrailing) { ToastView(store: store).padding(20) }
        .task { await store.start() }
    }
}

extension WorkflowsView where Leading == EmptyView {
    public init(store: WorkflowsStore) {
        self.init(store: store) { EmptyView() }
    }
}

/// The app's error toast: red X, gray-200 text, on a dark rounded card; clears after 4s.
struct ToastView: View {
    let store: WorkflowsStore

    var body: some View {
        if let message = store.toast {
            HStack(spacing: 10) {
                Icon(.x, size: 15, weight: .bold).foregroundStyle(Color(hex: 0xFF6467))
                Text(message)
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.gray200)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Hovering { hovered in
                    Button { store.toast = nil } label: {
                        Icon(.x, size: 12, weight: .semibold)
                            .foregroundStyle(hovered ? Theme.gray300 : Theme.gray500)
                            .padding(2)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .frame(minWidth: 200, maxWidth: 360)
            .fixedSize(horizontal: false, vertical: true)
            .background(Color(hex: 0xFB2C36).opacity(0.1), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .background(Color(red: 26 / 255, green: 26 / 255, blue: 30 / 255).opacity(0.92), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Color(hex: 0xFB2C36).opacity(0.2), lineWidth: 1))
            .shadow(color: .black.opacity(0.25), radius: 12, y: 8)
            .task(id: message) {
                try? await Task.sleep(for: .seconds(4))
                if store.toast == message { store.toast = nil }
            }
        }
    }
}
