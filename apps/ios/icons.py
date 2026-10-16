"""Writes Vorn/Icons.xcassets from vornui's icon set and the agent marks, the same SVGs the desktop draws."""
import json
import os
import shutil
import sys

here = os.path.dirname(os.path.abspath(__file__))
crates = os.path.join(here, "..", "..", "packages", "core", "crates")
out = os.path.join(here, "Vorn", "Icons.xcassets")


def imageset(name, svg, template):
    d = os.path.join(out, name + ".imageset")
    os.makedirs(d)
    with open(os.path.join(d, "icon.svg"), "w") as f:
        f.write(svg)
    contents = {
        "images": [{"filename": "icon.svg", "idiom": "universal"}],
        "info": {"author": "xcode", "version": 1},
        "properties": {
            "preserves-vector-representation": True,
            "template-rendering-intent": "template" if template else "original",
        },
    }
    with open(os.path.join(d, "Contents.json"), "w") as f:
        json.dump(contents, f, indent=2)


def main():
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    with open(os.path.join(out, "Contents.json"), "w") as f:
        json.dump({"info": {"author": "xcode", "version": 1}}, f)
    icons = os.path.join(crates, "vornui", "icons")
    count = 0
    for file in sorted(os.listdir(icons)):
        if file.endswith(".svg"):
            # A template image is tinted by its alpha, so the stroke's colour only has to be opaque.
            svg = open(os.path.join(icons, file)).read().replace("currentColor", "#000000")
            imageset(file[:-4], svg, True)
            count += 1
    marks = os.path.join(crates, "vorn-components", "assets", "agents")
    for file in sorted(os.listdir(marks)):
        if file.endswith(".svg"):
            imageset("agent-" + file[:-4], open(os.path.join(marks, file)).read(), False)
            count += 1
    print(f"{count} icons", file=sys.stderr)


main()
