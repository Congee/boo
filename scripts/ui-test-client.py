#!/usr/bin/env python3
import argparse
import re
import json
import socket
import sys
import time


def request(socket_path: str, payload: dict) -> dict:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.connect(socket_path)
        sock.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            chunk = sock.recv(65536)
            if not chunk:
                break
            data += chunk
    return json.loads(data.decode("utf-8"))


def decode_value(value: str):
    if value.isdigit():
        return int(value)
    if value.lower() in {"true", "false"}:
        return value.lower() == "true"
    def replace(match: re.Match[str]) -> str:
        token = match.group(0)
        if token == "\\n":
            return "\n"
        if token == "\\r":
            return "\r"
        if token == "\\t":
            return "\t"
        if token == "\\\\":
            return "\\"
        if token.startswith("\\x"):
            return chr(int(token[2:], 16))
        if token.startswith("\\u"):
            return chr(int(token[2:], 16))
        return token

    return re.sub(r"\\\\|\\n|\\r|\\t|\\x[0-9a-fA-F]{2}|\\u[0-9a-fA-F]{4}", replace, value)


def main() -> int:
    parser = argparse.ArgumentParser(description="Boo UI test control client")
    parser.add_argument("--socket", required=True, help="Path to Boo control socket")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("snapshot", help="Fetch current UI snapshot")

    req = sub.add_parser("request", help="Send an arbitrary control request")
    req.add_argument("name", help="Control request cmd name in kebab-case")
    req.add_argument("pairs", nargs="*", help="Optional key=value payload entries")

    wait = sub.add_parser("wait-ready", help="Wait until Boo reports a populated UI snapshot")
    wait.add_argument("--timeout", type=float, default=12.0, help="Timeout in seconds")

    args = parser.parse_args()

    if args.command == "snapshot":
        print(json.dumps(request(args.socket, {"cmd": "get-ui-snapshot"})))
        return 0

    if args.command == "request":
        payload = {"cmd": args.name}
        for pair in args.pairs:
            key, value = pair.split("=", 1)
            payload[key] = decode_value(value)
        print(json.dumps(request(args.socket, payload)))
        return 0

    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        try:
            snapshot = request(args.socket, {"cmd": "get-ui-snapshot"})
            if snapshot.get("snapshot", {}).get("tabs"):
                print(json.dumps(snapshot))
                return 0
        except (FileNotFoundError, ConnectionRefusedError, json.JSONDecodeError):
            pass
        time.sleep(0.1)

    print("timed out waiting for populated ui snapshot", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
