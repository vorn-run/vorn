#!/usr/bin/env python3
"""Writes Sources/VornUI/LucideGlyphs.swift from the lucide-static SVGs the renderer's icons come from."""

import concurrent.futures
import pathlib
import re
import urllib.request

VERSION = "1.34.0"  # lucide-react in package.json
ICONS = {
    # Swift name: lucide file
    "circle": "circle", "clock": "clock", "eye": "eye", "circleCheck": "circle-check",
    "circleX": "circle-x", "plus": "plus", "ellipsis": "ellipsis", "pencil": "pencil",
    "trash2": "trash-2", "archive": "archive", "archiveRestore": "archive-restore",
    "rotateCcw": "rotate-ccw", "fileCode": "file-code", "play": "play", "terminal": "terminal",
    "slidersHorizontal": "sliders-horizontal", "listTodo": "list-todo", "chevronDown": "chevron-down",
    "chevronRight": "chevron-right", "x": "x", "maximize2": "maximize-2", "paperclip": "paperclip",
    "folderGit2": "folder-git-2", "gitBranch": "git-branch", "calendar": "calendar",
    "imagePlus": "image-plus", "save": "save", "check": "check", "folder": "folder", "bot": "bot",
    "zap": "zap", "clipboardList": "clipboard-list", "code": "code", "globe": "globe",
    "database": "database", "server": "server", "smartphone": "smartphone", "package": "package",
    "cpu": "cpu", "cloud": "cloud", "shield": "shield", "gamepad2": "gamepad-2", "music": "music",
    "image": "image", "bookOpen": "book-open", "flaskConical": "flask-conical", "rocket": "rocket",
    "panelLeft": "panel-left",
    "monitor": "monitor",
    "triangleAlert": "triangle-alert",
    "workflow": "workflow",
    "chevronsRight": "chevrons-right",
    "layers": "layers",
    "folderPlus": "folder-plus",
    "listFilter": "list-filter",
    "circleHelp": "circle-help",
    "settings": "settings",
    "folderTree": "folder-tree",
    "folderOpen": "folder-open",
    "squareTerminal": "square-terminal",
    "minus": "minus",
    "minimize2": "minimize-2",
    "arrowUp": "arrow-up",
    "lightbulb": "lightbulb",
    "user": "user",
    "briefcase": "briefcase",
    "house": "house",
    "building2": "building-2",
    "heart": "heart",
    "star": "star",
    "sparkles": "sparkles",
    "chevronLeft": "chevron-left",
    "gripVertical": "grip-vertical",
    "arrowDown": "arrow-down",
    "keyboard": "keyboard",
    "refreshCw": "refresh-cw",
    "loaderCircle": "loader-circle",
    "gitCommitHorizontal": "git-commit-horizontal",
    "send": "send",
    "messageSquare": "message-square",
    "heading1": "heading-1",
    "heading2": "heading-2",
    "heading3": "heading-3",
    "list": "list",
    "listOrdered": "list-ordered",
    "squareCheckBig": "square-check-big",
    "quote": "quote",
    "bold": "bold",
    "italic": "italic",
    "strikethrough": "strikethrough",
    "link": "link",
    "unlink": "unlink",
}
# Inline glyphs from TaskToolbar.tsx and OptionRow.tsx rather than lucide.
INLINE = {
    "viewList": '<line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" />'
    '<line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" />'
    '<line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />',
    "viewKanban": '<rect x="3" y="3" width="18" height="18" rx="2" /><path d="M9 3v18" /><path d="M15 3v18" />',
    "optionCheck": '<polyline points="20 6 9 17 4 12" />',
    "layoutGrid": '<rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />'
    '<rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" />',
    "layoutTabs": '<rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18" /><path d="M9 3v6" />',
}

# Lucide component names a project or workspace stores in config, as the sidebar resolves them.
COMPONENTS = {
    "Folder": "folder",
    "FolderGit2": "folderGit2",
    "Code": "code",
    "Globe": "globe",
    "Database": "database",
    "Server": "server",
    "Smartphone": "smartphone",
    "Package": "package",
    "FileCode": "fileCode",
    "Terminal": "terminal",
    "Cpu": "cpu",
    "Cloud": "cloud",
    "Shield": "shield",
    "Zap": "zap",
    "Workflow": "workflow",
    "Gamepad2": "gamepad2",
    "Music": "music",
    "Image": "image",
    "BookOpen": "bookOpen",
    "FlaskConical": "flaskConical",
    "Rocket": "rocket",
    "Play": "play",
    "User": "user",
    "Briefcase": "briefcase",
    "Home": "house",
    "House": "house",
    "Building2": "building2",
    "Heart": "heart",
    "Star": "star",
    "Sparkles": "sparkles",
    "Layers": "layers",
}

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "Sources/VornUI/LucideGlyphs.swift"


def fetch(file: str) -> str:
    url = f"https://unpkg.com/lucide-static@{VERSION}/icons/{file}.svg"
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=30) as r:
                return r.read().decode()
        except OSError:
            if attempt == 3:
                raise


def attrs(tag: str) -> dict:
    return dict(re.findall(r'([\w-]+)="([^"]*)"', tag))


def elements(svg: str) -> list[str]:
    out = []
    for tag in re.findall(r"<(path|circle|rect|line|polyline|polygon|ellipse)\b([^>]*)/?>", svg):
        kind, rest = tag
        a = attrs(rest)
        n = lambda k, d="0": a.get(k, d)
        if kind == "path":
            out.append(f'.path("{a["d"]}")')
        elif kind == "circle":
            out.append(f".circle(cx: {n('cx')}, cy: {n('cy')}, r: {n('r')})")
        elif kind == "ellipse":
            out.append(f".ellipse(cx: {n('cx')}, cy: {n('cy')}, rx: {n('rx')}, ry: {n('ry')})")
        elif kind == "rect":
            rx = a.get("rx", a.get("ry", "0"))
            out.append(f".rect(x: {n('x')}, y: {n('y')}, width: {n('width')}, height: {n('height')}, rx: {rx})")
        elif kind == "line":
            out.append(f".line(x1: {n('x1')}, y1: {n('y1')}, x2: {n('x2')}, y2: {n('y2')})")
        elif kind in ("polyline", "polygon"):
            pts = [float(v) for v in re.split(r"[\s,]+", a["points"].strip())]
            pairs = ", ".join(f"({pts[i]:g}, {pts[i + 1]:g})" for i in range(0, len(pts), 2))
            out.append(f".poly([{pairs}], closed: {'true' if kind == 'polygon' else 'false'})")
    return out


def main() -> None:
    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        svgs = dict(zip(ICONS, pool.map(fetch, ICONS.values())))
    svgs.update(INLINE)
    lines = [
        "// Generated by scripts/gen-lucide.py from lucide-static " + VERSION + " (ISC). Do not edit.",
        "",
        "public extension LucideGlyph {",
    ]
    for name in sorted(svgs):
        body = ", ".join(elements(svgs[name]))
        lines.append(f"    static let {name} = LucideGlyph([{body}])")
    lines.append("")
    lines.append("    /// The icon a project or workspace names (a lucide component name, as stored in config).")
    lines.append("    init?(componentName: String) {")
    lines.append("        switch componentName {")
    for component, name in COMPONENTS.items():
        lines.append(f'        case "{component}": self = .{name}')
    lines.append("        default: return nil")
    lines.append("        }")
    lines.append("    }")
    lines.append("}")
    OUT.write_text("\n".join(lines) + "\n")
    print(f"wrote {OUT} ({len(svgs)} glyphs)")


if __name__ == "__main__":
    main()
