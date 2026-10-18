#!/usr/bin/env python3
"""Упаковывает .gltf с буферами `data:` в .glb: движок `data:`-буферы не грузит.

Запуск: python3 -I gltf_to_glb.py IN.gltf OUT.glb
"""

import base64
import json
import struct
import sys


def pad(data: bytes, fill: bytes) -> bytes:
    return data + fill * (-len(data) % 4)


def convert(src: str, dst: str) -> None:
    with open(src, encoding="utf-8") as f:
        doc = json.load(f)
    blob = b""
    starts = []
    for buf in doc["buffers"]:
        head, _, body = buf["uri"].partition(",")
        if not head.startswith("data:") or not head.endswith(";base64"):
            sys.exit(f"{src}: буфер не data:base64")
        starts.append(len(blob))
        blob = pad(blob + base64.b64decode(body), b"\0")
    for view in doc["bufferViews"]:
        view["byteOffset"] = starts[view.get("buffer", 0)] + view.get("byteOffset", 0)
        view["buffer"] = 0
    doc["buffers"] = [{"byteLength": len(blob)}]
    text = pad(json.dumps(doc, separators=(",", ":")).encode(), b" ")
    chunks = struct.pack("<II", len(text), 0x4E4F534A) + text
    chunks += struct.pack("<II", len(blob), 0x004E4942) + blob
    with open(dst, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, 12 + len(chunks)) + chunks)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    convert(sys.argv[1], sys.argv[2])
