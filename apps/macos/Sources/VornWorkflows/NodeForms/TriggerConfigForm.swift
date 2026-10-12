import AppKit
import SwiftUI
import VornCore
import VornUI

/// TriggerConfigForm.tsx's TRIGGER_TYPES: label, glyph and hint per trigger type.
enum TriggerKinds {
    struct Kind {
        let type: String
        let label: String
        let glyph: FormGlyph
        let hint: String
    }

    static let all: [Kind] = [
        .init(type: "manual", label: "Manual", glyph: .lucide(.zap), hint: "Run manually from the play button"),
        .init(type: "once", label: "Once", glyph: .lucide(.clock), hint: "Runs once at the scheduled time"),
        .init(type: "recurring", label: "Recurring", glyph: .lucide(.refreshCw), hint: "Runs on a repeating schedule"),
        .init(type: "taskCreated", label: "Task Created", glyph: .lucide(.listPlus), hint: "Fires when a new task is added"),
        .init(
            type: "taskStatusChanged", label: "Status Change", glyph: .lucide(.arrowRightLeft),
            hint: "Fires when a task's status changes"),
        .init(
            type: "sessionRestored", label: "Session Restored", glyph: .lucide(.rotateCcw),
            hint: "Fires when a session comes back after a quit, a crash or a reboot"),
        .init(
            type: "connectorPoll", label: "Connector Poll", glyph: .lucide(.plug),
            hint: "Polls an external connector on cron and fires per new item"),
        .init(
            type: "webhook", label: "Webhook", glyph: .lucide(.globe),
            hint: "Fires when this machine receives an HTTP request at the workflow URL"),
    ]

    static func kind(_ type: String?) -> Kind? { all.first { $0.type == type } }

    static let cronPresets: [(label: String, value: String)] = [
        ("Weekdays 9am", "0 9 * * 1-5"),
        ("Daily 9am", "0 9 * * *"),
        ("Every hour", "0 * * * *"),
        ("Every 30min", "*/30 * * * *"),
        ("Weekly (Mon 9am)", "0 9 * * 1"),
    ]

    static let statusOptions: [(value: String, label: String)] = [
        ("", "Any status"), ("todo", "To Do"), ("in_progress", "In Progress"), ("in_review", "In Review"),
        ("done", "Done"), ("cancelled", "Cancelled"),
    ]
}

/// The edits TriggerConfigForm.tsx makes, as pure functions over the config.
enum TriggerEdits {
    static func toggleContextual(_ c: JSONValue) -> JSONValue {
        c.merged(["contextual": c.bool("contextual") == true ? nil : .bool(true)])
    }

    /// An empty list removes `inputs`.
    static func setInputs(_ c: JSONValue, _ inputs: [JSONValue]) -> JSONValue {
        c.merged(["inputs": inputs.isEmpty ? nil : .array(inputs)])
    }

    static func setProjectFilter(_ c: JSONValue, _ name: String) -> JSONValue {
        c.merged(["projectFilter": name.isEmpty ? nil : .string(name)])
    }

    /// `cold` is the default and is not stored.
    static func setRestore(_ c: JSONValue, _ v: String) -> JSONValue {
        c.merged(["restore": v == "any" ? .string("any") : nil])
    }

    static func setConcurrency(_ c: JSONValue, _ v: String) -> JSONValue {
        c.merged(["concurrency": v == "unbounded" ? .string("unbounded") : nil])
    }

    static func setStatus(_ c: JSONValue, key: String, _ v: String) -> JSONValue {
        c.merged([key: v.isEmpty ? nil : .string(v)])
    }

    /// A once trigger keeps only its time.
    static func setRunAt(_ date: Date) -> JSONValue {
        ["triggerType": "once", "runAt": .string(FormHelpers.isoString(date))]
    }

    /// The datetime-local field shows `runAt`'s UTC wall time and reads what is typed as local time.
    static func runAtFromField(_ shownAsUTC: Date, zone: TimeZone = .current) -> Date {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        var local = Calendar(identifier: .gregorian)
        local.timeZone = zone
        let parts = utc.dateComponents([.year, .month, .day, .hour, .minute], from: shownAsUTC)
        return local.date(from: parts) ?? shownAsUTC
    }

    /// The field's text: `MM/dd/yyyy, hh:mm AM` of the UTC wall time.
    static func runAtText(_ date: Date) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = "MM/dd/yyyy, hh:mm a"
        return f.string(from: date)
    }

    /// ConnectorPollTriggerForm.tsx: a new connection clears the event.
    static func setPollConnection(_ c: JSONValue, _ id: String) -> JSONValue {
        c.merged(["connectionId": .string(id), "event": .string("")])
    }

    static func webhookURL(base: String?, workflowId: String?, token: String?) -> String? {
        guard let base, let workflowId else { return nil }
        return "\(base)/wf-hooks/\(workflowId)/\(token ?? "undefined")"
    }
}

