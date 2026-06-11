#!/usr/bin/env python3
import base64
import json
import os
import re
import struct
import sys
from datetime import datetime, timezone
from pathlib import Path


BASE_DIR = Path("/tmp/mindemon-dumps")
MAX_FILE_BYTES = 64 * 1024 * 1024
MAX_TOTAL_BYTES = 512 * 1024 * 1024


def read_message():
    raw_len = sys.stdin.buffer.read(4)
    if len(raw_len) == 0:
        return None
    if len(raw_len) != 4:
        raise RuntimeError("Invalid native message length prefix")
    msg_len = struct.unpack("<I", raw_len)[0]
    payload = sys.stdin.buffer.read(msg_len)
    if len(payload) != msg_len:
        raise RuntimeError("Truncated native message")
    return json.loads(payload.decode("utf-8"))


def write_message(message):
    payload = json.dumps(message, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(struct.pack("<I", len(payload)))
    sys.stdout.buffer.write(payload)
    sys.stdout.buffer.flush()


def safe_segment(segment):
    segment = str(segment or "unnamed")
    segment = re.sub(r'[<>:"\\|?*\x00-\x1f]', "_", segment)
    segment = segment.strip().strip(".")
    if not segment:
        segment = "unnamed"
    return segment[:180]


def safe_relative_path(path):
    raw_parts = Path(str(path or "unnamed")).parts
    parts = []
    for part in raw_parts:
        if part in ("", ".", "/", "\\"):
            continue
        if part == "..":
            parts.append("_")
            continue
        parts.append(safe_segment(part))
    if not parts:
        parts = ["unnamed"]
    return Path(*parts)


def decode_content(file_entry):
    content = file_entry.get("content", "")
    encoding = file_entry.get("encoding", "utf-8")
    if encoding == "base64":
      return base64.b64decode(content)
    if isinstance(content, str):
        return content.encode("utf-8")
    raise RuntimeError("Unsupported file content type")


def unique_output_dir():
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    base = BASE_DIR / stamp
    candidate = base
    index = 2
    while candidate.exists():
        candidate = BASE_DIR / f"{stamp}-{index}"
        index += 1
    return candidate


def dump_files(message):
    files = message.get("files")
    if not isinstance(files, list):
        raise RuntimeError("Expected files array")

    output_dir = unique_output_dir()
    output_dir.mkdir(parents=True, exist_ok=False)

    total = 0
    written = 0
    for file_entry in files:
        if not isinstance(file_entry, dict):
            raise RuntimeError("Invalid file entry")
        rel = safe_relative_path(file_entry.get("path"))
        target = output_dir / rel
        resolved = target.resolve()
        if output_dir.resolve() not in resolved.parents and resolved != output_dir.resolve():
            raise RuntimeError("Refusing to write outside output directory")

        data = decode_content(file_entry)
        if len(data) > MAX_FILE_BYTES:
            raise RuntimeError(f"Refusing oversized file: {rel}")
        total += len(data)
        if total > MAX_TOTAL_BYTES:
            raise RuntimeError("Refusing dump larger than limit")

        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        written += 1

    metadata = {
        "tool": "mindemon_dump_host",
        "target": message.get("target"),
        "outputDir": str(output_dir),
        "fileCount": written,
        "totalBytes": total,
        "writtenAt": datetime.now(timezone.utc).isoformat(),
    }
    (output_dir / "native-host-result.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    return {
        "ok": True,
        "outputDir": str(output_dir),
        "fileCount": written + 1,
        "totalBytes": total,
    }


class DumpSession:
    def __init__(self):
        self.output_dir = None
        self.target = None
        self.current_file = None
        self.current_path = None
        self.current_bytes = 0
        self.total_bytes = 0
        self.file_count = 0

    def start(self, message):
        if self.output_dir is not None:
            raise RuntimeError("Dump session already started")
        self.target = message.get("target")
        self.output_dir = unique_output_dir()
        self.output_dir.mkdir(parents=True, exist_ok=False)
        return {"ok": True, "outputDir": str(self.output_dir)}

    def start_file(self, message):
        self._require_started()
        if self.current_file is not None:
            raise RuntimeError("Previous file was not finished")
        rel = safe_relative_path(message.get("path"))
        target = self.output_dir / rel
        resolved = target.resolve()
        if self.output_dir.resolve() not in resolved.parents and resolved != self.output_dir.resolve():
            raise RuntimeError("Refusing to write outside output directory")
        target.parent.mkdir(parents=True, exist_ok=True)
        self.current_path = rel
        self.current_bytes = 0
        self.current_file = target.open("wb")
        return {"ok": True}

    def write_chunk(self, message):
        self._require_started()
        if self.current_file is None:
            raise RuntimeError("No open file")
        data = base64.b64decode(message.get("data", ""))
        self.current_bytes += len(data)
        self.total_bytes += len(data)
        if self.current_bytes > MAX_FILE_BYTES:
            raise RuntimeError(f"Refusing oversized file: {self.current_path}")
        if self.total_bytes > MAX_TOTAL_BYTES:
            raise RuntimeError("Refusing dump larger than limit")
        self.current_file.write(data)
        return {"ok": True}

    def finish_file(self):
        self._require_started()
        if self.current_file is None:
            raise RuntimeError("No open file")
        self.current_file.close()
        self.current_file = None
        self.current_path = None
        self.current_bytes = 0
        self.file_count += 1
        return {"ok": True}

    def finish(self):
        self._require_started()
        if self.current_file is not None:
            self.finish_file()
        metadata = {
            "tool": "mindemon_dump_host",
            "target": self.target,
            "outputDir": str(self.output_dir),
            "fileCount": self.file_count,
            "totalBytes": self.total_bytes,
            "writtenAt": datetime.now(timezone.utc).isoformat(),
        }
        (self.output_dir / "native-host-result.json").write_text(
            json.dumps(metadata, indent=2),
            encoding="utf-8",
        )
        self.file_count += 1
        return {
            "ok": True,
            "outputDir": str(self.output_dir),
            "fileCount": self.file_count,
            "totalBytes": self.total_bytes,
        }

    def close(self):
        if self.current_file is not None:
            try:
                self.current_file.close()
            finally:
                self.current_file = None

    def _require_started(self):
        if self.output_dir is None:
            raise RuntimeError("Dump session was not started")


def handle_message(session, message):
    action = message.get("action")
    if action == "dump":
        return dump_files(message)
    if action == "start":
        return session.start(message)
    if action == "startFile":
        return session.start_file(message)
    if action == "writeChunk":
        return session.write_chunk(message)
    if action == "finishFile":
        return session.finish_file()
    if action == "finish":
        return session.finish()
    raise RuntimeError("Unsupported action")


def main():
    session = DumpSession()
    try:
        while True:
            message = read_message()
            if message is None:
                return
            request_id = message.get("id")
            try:
                response = handle_message(session, message)
                if request_id is not None:
                    response["id"] = request_id
                write_message(response)
            except Exception as error:
                response = {"ok": False, "error": str(error)}
                if request_id is not None:
                    response["id"] = request_id
                write_message(response)
    finally:
        session.close()


if __name__ == "__main__":
    main()
