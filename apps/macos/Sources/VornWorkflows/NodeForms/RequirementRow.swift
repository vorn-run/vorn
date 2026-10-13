import SwiftUI
import VornCore
import VornUI

/// template-requirements.ts requirementAction, for the fix this build can offer: an HTTP profile made in place.
public enum RequirementFix: Equatable {
    case createProfile(name: String)
    case none

    public static func of(_ r: TemplateRequirement) -> RequirementFix {
        guard r.connectionId == nil, r.requirement.kind == .httpProfile else { return .none }
        return .createProfile(name: r.requirement.name)
    }

    var label: String? {
        if case .createProfile = self { return "Create profile" }
        return nil
    }
}

/// RequirementRow.tsx: what one unmet requirement says, and the button that answers it.
struct RequirementRow: View {
    let requirement: TemplateRequirement
    var onFix: ((RequirementFix) -> Void)?

    var body: some View {
        let fix = RequirementFix.of(requirement)
        HStack(spacing: 8) {
            Text("Needs \(WorkflowFiles.describeRequirement(requirement.requirement))")
                .font(.system(size: 11))
                .foregroundStyle(Theme.bronzo)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let label = fix.label, let onFix {
                Hovering { hovered in
                    Button {
                        onFix(fix)
                    } label: {
                        Text(label)
                            .font(.system(size: 11))
                            .foregroundStyle(hovered ? Color.white : Theme.gray300)
                            .frame(minHeight: 16.5)
                            .padding(.horizontal, 9)
                            .padding(.vertical, 3)
                            .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: 2))
                            .overlay(RoundedRectangle(cornerRadius: 2).strokeBorder(Theme.white(0.1), lineWidth: 1))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(.leading, 38)
        .padding(.trailing, 10)
        .padding(.bottom, 6)
    }
}

/// The HTTP profile form a requirement row opened, under its panel's header.
struct RequirementProfileSlot: View {
    let store: WorkflowsStore
    @Binding var profileFor: String?
    let onDone: () -> Void

    var body: some View {
        if let name = profileFor {
            HttpProfileForm(store: store, name: name) { _ in
                profileFor = nil
                onDone()
            } onCancel: {
                profileFor = nil
            }
            .padding(8)
            .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
            .id(name)
        }
    }
}
