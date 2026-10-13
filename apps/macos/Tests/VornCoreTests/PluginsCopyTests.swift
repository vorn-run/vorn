import Foundation
import Testing
@testable import VornCore

@Suite struct PluginsCopyTests {
    @Test func permissionsGroupIntoBucketsAndListWhatIsNotAsked() {
        let rows = ExtensionCopy.permissionRows([.terminalSend, .gitRead, .terminalRead])
        #expect(rows.map(\.label) == [.reads, .sends])
        #expect(rows.first?.items == ["the worktree's diff and status", "the session's recent terminal output"])
        #expect(ExtensionCopy.permissionRows(nil).isEmpty)
        let missing = ExtensionCopy.notAsked([.gitRead])
        #expect(!missing.contains("the worktree's diff and status"))
        #expect(missing.contains("the session card"))
    }

    @Test func activationReadsAsClauses() {
        #expect(ExtensionCopy.describeActivation(nil) == ["every session"])
        let a = ExtensionActivation(workspaceContains: ["package.json", "Cargo.toml"], remoteHost: ["github.com"],
                                    agent: ["claude", "shell"], platform: ["darwin"])
        let clauses = ExtensionCopy.describeActivation(a)
        #expect(clauses[0] == "projects with package.json or Cargo.toml")
        #expect(clauses[1] == "GitHub remotes")
        #expect(clauses[2].hasSuffix("or shell sessions"))
        #expect(clauses[3] == "macOS")
    }

    @Test func contributionsAndPanesAndFooters() {
        let pane = ExtensionContribution(id: "p", title: "P", web: "index.html")
        let c = ExtensionContributions(panes: [pane, pane], footers: [ExtensionContribution(id: "f", title: "F")])
        #expect(ExtensionCopy.describeContributions(c) == "2 panes, 1 footer")
        #expect(ExtensionCopy.describeContributions(nil) == "")
        #expect(ExtensionCopy.describePaneKind(pane) == "a page it ships")
        #expect(ExtensionCopy.describePaneKind(ExtensionContribution(id: "c", title: "C", command: ["npx", "x"])) == "runs npx x")
        #expect(ExtensionCopy.describePaneKind(ExtensionContribution(id: "n", title: "N")) == "a pane")
        #expect(ExtensionCopy.describeFooterInterval(30) == "every 30s")
        #expect(ExtensionCopy.describeFooterInterval(60) == "every minute")
        #expect(ExtensionCopy.describeFooterInterval(300) == "every 5 minutes")
        #expect(ExtensionCopy.describeFooterInterval(90) == "every 90s")
    }

    @Test func cronTextCoversSeededShapesOnly() {
        #expect(ExtensionCopy.humanCron("* * * * *") == "every minute")
        #expect(ExtensionCopy.humanCron("*/1 * * * *") == "every 1 minute")
        #expect(ExtensionCopy.humanCron("*/15 * * * *") == "every 15 minutes")
        #expect(ExtensionCopy.humanCron("0 9 * * 1") == "0 9 * * 1")
        #expect(ExtensionCopy.humanCron("*/x * * * *") == "*/x * * * *")
    }

    @Test func versionOrder() {
        #expect(PackState.isNewerVersion("1.2.0", than: "1.1.9"))
        #expect(PackState.isNewerVersion("1.10.0", than: "1.9.0"))
        #expect(!PackState.isNewerVersion("1.0.0", than: "1.0.0"))
        #expect(!PackState.isNewerVersion("1.0.0-beta", than: "1.0.0"))
        #expect(PackState.isNewerVersion("1.0.0", than: "1.0.0-beta"))
        #expect(!PackState.isNewerVersion("0.9.0", than: "1.0.0"))
    }