/// TriggerConfigForm.tsx.
struct TriggerConfigForm: View {
    let store: WorkflowsStore
    let data: NodeFormsData
    @Binding var config: JSONValue
    var onOpenLibrary: (() -> Void)?

    private var triggerType: String { config.str("triggerType") ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            typeSection
            switch triggerType {
            case "manual":
                SwitchCard(
                    isOn: config.bool("contextual") == true, symbol: .zap, title: "Contextual",
                    detail: "Run this workflow directly from any card or terminal, against that session's folder and branch."
                ) {
                    config = TriggerEdits.toggleContextual(config)
                }
                WorkflowInputsEditor(inputs: config["inputs"]?.arrayValue ?? []) {
                    config = TriggerEdits.setInputs(config, $0)
                }
            case "once": onceFields
            case "recurring": recurringFields
            case "taskCreated":
                FormSection("Project Filter", hint: "Only trigger for tasks in this project") { projectFilter }
            case "connectorPoll": ConnectorPollTriggerForm(data: data, config: $config)
            case "sessionRestored": sessionRestoredFields
            case "webhook": webhookFields
            case "taskStatusChanged": statusChangedFields
            default: EmptyView()
            }
        }
    }

    private var typeSection: some View {
        let kind = TriggerKinds.kind(triggerType)
        return VStack(alignment: .leading, spacing: 0) {
            FormLabel("Trigger Type")
            HStack(spacing: 8) {
                if let kind { FormIcon(kind.glyph, size: 13).foregroundStyle(Theme.gray400) }
                Text(kind?.label ?? triggerType).font(.system(size: 13)).foregroundStyle(Theme.gray200).cssLineHeight(13)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Theme.white(0.02), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
            if let kind { FormHint(kind.hint, top: 6) }
            if let onOpenLibrary {
                Hovering { hovered in
                    Button(action: onOpenLibrary) {
                        Text("Change trigger from the library")
                            .font(.system(size: 11))
                            .underline()
                            .foregroundStyle(hovered ? Theme.gray300 : Theme.gray500)
                            .cssLineHeight(11)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.top, 8)
            }
        }
    }

    private var projectFilter: some View {
        FormProjectPicker(current: config.str("projectFilter") ?? "", projects: store.projects, allowNone: true) {
            config = TriggerEdits.setProjectFilter(config, $0)
        }
    }

    private var onceFields: some View {
        FormSection("Run At") {
            RunAtField(date: config.str("runAt").flatMap(FormHelpers.parseISO)) {
                config = TriggerEdits.setRunAt(TriggerEdits.runAtFromField($0))
            }
        }
    }

    @ViewBuilder private var recurringFields: some View {
        FormSection("Preset") {
            FormSelect(
                value: config.str("cron") ?? "",
                options: TriggerKinds.cronPresets.map { SelectOption($0.value, $0.label) },
                placeholder: "Choose preset..."
            ) { config.set("cron", string: $0) }
        }
        FormSection("Cron Expression", hint: "min hour day month weekday") {
            FormTextField(placeholder: "* * * * *", text: configText("cron"), mono: true)
        }
        FormSection("Timezone") {
            FormTextField(
                placeholder: "",
                text: Binding(
                    get: {
                        let tz = config.str("timezone") ?? ""
                        return tz.isEmpty ? TimeZone.current.identifier : tz
                    },
                    set: { config.set("timezone", string: $0) }))
        }
    }

    @ViewBuilder private var sessionRestoredFields: some View {
        FormSection("Project Filter", hint: "Without one, this fires in every project on every start-up") {
            projectFilter
        }
        FormSection("Which restores", hint: "Cold: a process started again. Warm: a pane attached to one still running.") {
            FormSegmented(
                value: config.str("restore") ?? "cold", options: [("cold", "Cold only"), ("any", "Warm too")]
            ) { config = TriggerEdits.setRestore(config, $0) }
        }
        FormSection("At a time", hint: "Eight sessions in one project make one run, then the next") {
            FormSegmented(
                value: config.str("concurrency") ?? "perProject",
                options: [("perProject", "One per project"), ("unbounded", "Any number")]
            ) { config = TriggerEdits.setConcurrency(config, $0) }
        }
    }

    @ViewBuilder private var statusChangedFields: some View {
        FormSection("Project Filter") { projectFilter }
        ForEach([("fromStatus", "From Status"), ("toStatus", "To Status")], id: \.0) { key, label in
            FormSection(label) {
                FormSelect(
                    value: config.str(key) ?? "",
                    options: TriggerKinds.statusOptions.map { SelectOption($0.value, $0.label) },
                    placeholder: "Any status"
                ) { config = TriggerEdits.setStatus(config, key: key, $0) }
            }
        }
    }

    @ViewBuilder private var webhookFields: some View {
        let url = TriggerEdits.webhookURL(
            base: data.webhookBaseURL, workflowId: store.editingWorkflowId, token: config.str("token"))
        VStack(alignment: .leading, spacing: 0) {
            FormLabel("URL")
            if let url {
                WebhookURLRow(url: url)
            } else {
                FormHint("Save the workflow first — the URL includes its id.", top: 0)
            }
            FormHint(
                "Local machine only, and only while the app is open. Exposing it is a deliberate, separate step.",
                top: 6)
        }
        FormSection("Method") {
            FormSelect(value: config.str("method") ?? "", options: [SelectOption("POST", "POST"), SelectOption("GET", "GET")]) {
                config.set("method", string: $0)
            }
            FormHint("The request lands as {{trigger.body.*}}, {{trigger.headers.*}}, and {{trigger.query.*}}.", top: 6)
        }
    }

    private func configText(_ key: String) -> Binding<String> {
        Binding(get: { config.str(key) ?? "" }, set: { config.set(key, string: $0) })
    }
}

/// The webhook URL with its copy button.
private struct WebhookURLRow: View {
    let url: String
    @State private var copied = false

    var body: some View {
        HStack(spacing: 8) {
            Text(url)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Theme.gray300)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fieldChrome()
            Hovering { hovered in
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(url, forType: .string)
                    copied = true
                    Task {
                        try? await Task.sleep(for: .milliseconds(1500))
                        copied = false
                    }
                } label: {
                    Group {
                        if copied { LucideIcon(.check, size: 12) } else { Image(systemName: "doc.on.doc").font(.system(size: 10)) }
                    }
                    .frame(width: 12, height: 12)
                    .foregroundStyle(hovered ? Color.white : Theme.gray400)
                    .padding(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(hovered ? 0.2 : 0.1), lineWidth: 1))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Copy URL")
            }
        }
    }
}

