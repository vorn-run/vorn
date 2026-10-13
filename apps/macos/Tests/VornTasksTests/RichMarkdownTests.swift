import Testing
import VornUI

@Suite struct RichMarkdownTests {
    private let seeded = """
    ## Goal

    Port the **task board** to the *native* app, with `TasksView` mounted from the [top bar](https://vorn.run).

    - List and kanban

    - Drag to reorder

    1. Mount the view

    2. Wire the sidebar

    - [x] Status picker

    - [ ] Image attachments

    > Keep the copy as it is today.

    ```swift
    let store = TasksStore(service: client)
    ```

    ---

    Done when it ~~looks~~ matches.
    """

    /// What today's editor saved for `seeded` after one edit: every list item its own list, so both numbers read "1.".
    private let savedToday = "## Goal\n\nPort the **task board** to the *native* app, with `TasksView` mounted from the [top bar](https://vorn.run).\n\n- List and kanban\n\n- Drag to reorder\n\n1. Mount the view\n\n1. Wire the sidebar\n\n- [x] Status picker\n\n- [ ] Image attachments\n\n> Keep the copy as it is today.\n\n```swift\nlet store = TasksStore(service: client)\n```\n\n---\n\nDone when it ~~looks~~ matches.\n"

    @Test func roundTripMatchesTodaysEditor() {
        #expect(RichMarkdown.serialize(RichMarkdown.parse(seeded)) == savedToday)
    }

    @Test func blocksAndMarks() {
        let p = RichMarkdown.parse(seeded)
        #expect(p.first?.block == .heading(2))
        #expect(p[1].runs.map(\.marks) == [[], .bold, [], .italic, [], .code, [], [], []])
        #expect(p[1].runs.first { $0.link != nil }?.link == "https://vorn.run")
        #expect(p.contains { $0.block == .task(checked: true) })
        #expect(p.contains { $0.block == .code(language: "swift") })
        #expect(p.contains { $0.block == .rule })
    }

    @Test func consecutiveItemsShareAList() {
        let p = RichMarkdown.parse("1. one\n2. two\n3. three")
        #expect(Set(p.map(\.group)).count == 1)
        #expect(RichMarkdown.serialize(p) == "1. one\n\n2. two\n\n3. three\n")
    }

    @Test func quoteLinesBecomeOneParagraphWithBreaks() {
        let p = RichMarkdown.parse("> one\n> two")
        #expect(p.count == 1)
        #expect(p[0].text == "one\u{2028}two")
        #expect(RichMarkdown.serialize(p) == "> one\n> two\n")
    }

    @Test func emptyDocument() {
        #expect(RichMarkdown.parse("  \n").isEmpty)
        #expect(RichMarkdown.serialize([]) == "\n")
    }

    @Test func unsafeLinksKeepOnlyTheirLabel() {
        let runs = RichMarkdown.inline("[x](javascript:alert(1)) and [y](/docs)")
        #expect(runs.map(\.text).joined() == "x) and y")
        #expect(runs.last?.link == "/docs")
    }

    @Test func codeExcludesOtherMarks() {
        #expect(RichMarkdown.serialize([RichParagraph(.paragraph, runs: [RichRun("a", marks: [.bold, .italic]), RichRun("b", marks: .code)])])
            == "***a***`b`\n")
    }

    @Test func slashFilter() {
        #expect(RichSlashCommand.filter("").count == 9)
        #expect(RichSlashCommand.filter("list").map(\.title) == ["Bullet List", "Numbered List", "Task List"])
        #expect(RichSlashCommand.filter("separator").map(\.title) == ["Divider"])
    }
}