    @Test func packStatesAndTheirStatus() {
        let item = CatalogItem(id: "acme", name: "Acme", version: "1.1.0", packUrl: "https://example.com/acme.tgz")
        let unreleased = CatalogItem(id: "acme", name: "Acme", version: "1.1.0")
        let pack = InstalledPack(id: "acme", name: "Acme", version: "1.0.0", previousVersion: "0.9.0")

        #expect(PackState(installed: nil, catalogItem: item, progress: nil) == .absent)
        #expect(PackState(installed: nil, catalogItem: unreleased, progress: nil) == .notReleased)
        #expect(PackState(installed: pack, catalogItem: item, progress: nil)
            == .installed(version: "1.0.0", availableVersion: "1.1.0", previousVersion: "0.9.0"))
        #expect(PackState(installed: pack, catalogItem: unreleased, progress: nil)
            == .installed(version: "1.0.0", availableVersion: nil, previousVersion: "0.9.0"))
        let downloading = PackState(installed: nil, catalogItem: item, progress: InstallProgress(id: "acme", phase: .downloading, percent: 40))
        #expect(downloading == .installing(phase: .downloading, percent: 40))
        let failed = PackState(installed: nil, catalogItem: item, progress: InstallProgress(id: "acme", phase: .failed, error: "Bad hash"))
        #expect(failed == .rejected(error: "Bad hash"))

        #expect(PackStatusView(.absent).label == "Install")
        #expect(PackStatusView(.absent).action == .install)
        let update = PackStatusView(.installed(version: "1.0.0", availableVersion: "1.1.0", previousVersion: nil))
        #expect(update.label == "Update")
        #expect(update.detail == "v1.0.0 → 1.1.0 available")
        #expect(update.tone == .blocked)
        let installed = PackStatusView(.installed(version: "1.0.0", availableVersion: nil, previousVersion: nil))
        #expect(installed.label == "Installed")
        #expect(installed.tone == .settled)
        let progress = PackStatusView(downloading)
        #expect(progress.label == "Downloading")
        #expect(progress.percent == 40)
        #expect(progress.busy)
        #expect(PackStatusView(.installing(phase: .verifying, percent: 80)).percent == nil)
        let rejected = PackStatusView(failed)
        #expect(rejected.label == "Couldn't install")
        #expect(rejected.action == .retry)
        #expect(rejected.tone == .broken)
    }

    @Test func whoCanTakeAConnection() {
        #expect(PackStatusView.canAddConnection(.absent, source: .builtin, hasLegacyLaunch: false, kind: .connector))
        #expect(PackStatusView.canAddConnection(.absent, source: .mcp, hasLegacyLaunch: false, kind: .connector))
        #expect(!PackStatusView.canAddConnection(.absent, source: .catalog, hasLegacyLaunch: false, kind: .connector))
        #expect(PackStatusView.canAddConnection(.absent, source: .catalog, hasLegacyLaunch: true, kind: .connector))
        #expect(!PackStatusView.canAddConnection(.notReleased, source: .catalog, hasLegacyLaunch: true, kind: .connector))
        let installed = PackState.installed(version: "1.0.0", availableVersion: nil, previousVersion: nil)
        #expect(PackStatusView.canAddConnection(installed, source: .installed, hasLegacyLaunch: false, kind: .connector))
        #expect(!PackStatusView.canAddConnection(installed, source: .installed, hasLegacyLaunch: false, kind: .extension))
    }

    @Test func catalogAgeReadsInWholeUnits() {
        let now = 10_000_000_000.0
        #expect(ConnectorBrowse.catalogAge(fetchedAt: nil, now: now) == "Showing the connectors that shipped with Vorn")
        #expect(ConnectorBrowse.catalogAge(fetchedAt: now - 30_000, now: now) == "Updated just now")
        #expect(ConnectorBrowse.catalogAge(fetchedAt: now - 60_000, now: now) == "Updated 1 minute ago")
        #expect(ConnectorBrowse.catalogAge(fetchedAt: now - 5 * 3_600_000, now: now) == "Updated 5 hours ago")
        #expect(ConnectorBrowse.catalogAge(fetchedAt: now - 49 * 3_600_000, now: now) == "Updated 2 days ago")
    }

    @Test func browseListsBuiltInsCatalogAndInstalledOnce() {
        let builtIns = [BuiltInConnector(id: "github", name: "GitHub"), BuiltInConnector(id: "http", name: "HTTP")]
        let catalog = [CatalogItem(id: "acme", name: "Acme Tickets", description: "Tickets", version: "1.0.0", packUrl: "https://x/acme.tgz"),
                       CatalogItem(id: "timer", name: "Focus Timer", kind: .extension, version: "0.3.1", packUrl: "https://x/t.tgz")]
        let packs = [InstalledPack(id: "acme", name: "Acme Tickets", version: "1.0.0")]
        let listings = ConnectorBrowse.listings(builtIns: builtIns, catalog: catalog, connections: [], packs: packs, mcpServers: [])
        #expect(Set(listings.map(\.id)).count == listings.count)
        #expect(listings.contains { $0.id == "acme" && $0.pack != nil })
        #expect(ConnectorBrowse.filter(listings, kind: .extension).map(\.id) == ["timer"])
        #expect(ConnectorBrowse.filter(listings, query: "tick").map(\.id) == ["acme"])
        #expect(ConnectorBrowse.filter(listings, query: "").count == listings.count)
        let conns = [SourceConnection(id: "c1", connectorId: "github", name: "Repo A"),
                     SourceConnection(id: "c2", connectorId: "github", name: "Repo B"),
                     SourceConnection(id: "c3", connectorId: "http", name: "Status")]
        let groups = ConnectorBrowse.groups(conns, listings: listings)
        #expect(groups.map(\.name) == ["GitHub", "HTTP"])
        #expect(groups.first?.connections.count == 2)
    }
}
