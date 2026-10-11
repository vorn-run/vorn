import SwiftUI
import VornCore
import VornUI

/// WorkflowPropertiesPanel.tsx: the workflow's own settings, edited on the draft.
struct WorkflowPropertiesPanel: View {
    let store: WorkflowsStore
    @Binding var workflow: WorkflowDefinition
    let onClose: () -> Void

    var body: some View {
        Color.clear.frame(width: 420)
    }
}
