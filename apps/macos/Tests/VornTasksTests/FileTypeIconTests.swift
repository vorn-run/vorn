import Testing
import VornUI

@Suite struct FileTypeIconTests {
    @Test func resolvesAsTheRendererDoes() {
        #expect(FileTypeIcon.resolve("README.md") == (0xE04D, 0x519ABA))
        #expect(FileTypeIcon.resolve("board.ts") == FileTypeIcon.resolve("other.ts"))
        #expect(FileTypeIcon.resolve(".envrc") == FileTypeIcon.resolve(".zshrc"))
        #expect(FileTypeIcon.resolve("no-extension").ch == 0xE023)
    }
}
