#!/usr/bin/env python3
import argparse
import json
import socket
import struct
import sys


MAGIC = b"GS"
MSG_LIST_TABS = 0x02
MSG_TAB_LIST = 0x82


def encode_message(message_type: int, payload: bytes) -> bytes:
    return MAGIC + bytes([message_type]) + struct.pack("<I", len(payload)) + payload


def read_exact(sock: socket.socket, size: int) -> bytes:
    chunks = []
    remaining = size
    while remaining > 0:
        chunk = sock.recv(remaining)
        if not chunk:
            raise RuntimeError("unexpected EOF")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def read_message(sock: socket.socket) -> tuple[int, bytes]:
    header = read_exact(sock, 7)
    if header[:2] != MAGIC:
        raise RuntimeError("invalid stream magic")
    message_type = header[2]
    payload_len = struct.unpack("<I", header[3:7])[0]
    payload = read_exact(sock, payload_len) if payload_len else b""
    return message_type, payload


def decode_string(payload: bytes, offset: int) -> tuple[str, int]:
    if offset + 2 > len(payload):
        raise RuntimeError("truncated string length")
    length = struct.unpack("<H", payload[offset : offset + 2])[0]
    offset += 2
    if offset + length > len(payload):
        raise RuntimeError("truncated string payload")
    value = payload[offset : offset + length].decode("utf-8")
    return value, offset + length


def decode_tab_list(payload: bytes) -> list[dict[str, object]]:
    if len(payload) < 4:
        raise RuntimeError("truncated tab list")
    count = struct.unpack("<I", payload[:4])[0]
    offset = 4
    tabs: list[dict[str, object]] = []
    for _ in range(count):
        if offset + 4 > len(payload):
            raise RuntimeError("truncated tab id")
        tab_id = struct.unpack("<I", payload[offset : offset + 4])[0]
        offset += 4
        name, offset = decode_string(payload, offset)
        title, offset = decode_string(payload, offset)
        pwd, offset = decode_string(payload, offset)
        if offset >= len(payload):
            raise RuntimeError("truncated tab flags")
        flags = payload[offset]
        offset += 1
        tabs.append(
            {
                "id": tab_id,
                "name": name,
                "title": title,
                "pwd": pwd,
                "active": bool(flags & 0x01),
                "child_exited": bool(flags & 0x02),
            }
        )
    return tabs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True)
    args = parser.parse_args()

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.connect(f"{args.socket}.stream")
        sock.sendall(encode_message(MSG_LIST_TABS, b""))
        ignored: list[str] = []
        for _ in range(8):
            message_type, payload = read_message(sock)
            if message_type == MSG_TAB_LIST:
                print(
                    json.dumps(
                        {
                            "ok": True,
                            "ignored_message_types": ignored,
                            "tabs": decode_tab_list(payload),
                        }
                    )
                )
                return 0
            ignored.append(f"0x{message_type:02x}")

    print(
        json.dumps(
            {
                "ok": False,
                "error": "did not receive tab list",
                "ignored_message_types": ignored,
            }
        )
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