/// The once trigger's datetime-local field: it shows the UTC wall time, like `toISOString().slice(0, 16)`.
private struct RunAtField: View {
    let date: Date?
    let onChange: (Date) -> Void
    @Environment(\.workflowsStaticLayout) private var isStatic

    var body: some View {
        HStack(spacing: 8) {
            if isStatic {
                Text(date.map(TriggerEdits.runAtText) ?? "mm/dd/yyyy, --:-- --")
                    .font(.system(size: 13))
                    .foregroundStyle(.white)
                    .frame(maxWidth: .infinity, minHeight: 19.5, alignment: .leading)
            } else {
                DatePicker(
                    "", selection: Binding(get: { date ?? Date() }, set: onChange),
                    displayedComponents: [.date, .hourAndMinute]
                )
                .labelsHidden()
                .datePickerStyle(.field)
                .environment(\.timeZone, TimeZone(identifier: "UTC")!)
                .colorScheme(.dark)
                .frame(maxWidth: .infinity, minHeight: 19.5, alignment: .leading)
            }
            LucideIcon(.calendar, size: 13).foregroundStyle(Color.white)
        }
        .fieldChrome()
    }
}

/// ConnectorPollTriggerForm.tsx.
struct ConnectorPollTriggerForm: View {
    let data: NodeFormsData
    @Binding var config: JSONValue

    var body: some View {
        let selected = data.connection(config.str("connectionId"))
        let connector = selected.flatMap { c in data.connectors.first { $0.id == c.connectorId } }
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Connection") {
                FormSelect(
                    value: config.str("connectionId") ?? "",
                    options: data.connections.map { c in
                        SelectOption(c.id, c.name, icon: AnyView(ConnectorMark(connectorId: c.realConnectorId)))
                    },
                    placeholder: "Select a connection..."
                ) { config = TriggerEdits.setPollConnection(config, $0) }
                if data.connections.isEmpty {
                    FormHint("No connections yet. Add one from Settings › Connectors.", top: 6)
                }
            }
            if let connector {
                FormSection("Event") {
                    FormSelect(
                        value: config.str("event") ?? "",
                        options: connector.triggers.map { SelectOption($0.type, $0.label) },
                        placeholder: "Select an event..."
                    ) { config.set("event", string: $0) }
                }
            }
            FormSection("Cron", hint: "min hour day month weekday · default every 5 minutes") {
                BareTextInput(
                    placeholder: "*/5 * * * *",
                    text: Binding(get: { config.str("cron") ?? "" }, set: { config.set("cron", string: $0) }),
                    mono: true
                )
                .fieldChrome()
                .clipShape(RoundedRectangle(cornerRadius: 2))
            }
        }
    }
}
